use super::{
    CloseErr, CloseErrKind, CloseMsg, CloseTransport, GraceType, MsgBody, PushMsg, TracingMsg,
};
use std::{
    fmt::Debug,
    future::Future,
    io,
    pin::Pin,
    task::{self, Poll},
};
use thiserror::Error;
use tokio::{
    signal::ctrl_c,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    task::{JoinError, JoinHandle},
};
use tokio_util::sync::CancellationToken;
use tracing_core::{
    span::{self, Attributes, Record},
    Event, Subscriber,
};
use tracing_subscriber::{filter::Filtered, layer::Context, Layer};

pub use tracing_subscriber::filter::LevelFilter;

#[derive(Clone, Debug)]
pub struct MsgLayer(UnboundedSender<TracingMsg>);

impl<S: Subscriber> Layer<S> for MsgLayer {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &span::Id, _ctx: Context<'_, S>) {
        self.0.send(MsgBody::on_new_span(attrs, id).into()).ok();
    }

    fn on_record(&self, span: &span::Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        self.0.send(MsgBody::on_record(span, values).into()).ok();
    }

    fn on_follows_from(&self, span: &span::Id, follows: &span::Id, _ctx: Context<'_, S>) {
        self.0
            .send(MsgBody::on_follows_from(span, follows).into())
            .ok();
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        self.0.send(MsgBody::on_event(event).into()).ok();
    }

    fn on_enter(&self, id: &span::Id, _ctx: Context<'_, S>) {
        self.0.send(MsgBody::on_enter(id).into()).ok();
    }

    fn on_exit(&self, id: &span::Id, _ctx: Context<'_, S>) {
        self.0.send(MsgBody::on_exit(id).into()).ok();
    }

    fn on_close(&self, id: span::Id, _ctx: Context<'_, S>) {
        self.0.send(MsgBody::on_close(id).into()).ok();
    }

    fn on_id_change(&self, old: &span::Id, new: &span::Id, _ctx: Context<'_, S>) {
        self.0.send(MsgBody::on_id_change(old, new).into()).ok();
    }
}

#[derive(Error, Debug)]
pub enum LayerError<T: PushMsg> {
    #[error("io error: `{0}`")]
    Io(#[from] io::Error),
    #[error("MsgLayer dropped")]
    LayerDropped,
    #[error("push_msg error: `{0}`")]
    PushMsgErr(T::Error),
    #[error("bulk_push error: `{0}`")]
    BulkPushErr(T::Error),
    #[error("msg buffer full")]
    BufferFull,
}

impl<T: PushMsg> From<&LayerError<T>> for CloseErr {
    fn from(err: &LayerError<T>) -> Self {
        let kind = match err {
            LayerError::Io(_) => CloseErrKind::Io,
            LayerError::LayerDropped => CloseErrKind::LayerDropped,
            LayerError::PushMsgErr(_) => CloseErrKind::PushMsgErr,
            LayerError::BulkPushErr(_) => CloseErrKind::BulkPushErr,
            LayerError::BufferFull => CloseErrKind::BufferFull,
        };

        Self::new(kind, err)
    }
}

type RoutineOutput<T> = Result<GraceType, LayerError<T>>;
type HandleOutput<T> = Result<RoutineOutput<T>, JoinError>;

impl<T: PushMsg> From<&RoutineOutput<T>> for CloseMsg {
    fn from(res: &RoutineOutput<T>) -> Self {
        match res {
            Ok(ok) => Self::ok((*ok).into()),
            Err(err) => Self::err(err.into()),
        }
    }
}

#[derive(Debug)]
pub struct MsgRoutine<T: PushMsg> {
    shutdown_trigger: CancellationToken,
    routine: JoinHandle<RoutineOutput<T>>,
}

impl<T: PushMsg> MsgRoutine<T> {
    pub fn trigger_graceful_shutdown(&self) {
        self.shutdown_trigger.cancel();
    }

    pub async fn graceful_shutdown(self) -> HandleOutput<T> {
        self.trigger_graceful_shutdown();
        self.await
    }
}

impl<T: PushMsg> Future for MsgRoutine<T> {
    type Output = HandleOutput<T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.routine).poll(cx)
    }
}

pub type FilteredLayer<S> = Filtered<MsgLayer, LevelFilter, S>;

#[derive(Clone, Debug)]
pub struct MsgLayerBuiler<T: CloseTransport + PushMsg + Clone + Debug> {
    transport: T,
    level_filter: LevelFilter,
    ctrlc_shutdown: bool,
    close_on_shutdown: bool,
    abort_on_error: bool,
}

impl<T: CloseTransport + PushMsg + Clone + Debug> MsgLayerBuiler<T> {
    pub fn set_level_filter(self, level_filter: LevelFilter) -> Self {
        Self {
            level_filter,
            ..self
        }
    }

    pub fn disable_level_filter(self) -> Self {
        Self {
            level_filter: LevelFilter::TRACE,
            ..self
        }
    }

    pub fn disable_ctrlc_shutdown(self) -> Self {
        Self {
            ctrlc_shutdown: false,
            ..self
        }
    }

    pub fn close_transport_on_shutdown(self) -> Self {
        Self {
            close_on_shutdown: true,
            ..self
        }
    }

    pub fn discard_push_error(self) -> Self {
        Self {
            abort_on_error: false,
            ..self
        }
    }

    async fn close(&mut self, output: RoutineOutput<T>) -> RoutineOutput<T> {
        if self.close_on_shutdown {
            self.transport.close_transport(Some((&output).into())).await;
        }

        output
    }

    async fn push_msg(&mut self, msg: TracingMsg) -> Result<(), LayerError<T>> {
        if let Err(err) = self.transport.push_msg(msg).await {
            if self.abort_on_error {
                self.close(Err(LayerError::PushMsgErr(err))).await?;
            }
        }

        Ok(())
    }

    async fn flush_close(
        mut self,
        mut recv: UnboundedReceiver<TracingMsg>,
        mut output: RoutineOutput<T>,
    ) -> RoutineOutput<T> {
        recv.close();
        let mut msgs = Vec::new();

        while let Ok(msg) = recv.try_recv() {
            msgs.push(msg);
        }

        let mut chunks = msgs.chunks(1024);

        if let Some(chunk) = chunks.next() {
            match self.transport.bulk_push(chunk.into()).await {
                Err(err) => output = Err(LayerError::BulkPushErr(err)),
                _ => {
                    if chunks.next().is_some() {
                        output = Err(LayerError::BufferFull);
                    }
                }
            }
        }

        self.close(output).await
    }

    pub fn build<S: Subscriber>(self) -> (FilteredLayer<S>, MsgRoutine<T>) {
        let level_filter = self.level_filter;
        let mut builder = self;
        let (send, mut recv) = unbounded_channel();
        let shutdown_trigger = CancellationToken::new();
        let shutdown_waiter = shutdown_trigger.clone();
        let routine = tokio::spawn(async move {
            loop {
                tokio::select! {
                    res = ctrl_c(), if builder.ctrlc_shutdown => {
                        return builder
                            .flush_close(recv, res.map(|_| GraceType::CtrlC).map_err(From::from))
                            .await;
                    }
                    _ = shutdown_waiter.cancelled() => {
                        return builder.flush_close(recv, Ok(GraceType::Explicit)).await;
                    }
                    msg = recv.recv() => match msg {
                        None => {
                            return builder.close(Err(LayerError::LayerDropped)).await;
                        }
                        Some(msg) => builder.push_msg(msg).await?,
                    }
                };
            }
        });
        let filtered_layer = MsgLayer(send).with_filter(level_filter);
        let msg_routine = MsgRoutine {
            shutdown_trigger,
            routine,
        };

        (filtered_layer, msg_routine)
    }
}

pub trait TracingLayerDefault {
    type Transport: CloseTransport + PushMsg + Clone + Debug;
    fn tracing_layer_default(&self) -> MsgLayerBuiler<Self::Transport>;
}

impl<T> TracingLayerDefault for T
where
    T: CloseTransport + PushMsg + Clone + Debug,
{
    type Transport = T;

    fn tracing_layer_default(&self) -> MsgLayerBuiler<Self::Transport> {
        MsgLayerBuiler {
            transport: self.clone(),
            level_filter: LevelFilter::DEBUG,
            ctrlc_shutdown: true,
            close_on_shutdown: false,
            abort_on_error: true,
        }
    }
}

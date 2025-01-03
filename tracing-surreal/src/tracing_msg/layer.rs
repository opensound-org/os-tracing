use super::{CloseTransport, MsgBody, PushMsg, TracingMsg};
use tokio::{
    //signal::ctrl_c,
    sync::mpsc::UnboundedSender,
    //task::{JoinError, JoinHandle},
};
//use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct MsgLayer(UnboundedSender<TracingMsg>);

impl<S: tracing_core::Subscriber> tracing_subscriber::Layer<S> for MsgLayer {
    fn on_new_span(
        &self,
        attrs: &tracing_core::span::Attributes<'_>,
        id: &tracing_core::span::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        self.0.send(MsgBody::on_new_span(attrs, id).into()).ok();
    }

    fn on_record(
        &self,
        span: &tracing_core::span::Id,
        values: &tracing_core::span::Record<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        self.0.send(MsgBody::on_record(span, values).into()).ok();
    }

    fn on_follows_from(
        &self,
        span: &tracing_core::span::Id,
        follows: &tracing_core::span::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        self.0
            .send(MsgBody::on_follows_from(span, follows).into())
            .ok();
    }

    fn on_event(
        &self,
        event: &tracing_core::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        self.0.send(MsgBody::on_event(event).into()).ok();
    }

    fn on_enter(
        &self,
        id: &tracing_core::span::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        self.0.send(MsgBody::on_enter(id).into()).ok();
    }

    fn on_exit(
        &self,
        id: &tracing_core::span::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        self.0.send(MsgBody::on_exit(id).into()).ok();
    }

    fn on_close(
        &self,
        id: tracing_core::span::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        self.0.send(MsgBody::on_close(id).into()).ok();
    }

    fn on_id_change(
        &self,
        old: &tracing_core::span::Id,
        new: &tracing_core::span::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        self.0.send(MsgBody::on_id_change(old, new).into()).ok();
    }
}

pub struct MsgRoutine;

pub struct MsgLayerBuiler<T: CloseTransport + PushMsg> {
    _transport: T,
    _ctrlc_shutdown: bool,
    _close_on_shutdown: bool,
    _abort_on_error: bool,
}

pub trait TracingLayerDefault {}

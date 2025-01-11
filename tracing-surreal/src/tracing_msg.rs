use chrono::{DateTime, Local};
use derive_more::Display;
use est::{task::TaskId, thread::ThreadId};
use indexmap::{map::Entry, IndexMap};
use serde::{Deserialize, Serialize};
use std::{error, fmt, future::Future, num::NonZeroU64, ops::Deref, thread};
use tokio::task;
use tracing_core::{field, span};

pub(crate) mod handshake;
pub mod layer;
pub mod observe;
pub mod proc_env;

pub use handshake::{ClientRole, Handshake, MsgFormat};
pub use layer::TracingLayerDefault;
pub use observe::observer;
pub use proc_env::ProcEnv;

#[derive(Debug, Display, Serialize, Deserialize, Copy, Clone, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct SpanId(pub NonZeroU64);

impl From<span::Id> for SpanId {
    fn from(value: span::Id) -> Self {
        Self(value.into_non_zero_u64())
    }
}

impl From<&span::Id> for SpanId {
    fn from(value: &span::Id) -> Self {
        Self(value.into_non_zero_u64())
    }
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug, Hash, Eq, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl From<tracing_core::Level> for Level {
    fn from(value: tracing_core::Level) -> Self {
        match value {
            tracing_core::Level::TRACE => Self::Trace,
            tracing_core::Level::DEBUG => Self::Debug,
            tracing_core::Level::INFO => Self::Info,
            tracing_core::Level::WARN => Self::Warn,
            tracing_core::Level::ERROR => Self::Error,
        }
    }
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Parent {
    Root,
    Current,
    #[serde(untagged)]
    Explicit(SpanId),
}

impl From<&span::Attributes<'_>> for Parent {
    fn from(attrs: &span::Attributes<'_>) -> Self {
        if let Some(parent) = attrs.parent() {
            Self::Explicit(parent.into())
        } else if attrs.is_root() {
            Self::Root
        } else {
            Self::Current
        }
    }
}

impl From<&tracing_core::Event<'_>> for Parent {
    fn from(event: &tracing_core::Event<'_>) -> Self {
        if let Some(parent) = event.parent() {
            Self::Explicit(parent.into())
        } else if event.is_root() {
            Self::Root
        } else {
            Self::Current
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum Value {
    Debug(String),
    F64(f64),
    I64(i64),
    U64(u64),
    I128(i128),
    U128(u128),
    Bool(bool),
    String(String),
    Bytes(Vec<u8>),
    Error(String),
}

impl Value {
    fn get_text(&self) -> Option<String> {
        match self {
            Self::Debug(value) => Some(value.clone()),
            Self::String(value) => Some(value.clone()),
            Self::Error(value) => Some(value.clone()),
            _ => None,
        }
    }

    fn nulls_removed(self) -> Self {
        match self {
            Self::Debug(value) => Self::Debug(value.replace('\0', "")),
            Self::String(value) => Self::String(value.replace('\0', "")),
            Self::Error(value) => Self::Error(value.replace('\0', "")),
            _ => self,
        }
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(transparent)]
pub struct Payload(IndexMap<String, Vec<Value>>);

impl Payload {
    pub fn insert_empty(&mut self, key: &str) {
        self.0.entry(key.into()).or_default();
    }

    pub fn record(&mut self, key: &str, value: Value) {
        match self.0.entry(key.into()) {
            Entry::Occupied(entry) => {
                entry.into_mut().push(value);
            }
            Entry::Vacant(entry) => {
                entry.insert(vec![value]);
            }
        }
    }

    fn record_field(&mut self, field: &tracing_core::Field, value: Value) {
        self.record(field.name(), value.nulls_removed());
    }
}

impl Deref for Payload {
    type Target = IndexMap<String, Vec<Value>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl field::Visit for Payload {
    fn record_debug(&mut self, field: &tracing_core::Field, value: &dyn fmt::Debug) {
        self.record_field(field, Value::Debug(format!("{:?}", value)));
    }

    fn record_f64(&mut self, field: &tracing_core::Field, value: f64) {
        self.record_field(field, Value::F64(value));
    }

    fn record_i64(&mut self, field: &tracing_core::Field, value: i64) {
        self.record_field(field, Value::I64(value));
    }

    fn record_u64(&mut self, field: &tracing_core::Field, value: u64) {
        self.record_field(field, Value::U64(value));
    }

    fn record_i128(&mut self, field: &tracing_core::Field, value: i128) {
        self.record_field(field, Value::I128(value));
    }

    fn record_u128(&mut self, field: &tracing_core::Field, value: u128) {
        self.record_field(field, Value::U128(value));
    }

    fn record_bool(&mut self, field: &tracing_core::Field, value: bool) {
        self.record_field(field, Value::Bool(value));
    }

    fn record_str(&mut self, field: &tracing_core::Field, value: &str) {
        self.record_field(field, Value::String(value.into()));
    }

    fn record_bytes(&mut self, field: &tracing_core::Field, value: &[u8]) {
        self.record_field(field, Value::Bytes(value.into()));
    }

    fn record_error(&mut self, field: &tracing_core::Field, value: &(dyn error::Error + 'static)) {
        self.record_field(field, Value::Error(value.to_string()));
    }
}

impl From<field::Iter> for Payload {
    fn from(iter: field::Iter) -> Self {
        let mut payload = Self::default();

        for field in iter {
            payload.insert_empty(field.name());
        }

        payload
    }
}

impl From<&span::Attributes<'_>> for Payload {
    fn from(attrs: &span::Attributes<'_>) -> Self {
        let mut payload = attrs.fields().iter().into();
        attrs.record(&mut payload);
        payload
    }
}

impl From<&tracing_core::Event<'_>> for Payload {
    fn from(event: &tracing_core::Event<'_>) -> Self {
        let mut payload = event.fields().into();
        event.record(&mut payload);
        payload
    }
}

impl From<&span::Record<'_>> for Payload {
    fn from(record: &span::Record<'_>) -> Self {
        let mut payload = Self::default();
        record.record(&mut payload);
        payload
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MsgBody {
    OnNewSpan {
        span_id: SpanId,
        level: Level,
        name: String,
        target: String,
        module_path: Option<String>,
        file: Option<String>,
        line: Option<u32>,
        parent: Parent,
        payload: Payload,
    },
    OnRecord {
        span_id: SpanId,
        payload: Payload,
    },
    OnFollowsFrom {
        span_id: SpanId,
        follows: SpanId,
    },
    OnEvent {
        message: String,
        level: Level,
        name: String,
        target: String,
        module_path: Option<String>,
        file: Option<String>,
        line: Option<u32>,
        parent: Parent,
        payload: Payload,
    },
    OnEnter {
        span_id: SpanId,
    },
    OnExit {
        span_id: SpanId,
    },
    OnClose {
        span_id: SpanId,
    },
    OnIdChange {
        old_span: SpanId,
        new_span: SpanId,
    },
}

impl MsgBody {
    fn on_new_span(attrs: &span::Attributes<'_>, id: &span::Id) -> Self {
        let metadata = attrs.metadata();
        let parent = Parent::from(attrs);
        let payload = Payload::from(attrs);
        let span_id = SpanId::from(id);

        (metadata, parent, payload, span_id).into()
    }

    fn on_record(span: &span::Id, values: &span::Record<'_>) -> Self {
        let span_id = span.into();
        let payload = values.into();

        Self::OnRecord { span_id, payload }
    }

    fn on_follows_from(span: &span::Id, follows: &span::Id) -> Self {
        let span_id = span.into();
        let follows = follows.into();

        Self::OnFollowsFrom { span_id, follows }
    }

    fn on_event(event: &tracing_core::Event<'_>) -> Self {
        let metadata = event.metadata();
        let parent = Parent::from(event);
        let payload = Payload::from(event);
        let message = payload
            .get("message")
            .map(Deref::deref)
            .and_then(<[_]>::first)
            .and_then(Value::get_text)
            .unwrap_or_default();

        (metadata, parent, payload, message).into()
    }

    fn on_enter(id: &span::Id) -> Self {
        let span_id = id.into();
        Self::OnEnter { span_id }
    }

    fn on_exit(id: &span::Id) -> Self {
        let span_id = id.into();
        Self::OnExit { span_id }
    }

    fn on_close(id: span::Id) -> Self {
        let span_id = id.into();
        Self::OnClose { span_id }
    }

    fn on_id_change(old: &span::Id, new: &span::Id) -> Self {
        let old_span = old.into();
        let new_span = new.into();

        Self::OnIdChange { old_span, new_span }
    }
}

impl From<(&tracing_core::Metadata<'_>, Parent, Payload, SpanId)> for MsgBody {
    fn from(
        (metadata, parent, payload, span_id): (
            &tracing_core::Metadata<'_>,
            Parent,
            Payload,
            SpanId,
        ),
    ) -> Self {
        Self::OnNewSpan {
            span_id,
            level: (*metadata.level()).into(),
            name: metadata.name().into(),
            target: metadata.target().into(),
            module_path: metadata.module_path().map(From::from),
            file: metadata.file().map(From::from),
            line: metadata.line(),
            parent,
            payload,
        }
    }
}

impl From<(&tracing_core::Metadata<'_>, Parent, Payload, String)> for MsgBody {
    fn from(
        (metadata, parent, payload, message): (
            &tracing_core::Metadata<'_>,
            Parent,
            Payload,
            String,
        ),
    ) -> Self {
        Self::OnEvent {
            message,
            level: (*metadata.level()).into(),
            name: metadata.name().into(),
            target: metadata.target().into(),
            module_path: metadata.module_path().map(From::from),
            file: metadata.file().map(From::from),
            line: metadata.line(),
            parent,
            payload,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TracingMsg {
    pub timestamp: DateTime<Local>,
    pub thread_name: Option<String>,
    pub thread_id: ThreadId,
    pub task_id: Option<TaskId>,
    #[serde(flatten)]
    pub body: MsgBody,
}

impl From<MsgBody> for TracingMsg {
    fn from(body: MsgBody) -> Self {
        let thread = thread::current();
        let timestamp = Local::now();
        let thread_name = thread.name().map(From::from);
        let thread_id = thread.id().into();
        let task_id = task::try_id().map(From::from);

        Self {
            timestamp,
            thread_name,
            thread_id,
            task_id,
            body,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Eq, PartialEq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Host,
    Pusher,
    Observer,
    Director,
}

impl Role {
    pub fn can_push(&self) -> bool {
        match self {
            Self::Observer => false,
            _ => true,
        }
    }

    pub fn can_observe(&self) -> bool {
        match self {
            Self::Pusher => false,
            _ => true,
        }
    }
}

impl From<ClientRole> for Role {
    fn from(value: ClientRole) -> Self {
        match value {
            ClientRole::Pusher => Self::Pusher,
            ClientRole::Observer => Self::Observer,
            ClientRole::Director => Self::Director,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GraceType {
    CtrlC,
    Explicit,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Eq, PartialEq, Hash)]
#[serde(tag = "type", content = "inner", rename_all = "lowercase")]
#[non_exhaustive]
pub enum CloseOk {
    Grace(GraceType),
    Other,
}

impl From<GraceType> for CloseOk {
    fn from(value: GraceType) -> Self {
        Self::Grace(value)
    }
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CloseErrKind {
    Io,
    LayerDropped,
    PushMsgErr,
    BulkPushErr,
    BufferFull,
    Other,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Hash)]
pub struct CloseErr {
    pub kind: CloseErrKind,
    pub display: String,
}

impl CloseErr {
    pub fn new(kind: CloseErrKind, display: impl fmt::Display) -> Self {
        Self {
            kind,
            display: display.to_string(),
        }
    }

    pub fn other(err: impl error::Error) -> Self {
        Self {
            kind: CloseErrKind::Other,
            display: err.to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct CloseMsg(Result<CloseOk, CloseErr>);

impl CloseMsg {
    pub fn ok(value: CloseOk) -> Self {
        Self(Ok(value))
    }

    pub fn err(err: CloseErr) -> Self {
        Self(Err(err))
    }
}

impl Deref for CloseMsg {
    type Target = Result<CloseOk, CloseErr>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[trait_variant::make(Send)]
pub trait CloseTransport: 'static {
    async fn close_transport(&mut self, msg: Option<CloseMsg>);
}

pub trait PushMsg: Send + 'static {
    type Error: error::Error + Send + 'static;

    fn bulk_push(
        &mut self,
        msgs: Vec<TracingMsg>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn push_msg(
        &mut self,
        msg: TracingMsg,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.bulk_push(vec![msg])
    }
}

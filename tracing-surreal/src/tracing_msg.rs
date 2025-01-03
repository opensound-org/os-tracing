use chrono::{DateTime, Local};
use derive_more::Display;
use est::{task::TaskId, thread::ThreadId};
use indexmap::{map::Entry, IndexMap};
use serde::{Deserialize, Serialize};
use std::{future::Future, num::NonZeroU64, ops::Deref, thread};
use tokio::task;

pub(crate) mod handshake;
pub mod layer;
pub mod proc_env;

pub use handshake::{ClientRole, Handshake, MsgFormat};
pub use layer::TracingLayerDefault;
pub use proc_env::ProcEnv;

#[derive(Debug, Display, Serialize, Deserialize, Copy, Clone, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct SpanId(pub NonZeroU64);

impl From<tracing_core::span::Id> for SpanId {
    fn from(value: tracing_core::span::Id) -> Self {
        Self(value.into_non_zero_u64())
    }
}

impl From<&tracing_core::span::Id> for SpanId {
    fn from(value: &tracing_core::span::Id) -> Self {
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
    Explicit(SpanId),
}

impl From<&tracing_core::span::Attributes<'_>> for Parent {
    fn from(attrs: &tracing_core::span::Attributes<'_>) -> Self {
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
        self.record(field.name(), value);
    }
}

impl Deref for Payload {
    type Target = IndexMap<String, Vec<Value>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl tracing_core::field::Visit for Payload {
    fn record_debug(&mut self, field: &tracing_core::Field, value: &dyn std::fmt::Debug) {
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

    fn record_error(
        &mut self,
        field: &tracing_core::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        self.record_field(field, Value::Error(value.to_string()));
    }
}

impl From<tracing_core::field::Iter> for Payload {
    fn from(iter: tracing_core::field::Iter) -> Self {
        let mut payload = Self::default();

        for field in iter {
            payload.insert_empty(field.name());
        }

        payload
    }
}

impl From<&tracing_core::span::Attributes<'_>> for Payload {
    fn from(attrs: &tracing_core::span::Attributes<'_>) -> Self {
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

impl From<&tracing_core::span::Record<'_>> for Payload {
    fn from(record: &tracing_core::span::Record<'_>) -> Self {
        let mut payload = Self::default();
        record.record(&mut payload);
        payload
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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
    pub fn on_new_span(
        attrs: &tracing_core::span::Attributes<'_>,
        id: &tracing_core::span::Id,
    ) -> Self {
        let metadata = attrs.metadata();
        let parent = Parent::from(attrs);
        let payload = Payload::from(attrs);
        let span_id = SpanId::from(id);

        (metadata, parent, payload, span_id).into()
    }

    pub fn on_record(
        span: &tracing_core::span::Id,
        values: &tracing_core::span::Record<'_>,
    ) -> Self {
        let span_id = span.into();
        let payload = values.into();

        Self::OnRecord { span_id, payload }
    }

    pub fn on_follows_from(
        span: &tracing_core::span::Id,
        follows: &tracing_core::span::Id,
    ) -> Self {
        let span_id = span.into();
        let follows = follows.into();

        Self::OnFollowsFrom { span_id, follows }
    }

    pub fn on_event(event: &tracing_core::Event<'_>) -> Self {
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

    pub fn on_enter(id: &tracing_core::span::Id) -> Self {
        let span_id = id.into();
        Self::OnEnter { span_id }
    }

    pub fn on_exit(id: &tracing_core::span::Id) -> Self {
        let span_id = id.into();
        Self::OnExit { span_id }
    }

    pub fn on_close(id: tracing_core::span::Id) -> Self {
        let span_id = id.into();
        Self::OnClose { span_id }
    }

    pub fn on_id_change(old: &tracing_core::span::Id, new: &tracing_core::span::Id) -> Self {
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

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum GracefulType {
    CtrlC,
    Explicit,
}

pub trait CloseTransport: Send {
    fn close_transport(&mut self) -> impl Future<Output = ()> + Send {
        async {}
    }
}

#[trait_variant::make(Send)]
pub trait PushMsg {
    type Error;
    async fn push_msg(&mut self, msg: TracingMsg) -> Result<(), Self::Error>;
}

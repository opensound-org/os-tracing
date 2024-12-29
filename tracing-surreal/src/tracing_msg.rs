use chrono::{DateTime, Local};
use derive_more::Display;
use est::{task::TaskId, thread::ThreadId};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::{num::NonZeroU64, thread};
use tokio::task::{self, spawn_blocking};

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Eq, PartialEq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ClientRole {
    Pusher,
    Observer,
    Director,
}

#[derive(Serialize, Deserialize, Default, Debug, Copy, Clone, Eq, PartialEq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum MsgFormat {
    Json,
    #[default]
    Bincode,
    Msgpack,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Hash)]
pub struct ProcEnv {
    pub proc_id: u32,
    pub proc_name: Option<String>,
}

impl ProcEnv {
    // Need to gate this under `proc-env` & `sysinfo` & `wgpu` feature flag.
    pub fn create() -> Self {
        let proc_id = std::process::id();
        let proc_name = current_exe_name().ok();

        Self { proc_id, proc_name }
    }

    pub async fn create_async() -> Option<Self> {
        spawn_blocking(Self::create).await.ok()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Hash)]
pub struct Handshake {
    pub client_name: String,
    pub msg_format: MsgFormat,
    pub proc_env: Option<ProcEnv>,
}

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

// todo: impl From<&tracing_core::span::Attributes> for Parent
// todo: impl From<&tracing_core::Event> for Parent

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
enum Value {
    Debug(String),
    F64(f64),
    I64(i64),
    U64(u64),
    I128(i128),
    U128(u128),
    Bool(bool),
    String(String),
    Error(String),
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(transparent)]
pub struct Payload(IndexMap<String, Vec<Value>>);

// todo: impl Deref for Payload
// todo: impl tracing_core::field::Visit for Payload
// todo: impl From<&tracing_core::span::Attributes> for Payload
// todo: impl From<&tracing_core::span::Record> for Payload
// todo: impl From<&tracing_core::Event> for Payload

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

// todo: fn on_new_span() etc...
// todo: impl From<(&tracing_core::Metadata, Parent, Payload, SpanId)> for MsgBody
// todo: impl From<(&tracing_core::Metadata, Parent, Payload, String)> for MsgBody

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

// todo: #[trait_variant::make(Send)]
// todo: PushMsg, TracingMsg, MsgLayer, MsgRoutine, tracing-core, tracing-subscriber::Layer

// Need to gate this under `experimental` feature flag.
#[doc(hidden)]
pub fn current_exe_name() -> std::io::Result<String> {
    Ok(std::env::current_exe()?
        .file_name()
        .expect("this should not happen here")
        .to_string_lossy()
        .into())
}

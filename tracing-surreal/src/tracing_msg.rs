use chrono::{DateTime, Local};
use derive_more::Display;
use est::{task::TaskId, thread::ThreadId};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;

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
pub struct Handshake {
    pub client_name: String,
    pub proc_name: String,
    pub proc_id: u32,
    pub msg_format: MsgFormat,
}

#[derive(Debug, Display, Serialize, Deserialize, Copy, Clone, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct SpanId(pub NonZeroU64);

#[derive(Serialize, Deserialize, Copy, Clone, Debug, Hash, Eq, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Parent {
    Root,
    Current,
    Explicit(SpanId),
}

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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Payload(IndexMap<String, Vec<Value>>);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum MsgBody {
    OnNewSpan {
        id: SpanId,
        name: String,
        target: String,
        level: Level,
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
        id: SpanId,
        follows: SpanId,
    },
    OnEvent {
        name: String,
        target: String,
        level: Level,
        message: String,
        module_path: Option<String>,
        file: Option<String>,
        line: Option<u32>,
        parent: Parent,
        payload: Payload,
    },
    OnEnter {
        id: SpanId,
    },
    OnExit {
        id: SpanId,
    },
    OnClose {
        id: SpanId,
    },
    OnIdChange {
        old: SpanId,
        new: SpanId,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TracingMsg {
    pub timestamp: DateTime<Local>,
    pub thread_name: Option<String>,
    pub thread_id: ThreadId,
    pub task_id: Option<TaskId>,
    pub body: MsgBody,
}

// todo: PushMsg, tracing-core, tracing-subscriber::Layer

// Need to gate this under `experimental` feature flag.
#[doc(hidden)]
pub fn current_exe_name() -> std::io::Result<String> {
    Ok(std::env::current_exe()?
        .file_name()
        .expect("this should not happen here")
        .to_string_lossy()
        .into())
}

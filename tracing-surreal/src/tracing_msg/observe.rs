use super::{CloseMsg, HelloMsg, Role, TracingMsg};
use chrono::{DateTime, Local};
use either::Either;
use serde::{Deserialize, Serialize};
use std::{mem, net::SocketAddr};
use tokio::sync::broadcast::Receiver;

pub use tokio::sync::broadcast::error::RecvError;

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Hash)]
pub struct ClientId(String, String);

impl From<surrealdb::RecordId> for ClientId {
    fn from(value: surrealdb::RecordId) -> Self {
        Self(value.table().into(), value.key().to_string())
    }
}

impl From<ClientId> for surrealdb::RecordId {
    fn from(value: ClientId) -> Self {
        (value.0, value.1).into()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct ClientInfo {
    pub hello_timestamp: DateTime<Local>,
    pub hello_msg: HelloMsg,
    pub client_role: Role,
    pub client_addr: Option<SocketAddr>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct CloseInfo {
    pub close_timestamp: DateTime<Local>,
    pub client_info: Either<ClientId, ClientInfo>,
    pub close_msg: CloseMsg,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MsgInfo {
    pub client_info: Either<ClientId, ClientInfo>,
    pub tracing_msg: TracingMsg,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ObserveMsg {
    OnClientHello(ClientId, ClientInfo),
    OnDisconnect(CloseInfo),
    OnMsg(MsgInfo),
}

#[derive(Debug)]
pub struct Observer {
    history: Vec<ObserveMsg>,
    live: Receiver<ObserveMsg>,
}

impl Observer {
    pub fn history(&mut self) -> Vec<ObserveMsg> {
        mem::take(&mut self.history)
    }

    pub async fn next_live(&mut self) -> Result<ObserveMsg, RecvError> {
        self.live.recv().await
    }
}

pub fn observer(history: Vec<ObserveMsg>, live: Receiver<ObserveMsg>) -> Observer {
    Observer { history, live }
}

use super::{CloseMsg, Handshake, Role, TracingMsg};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::{mem, net::SocketAddr};
use tokio::sync::broadcast::Receiver;

pub use tokio::sync::broadcast::error::RecvError;

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct ClientInfo {
    pub handshake_timestamp: DateTime<Local>,
    pub handshake_info: Handshake,
    pub client_role: Role,
    pub client_addr: Option<SocketAddr>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct CloseInfo {
    pub close_timestamp: DateTime<Local>,
    pub client_info: ClientInfo,
    pub close_msg: CloseMsg,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MsgInfo {
    pub client_info: ClientInfo,
    pub tracing_msg: TracingMsg,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ObserveMsg {
    OnClientHandshake(ClientInfo),
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

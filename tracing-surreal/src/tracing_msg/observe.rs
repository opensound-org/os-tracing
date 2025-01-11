use super::{Handshake, Role};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct ClientInfo {
    pub handshake_timestamp: DateTime<Local>,
    pub handshake_info: Handshake,
    pub client_role: Role,
    pub client_addr: Option<SocketAddr>,
}

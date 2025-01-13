use super::ProcEnv;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Debug, Copy, Clone, Eq, PartialEq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum MsgFormat {
    Json,
    #[default]
    Bincode,
    Msgpack,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub struct Handshake {
    pub client_name: String,
    pub msg_format: MsgFormat,
    pub proc_env: Option<ProcEnv>,
}

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum Role {
    Host,
    Pusher,
    Observer,
    Director,
}

#[derive(Serialize, Deserialize, Default, Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum MsgFormat {
    Json,
    #[default]
    Bincode,
    Msgpack,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Hash)]
pub struct ClientHandshake {
    pub client_name: String,
    pub proc_name: String,
    pub proc_id: u32,
    pub msg_format: MsgFormat,
}

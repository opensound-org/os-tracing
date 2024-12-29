use crate::tracing_msg::{ClientRole, Handshake, MsgFormat, ProcEnv};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, io, net::SocketAddr};
use surrealdb::{Connection, RecordId, Surreal};
use thiserror::Error;
use ulid::Ulid;

#[derive(Error, Debug)]
pub enum StopError {
    #[error("surrealdb error: `{0}`")]
    Surreal(#[from] surrealdb::Error),
    #[error("io error: `{0}`")]
    Io(#[from] io::Error),
}

#[derive(Clone, Debug)]
pub struct Stop<C: Connection> {
    db: Surreal<C>,
    session_id: RecordId,
    formatted_timestamp: String,
    client_id: RecordId,
    can_push: bool,
    can_observe: bool,
}

#[derive(Deserialize)]
struct RID {
    id: RecordId,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Eq, PartialEq, Hash)]
#[serde(rename_all = "lowercase")]
enum Role {
    Host,
    Pusher,
    Observer,
    Director,
}

impl Role {
    fn can_push(&self) -> bool {
        match self {
            Self::Observer => false,
            _ => true,
        }
    }

    fn can_observe(&self) -> bool {
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

impl<C: Connection> Stop<C> {
    pub async fn init(db: Surreal<C>, app: &str) -> Result<Self, StopError> {
        db.use_db(format!("app-tracing-{}", app)).await?;

        #[derive(Serialize)]
        struct SessionRecord {
            a_timestamp: DateTime<Local>,
            b_access_method: Option<String>,
            c_record_auth: Option<String>,
            d_http_origin: Option<String>,
            e_session_ip: Option<String>,
            f_session_id: Option<String>,
            g_session_token: Option<Value>,
        }

        let a_timestamp = Local::now();
        let b_access_method = db.run("session::ac").await?;
        let c_record_auth = db.run("session::rd").await?;
        let d_http_origin = db.run("session::origin").await?;
        let e_session_ip = db.run("session::ip").await?;
        let f_session_id = db.run("session::id").await?;
        let g_session_token = db.run("session::token").await?;
        let record = SessionRecord {
            a_timestamp,
            b_access_method,
            c_record_auth,
            d_http_origin,
            e_session_ip,
            f_session_id,
            g_session_token,
        };
        let rid: Option<RID> = db
            .create((
                ".sessions",
                Ulid::from_datetime(a_timestamp.into()).to_string(),
            ))
            .content(record)
            .await?;
        let session_id = rid.unwrap().id;
        let formatted_timestamp = a_timestamp.format("%y%m%d-%H%M%S").to_string();
        let client_name = app;
        let client_role = Role::Host;
        let msg_format = None;
        let client_addr = None;
        let query_map = None;
        let proc_env = ProcEnv::create_async().await;

        Ok(Self::handshake_internal(
            &db,
            &session_id,
            &formatted_timestamp,
            client_name,
            client_role,
            msg_format,
            client_addr,
            &query_map,
            &proc_env,
        )
        .await?)
    }

    pub async fn client_handshake(
        &self,
        client_info: Handshake,
        client_role: ClientRole,
        client_addr: SocketAddr,
        query_map: Option<HashMap<String, String>>,
    ) -> surrealdb::Result<Self> {
        Self::handshake_internal(
            &self.db,
            &self.session_id,
            &self.formatted_timestamp,
            &client_info.client_name,
            client_role.into(),
            Some(client_info.msg_format),
            Some(client_addr),
            &query_map,
            &client_info.proc_env,
        )
        .await
    }

    async fn handshake_internal(
        db: &Surreal<C>,
        session_id: &RecordId,
        formatted_timestamp: &str,
        client_name: &str,
        client_role: Role,
        msg_format: Option<MsgFormat>,
        client_addr: Option<SocketAddr>,
        query_map: &Option<HashMap<String, String>>,
        proc_env: &Option<ProcEnv>,
    ) -> surrealdb::Result<Self> {
        #[derive(Serialize)]
        struct ClientRecord {
            a_timestamp: DateTime<Local>,
            b_session_id: RecordId,
            c_client_name: String,
            d_client_role: Role,
            e_msg_format: Option<MsgFormat>,
            f_client_addr: Option<SocketAddr>,
            g_query_map: Option<HashMap<String, String>>,
            h_proc_env: Option<ProcEnv>,
        }

        let a_timestamp = Local::now();
        let b_session_id = session_id.clone();
        let c_client_name = client_name.into();
        let d_client_role = client_role;
        let e_msg_format = msg_format;
        let f_client_addr = client_addr;
        let g_query_map = query_map.clone();
        let h_proc_env = proc_env.clone();
        let record = ClientRecord {
            a_timestamp,
            b_session_id,
            c_client_name,
            d_client_role,
            e_msg_format,
            f_client_addr,
            g_query_map,
            h_proc_env,
        };
        let rid: Option<RID> = db
            .create((
                format!("{}-clients", formatted_timestamp),
                Ulid::from_datetime(a_timestamp.into()).to_string(),
            ))
            .content(record)
            .await?;
        let db = db.clone();
        let session_id = session_id.clone();
        let formatted_timestamp = formatted_timestamp.into();
        let client_id = rid.unwrap().id;
        let can_push = client_role.can_push();
        let can_observe = client_role.can_observe();

        Ok(Self {
            db,
            session_id,
            formatted_timestamp,
            client_id,
            can_push,
            can_observe,
        })
    }

    pub async fn print(&self) {
        println!("{}", self.db.version().await.unwrap());
        println!("{}", self.session_id);
        println!("{}", self.formatted_timestamp);
        println!("{}", self.client_id);
        println!("{}", self.can_push);
        println!("{}", self.can_observe);
    }
}

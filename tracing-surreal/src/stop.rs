use crate::tracing_msg::{current_exe_name, ClientHandshake, MsgFormat, Role};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, io, net::SocketAddr, process};
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
}

#[derive(Deserialize)]
struct RID {
    id: RecordId,
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
        let proc_name = current_exe_name()?;
        let proc_id = process::id();
        let msg_format = None;
        let client_addr = None;
        let query_map = None;

        Ok(Self::handshake_internal(
            &db,
            &session_id,
            &formatted_timestamp,
            client_name,
            client_role,
            &proc_name,
            proc_id,
            msg_format,
            client_addr,
            &query_map,
        )
        .await?)
    }

    pub async fn client_handshake(
        &self,
        client_info: ClientHandshake,
        client_role: Role,
        client_addr: SocketAddr,
        query_map: Option<HashMap<String, String>>,
    ) -> surrealdb::Result<Self> {
        Self::handshake_internal(
            &self.db,
            &self.session_id,
            &self.formatted_timestamp,
            &client_info.client_name,
            client_role,
            &client_info.proc_name,
            client_info.proc_id,
            Some(client_info.msg_format),
            Some(client_addr),
            &query_map,
        )
        .await
    }

    async fn handshake_internal(
        db: &Surreal<C>,
        session_id: &RecordId,
        formatted_timestamp: &str,
        client_name: &str,
        client_role: Role,
        proc_name: &str,
        proc_id: u32,
        msg_format: Option<MsgFormat>,
        client_addr: Option<SocketAddr>,
        query_map: &Option<HashMap<String, String>>,
    ) -> surrealdb::Result<Self> {
        #[derive(Serialize)]
        struct ClientRecord {
            a_timestamp: DateTime<Local>,
            b_session_id: RecordId,
            c_client_name: String,
            d_client_role: Role,
            e_proc_name: String,
            f_proc_id: u32,
            g_msg_format: Option<MsgFormat>,
            h_client_addr: Option<SocketAddr>,
            i_query_map: Option<HashMap<String, String>>,
        }

        let a_timestamp = Local::now();
        let b_session_id = session_id.clone();
        let c_client_name = client_name.into();
        let d_client_role = client_role;
        let e_proc_name = proc_name.into();
        let f_proc_id = proc_id;
        let g_msg_format = msg_format;
        let h_client_addr = client_addr;
        let i_query_map = query_map.clone();
        let record = ClientRecord {
            a_timestamp,
            b_session_id,
            c_client_name,
            d_client_role,
            e_proc_name,
            f_proc_id,
            g_msg_format,
            h_client_addr,
            i_query_map,
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

        Ok(Self {
            db,
            session_id,
            formatted_timestamp,
            client_id,
        })
    }

    pub async fn print(&self) {
        println!("{}", self.db.version().await.unwrap());
        println!("{}", self.session_id);
        println!("{}", self.formatted_timestamp);
        println!("{}", self.client_id);
    }
}

use crate::tracing_msg::{
    ClientRole, CloseErr, CloseErrKind, CloseMsg, CloseOk, CloseTransport, Handshake, MsgFormat,
    ProcEnv, PushMsg, TracingMsg,
};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, fmt, io, net::SocketAddr, sync::Arc};
use surrealdb::{Connection, RecordId, Surreal};
use thiserror::Error;
use tokio::sync::RwLock;
use ulid::Generator;

pub use crate::tracing_msg;
pub use surrealdb;

#[derive(Error, Debug)]
pub enum StopError {
    #[error("surrealdb error: `{0}`")]
    Surreal(#[from] surrealdb::Error),
    #[error("io error: `{0}`")]
    Io(#[from] io::Error),
    #[error("observer cannot push")]
    ObserverCannotPush,
}

#[derive(Clone, Default)]
struct IdGen(Arc<RwLock<Generator>>);

impl IdGen {
    async fn next(&self, timestamp: DateTime<Local>) -> String {
        let datetime = timestamp.into();
        let mut gen = self.0.write().await;

        loop {
            if let Ok(ulid) = gen.generate_from_datetime(datetime) {
                return ulid.to_string();
            }

            *gen = Default::default();
        }
    }
}

impl fmt::Debug for IdGen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("IdGen").field(&"Generator").finish()
    }
}

#[derive(Clone, Debug)]
pub struct Stop<C: Connection> {
    db: Surreal<C>,
    id_gen: IdGen,
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

#[derive(Clone, Debug)]
pub struct StopBuilder<C: Connection> {
    db: Surreal<C>,
    app: String,
    host: String,
}

impl<C: Connection> StopBuilder<C> {
    pub fn host_name(self, host: &str) -> Self {
        Self {
            host: host.into(),
            ..self
        }
    }

    pub async fn init(self) -> Result<Stop<C>, StopError> {
        let db = self.db;

        db.use_db(format!("app-tracing-{}", self.app)).await?;

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

        let id_gen = IdGen::default();
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
            .create((".sessions", id_gen.next(a_timestamp).await))
            .content(record)
            .await?;
        let session_id = rid.unwrap().id;
        let formatted_timestamp = a_timestamp.format("%y%m%d-%H%M%S").to_string();
        let client_name = self.host;
        let client_role = Role::Host;
        let msg_format = None;
        let client_addr = None;
        let query_map = None;
        let proc_env = ProcEnv::create_async().await;

        Ok(Stop::handshake_internal(
            &db,
            &id_gen,
            &session_id,
            &formatted_timestamp,
            &client_name,
            client_role,
            msg_format,
            client_addr,
            &query_map,
            &proc_env,
        )
        .await?)
    }
}

impl<C: Connection> Stop<C> {
    pub fn builder_default(db: Surreal<C>, app: &str) -> StopBuilder<C> {
        StopBuilder {
            db,
            app: app.into(),
            host: "host".into(),
        }
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
            &self.id_gen,
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
        id_gen: &IdGen,
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
            h_proc_env: Option<Value>,
        }

        let a_timestamp = Local::now();
        let b_session_id = session_id.clone();
        let c_client_name = client_name.into();
        let d_client_role = client_role;
        let e_msg_format = msg_format;
        let f_client_addr = client_addr;
        let g_query_map = query_map.clone();
        let h_proc_env = proc_env.as_ref().and_then(|v| serde_json::to_value(v).ok());
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
                id_gen.next(a_timestamp).await,
            ))
            .content(record)
            .await?;
        let db = db.clone();
        let id_gen = id_gen.clone();
        let session_id = session_id.clone();
        let formatted_timestamp = formatted_timestamp.into();
        let client_id = rid.unwrap().id;
        let can_push = client_role.can_push();
        let can_observe = client_role.can_observe();

        Ok(Self {
            db,
            id_gen,
            session_id,
            formatted_timestamp,
            client_id,
            can_push,
            can_observe,
        })
    }

    pub async fn query_last_n(&self, n: u8) -> Result<Vec<String>, surrealdb::Error> {
        #[derive(Serialize)]
        struct Bind {
            record_id_range: String,
            n: u8,
        }

        // ..=01JH8CBYAKFSRK8Y9HY02QVTDS
        // ..={last_id}
        let record_id_range = format!("⟨{}-msg⟩:..", self.formatted_timestamp);

        #[derive(Deserialize)]
        struct Msg {
            message: String,
        }

        // SELECT * FROM (SELECT *, session_id[*], client_id[*] FROM type::record($record_id_range) ORDER BY id DESC LIMIT $n) ORDER BY id
        let query = "SELECT * FROM (SELECT * FROM type::record($record_id_range) ORDER BY id DESC LIMIT $n) ORDER BY id";
        let msgs: Vec<Msg> = self
            .db
            .query(query)
            .bind(Bind { record_id_range, n })
            .await?
            .take(0)?;

        // clients + disconnects & merge

        Ok(msgs.iter().map(|m| m.message.clone()).collect())
    }

    pub async fn print(&self) {
        println!("{}", self.can_observe);
    }
}

impl<C: Connection> CloseTransport for Stop<C> {
    async fn close_transport(&mut self, msg: Option<CloseMsg>) {
        if let Some(msg) = msg {
            #[derive(Serialize)]
            struct DisconnectRecord {
                a_timestamp: DateTime<Local>,
                b_session_id: RecordId,
                c_client_id: RecordId,
                d_normal: bool,
                e_ok_kind: Option<CloseOk>,
                f_err_kind: Option<CloseErrKind>,
                g_err_msg: Option<String>,
            }

            let a_timestamp = Local::now();
            let b_session_id = self.session_id.clone();
            let c_client_id = self.client_id.clone();
            let d_normal = msg.is_ok();
            let e_ok_kind = msg.as_ref().ok().copied();
            let (f_err_kind, g_err_msg) = match msg.as_ref().err() {
                None => (None, None),
                Some(CloseErr { kind, display }) => (Some(*kind), Some(display.clone())),
            };
            let record = DisconnectRecord {
                a_timestamp,
                b_session_id,
                c_client_id,
                d_normal,
                e_ok_kind,
                f_err_kind,
                g_err_msg,
            };
            let _rid: Option<Option<RID>> = self
                .db
                .create((
                    format!("{}-disconnects", self.formatted_timestamp),
                    self.id_gen.next(a_timestamp).await,
                ))
                .content(record)
                .await
                .ok();
        }
    }
}

impl<C: Connection> PushMsg for Stop<C> {
    type Error = StopError;

    async fn bulk_push(&mut self, msgs: Vec<TracingMsg>) -> Result<(), Self::Error> {
        if !self.can_push {
            return Err(StopError::ObserverCannotPush);
        }

        if msgs.is_empty() {
            return Ok(());
        }

        #[derive(Serialize)]
        struct MsgRecord {
            id: RecordId,
            session_id: RecordId,
            client_id: RecordId,
            #[serde(flatten)]
            msg: TracingMsg,
        }

        let table_name = format!("{}-msg", self.formatted_timestamp);
        let mut records = Vec::new();

        for msg in msgs {
            let id = RecordId::from_table_key(&table_name, self.id_gen.next(msg.timestamp).await);
            let session_id = self.session_id.clone();
            let client_id = self.client_id.clone();

            records.push(MsgRecord {
                id,
                session_id,
                client_id,
                msg,
            });
        }

        let _rids: Vec<RID> = self.db.insert(table_name).content(records).await?;

        Ok(())
    }
}

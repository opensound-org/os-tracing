//#![allow(warnings)]

use crate::{
    async_req_res::{req_res, Requester},
    tracing_msg::{
        observe::{CloseInfo, MsgInfo},
        ClientInfo, ClientRole, CloseErr, CloseErrKind, CloseMsg, CloseOk, CloseTransport,
        GraceType, HelloMsg, MsgFormat, ObserveMsg, Observer, ProcEnv, PushMsg, QueryHistory, Role,
        TracingMsg,
    },
};
use chrono::{DateTime, Local};
use either::Either;
use futures::StreamExt;
use indexmap::IndexMap;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::{
    fmt,
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use surrealdb::{
    method::{QueryStream, Stream},
    value::{Action, Notification},
    Connection, RecordId, RecordIdKey, Surreal,
};
use thiserror::Error;
use tokio::{
    sync::{broadcast, oneshot, RwLock},
    task::{JoinError, JoinHandle},
};
use tokio_util::sync::CancellationToken;
use ulid::Generator;

pub use crate::tracing_msg;
pub use surrealdb;

#[derive(Error, Debug)]
pub enum StopError {
    #[error("surrealdb error: `{0}`")]
    Surreal(#[from] surrealdb::Error),
    #[error("io error: `{0}`")]
    Io(#[from] io::Error),
    #[error("serde_json error: `{0}`")]
    Json(#[from] serde_json::Error),
    #[error("observer cannot push")]
    ObserverCannotPush,
    #[error("client cannot explicitly observe")]
    ClientCannotObserve,
    #[error("observer/director must fill QueryHistory on hello")]
    MustFillQueryHistory,
    #[error("live query stream closed")]
    StreamClosed,
    #[error("corrupted data")]
    CorruptedData,
    #[error("requester dropped")]
    RequesterDropped,
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

type ObserverRequester = Requester<QueryHistory, Result<Observer, StopError>>;

#[derive(Clone, Debug)]
pub struct Stop<C: Connection> {
    db: Surreal<C>,
    id_gen: IdGen,
    session_id: RecordId,
    formatted_timestamp: String,
    client_id: RecordId,
    can_push: bool,
    is_client: bool,
    link_client: bool,
    ob_requester: ObserverRequester,
}

type RoutineOutput = Result<GraceType, StopError>;
type HandleOutput = Result<RoutineOutput, JoinError>;

#[derive(Debug)]
pub struct ObserveRoutine {
    shutdown_trigger: CancellationToken,
    routine: JoinHandle<RoutineOutput>,
}

impl ObserveRoutine {
    pub fn trigger_graceful_shutdown(&self) {
        self.shutdown_trigger.cancel();
    }

    pub async fn graceful_shutdown(self) -> HandleOutput {
        self.trigger_graceful_shutdown();
        self.await
    }
}

impl Future for ObserveRoutine {
    type Output = HandleOutput;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.routine).poll(cx)
    }
}

#[derive(Deserialize)]
struct RID {
    id: RecordId,
}

#[derive(Clone, Debug)]
pub struct StopBuilder<C: Connection> {
    db: Surreal<C>,
    app: String,
    host: String,
    link_client: bool,
    ctrlc_shutdown: bool,
}

impl<C: Connection> StopBuilder<C> {
    pub fn host_name(self, host: &str) -> Self {
        Self {
            host: host.into(),
            ..self
        }
    }

    pub fn no_link_client(self) -> Self {
        Self {
            link_client: false,
            ..self
        }
    }

    pub fn disable_ctrlc_shutdown(self) -> Self {
        Self {
            ctrlc_shutdown: false,
            ..self
        }
    }

    pub async fn init(self) -> Result<(Stop<C>, ObserveRoutine), StopError> {
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
            h_link_client: bool,
        }

        let id_gen = IdGen::default();
        let link_client = self.link_client;
        let a_timestamp = Local::now();
        let b_access_method = db.run("session::ac").await?;
        let c_record_auth = db.run("session::rd").await?;
        let d_http_origin = db.run("session::origin").await?;
        let e_session_ip = db.run("session::ip").await?;
        let f_session_id = db.run("session::id").await?;
        let g_session_token = db.run("session::token").await?;
        let h_link_client = link_client;
        let record = SessionRecord {
            a_timestamp,
            b_access_method,
            c_record_auth,
            d_http_origin,
            e_session_ip,
            f_session_id,
            g_session_token,
            h_link_client,
        };
        let rid: Option<RID> = db
            .create((".sessions", id_gen.next(a_timestamp).await))
            .content(record)
            .await?;
        let formatted_timestamp = a_timestamp.format("%y%m%d-%H%M%S").to_string();

        db.query(if link_client {
            include_str!("surql/fns_link.surql")
        } else {
            include_str!("surql/fns.surql")
        })
        .await?
        .check()?;

        trait ToObserveMsg {
            fn get_id(&self) -> &RecordId;
            fn to_observe_msg(self) -> Result<ObserveMsg, StopError>;
        }

        #[derive(Deserialize)]
        struct ClientModel {
            id: RecordId,
            a_timestamp: DateTime<Local>,
            c_client_name: String,
            d_client_role: Role,
            g_client_addr: Option<SocketAddr>,
            i_proc_env: Option<Value>,
        }

        impl ClientModel {
            fn to_client_info(self) -> Result<ClientInfo, StopError> {
                let hello_timestamp = self.a_timestamp;
                let client_name = self.c_client_name;
                let proc_env = match self.i_proc_env {
                    None => None,
                    Some(v) => serde_json::from_value(v)?,
                };
                let hello_msg = HelloMsg {
                    client_name,
                    proc_env,
                };
                let client_role = self.d_client_role;
                let client_addr = self.g_client_addr;

                Ok(ClientInfo {
                    hello_timestamp,
                    hello_msg,
                    client_role,
                    client_addr,
                })
            }
        }

        impl ToObserveMsg for ClientModel {
            fn get_id(&self) -> &RecordId {
                &self.id
            }

            fn to_observe_msg(self) -> Result<ObserveMsg, StopError> {
                let client_id = self.id.clone().into();
                let client_info = self.to_client_info()?;

                Ok(ObserveMsg::OnClientHello(client_id, client_info))
            }
        }

        #[derive(Deserialize)]
        struct DisconnectIdModel {
            id: RecordId,
            a_timestamp: DateTime<Local>,
            c_client_id: RecordId,
            d_normal: bool,
            e_ok_kind: Option<CloseOk>,
            f_err_kind: Option<CloseErrKind>,
            g_err_msg: Option<String>,
        }

        impl ToObserveMsg for DisconnectIdModel {
            fn get_id(&self) -> &RecordId {
                &self.id
            }

            fn to_observe_msg(self) -> Result<ObserveMsg, StopError> {
                let msg_key = self.id.key().to_string();
                let close_timestamp = self.a_timestamp;
                let client_info = Either::Left(self.c_client_id.into());
                let close_msg = if self.d_normal {
                    CloseMsg::ok(self.e_ok_kind.ok_or(StopError::CorruptedData)?)
                } else {
                    CloseMsg::err(CloseErr::new(
                        self.f_err_kind.ok_or(StopError::CorruptedData)?,
                        self.g_err_msg.ok_or(StopError::CorruptedData)?,
                    ))
                };
                let close_info = CloseInfo {
                    close_timestamp,
                    client_info,
                    close_msg,
                };

                Ok(ObserveMsg::OnDisconnect(msg_key, close_info))
            }
        }

        #[derive(Deserialize)]
        struct DisconnectClientModel {
            id: RecordId,
            a_timestamp: DateTime<Local>,
            c_client_id: ClientModel,
            d_normal: bool,
            e_ok_kind: Option<CloseOk>,
            f_err_kind: Option<CloseErrKind>,
            g_err_msg: Option<String>,
        }

        impl ToObserveMsg for DisconnectClientModel {
            fn get_id(&self) -> &RecordId {
                &self.id
            }

            fn to_observe_msg(self) -> Result<ObserveMsg, StopError> {
                let msg_key = self.id.key().to_string();
                let close_timestamp = self.a_timestamp;
                let client_info = Either::Right(self.c_client_id.to_client_info()?);
                let close_msg = if self.d_normal {
                    CloseMsg::ok(self.e_ok_kind.ok_or(StopError::CorruptedData)?)
                } else {
                    CloseMsg::err(CloseErr::new(
                        self.f_err_kind.ok_or(StopError::CorruptedData)?,
                        self.g_err_msg.ok_or(StopError::CorruptedData)?,
                    ))
                };
                let close_info = CloseInfo {
                    close_timestamp,
                    client_info,
                    close_msg,
                };

                Ok(ObserveMsg::OnDisconnect(msg_key, close_info))
            }
        }

        #[derive(Deserialize)]
        struct MsgIdModel {
            id: RecordId,
            client_id: RecordId,
            #[serde(flatten)]
            msg: TracingMsg,
        }

        impl ToObserveMsg for MsgIdModel {
            fn get_id(&self) -> &RecordId {
                &self.id
            }

            fn to_observe_msg(self) -> Result<ObserveMsg, StopError> {
                let msg_key = self.id.key().to_string();
                let client_info = Either::Left(self.client_id.into());
                let tracing_msg = self.msg;
                let msg_info = MsgInfo {
                    client_info,
                    tracing_msg,
                };

                Ok(ObserveMsg::OnMsg(msg_key, msg_info))
            }
        }

        #[derive(Deserialize)]
        struct MsgClientModel {
            id: RecordId,
            client_id: ClientModel,
            #[serde(flatten)]
            msg: TracingMsg,
        }

        impl ToObserveMsg for MsgClientModel {
            fn get_id(&self) -> &RecordId {
                &self.id
            }

            fn to_observe_msg(self) -> Result<ObserveMsg, StopError> {
                let msg_key = self.id.key().to_string();
                let client_info = Either::Right(self.client_id.to_client_info()?);
                let tracing_msg = self.msg;
                let msg_info = MsgInfo {
                    client_info,
                    tracing_msg,
                };

                Ok(ObserveMsg::OnMsg(msg_key, msg_info))
            }
        }

        fn handle_item<T: ToObserveMsg>(
            item: Notification<T>,
            last_key: &mut Option<RecordIdKey>,
            br_send: &broadcast::Sender<ObserveMsg>,
        ) -> Result<(), StopError> {
            if item.action == Action::Create {
                let model = item.data;
                *last_key = Some(model.get_id().key().clone());
                br_send.send(model.to_observe_msg()?).ok();
            }

            Ok(())
        }

        enum UnifiedStream<I, C> {
            Select(Stream<Vec<I>>),
            Query(QueryStream<Notification<C>>),
        }

        impl<I: DeserializeOwned + Unpin, C: DeserializeOwned + Unpin> UnifiedStream<I, C> {
            async fn next(
                &mut self,
            ) -> Option<surrealdb::Result<Either<Notification<I>, Notification<C>>>> {
                match self {
                    Self::Select(s) => Some(s.next().await?.map(Either::Left)),
                    Self::Query(s) => Some(s.next().await?.map(Either::Right)),
                }
            }
        }

        impl<I, C> From<Stream<Vec<I>>> for UnifiedStream<I, C> {
            fn from(value: Stream<Vec<I>>) -> Self {
                Self::Select(value)
            }
        }

        impl<I, C> From<QueryStream<Notification<C>>> for UnifiedStream<I, C> {
            fn from(value: QueryStream<Notification<C>>) -> Self {
                Self::Query(value)
            }
        }

        #[derive(Serialize)]
        struct TableName {
            table_name: String,
        }

        impl TableName {
            fn new(table_name: String) -> Self {
                Self { table_name }
            }
        }

        let live_link_ql = include_str!("surql/live_link.surql");
        let clients_name = format!("{}-clients", formatted_timestamp);
        let disconnects_name = format!("{}-disconnects", formatted_timestamp);
        let msg_name = format!("{}-msg", formatted_timestamp);
        let mut client_stream: Stream<Vec<ClientModel>> = db.select(clients_name).live().await?;
        let mut close_stream: UnifiedStream<DisconnectIdModel, DisconnectClientModel> =
            if link_client {
                db.query(live_link_ql)
                    .bind(TableName::new(disconnects_name))
                    .await?
                    .stream(1)?
                    .into()
            } else {
                db.select(disconnects_name).live().await?.into()
            };
        let mut msg_stream: UnifiedStream<MsgIdModel, MsgClientModel> = if link_client {
            db.query(live_link_ql)
                .bind(TableName::new(msg_name))
                .await?
                .stream(1)?
                .into()
        } else {
            db.select(msg_name).live().await?.into()
        };

        let db_routine = db.clone();
        let shutdown_trigger = CancellationToken::new();
        let shutdown_waiter = shutdown_trigger.clone();
        let (wait_send, wait_recv) = oneshot::channel();
        let (ob_requester, mut ob_responder) =
            req_res::<QueryHistory, Result<Observer, StopError>>();
        let routine = tokio::spawn(async move {
            let (br_send, _) = broadcast::channel(65536);
            let mut last_key = None;

            match client_stream.next().await {
                None => return Err(StopError::StreamClosed),
                Some(res) => handle_item(res?, &mut last_key, &br_send)?,
            }

            match close_stream.next().await {
                None => return Err(StopError::StreamClosed),
                Some(res) => match res? {
                    Either::Left(item) => handle_item(item, &mut last_key, &br_send)?,
                    Either::Right(item) => handle_item(item, &mut last_key, &br_send)?,
                },
            }

            match msg_stream.next().await {
                None => return Err(StopError::StreamClosed),
                Some(res) => match res? {
                    Either::Left(item) => handle_item(item, &mut last_key, &br_send)?,
                    Either::Right(item) => handle_item(item, &mut last_key, &br_send)?,
                },
            }

            // todo
            async fn build_observer<C: Connection>(
                db: Surreal<C>,
                history: QueryHistory,
                link_client: bool,
                br_send: &broadcast::Sender<ObserveMsg>,
            ) -> Result<Observer, StopError> {
                match (history, link_client) {
                    (QueryHistory::Full, false) => (),
                    (QueryHistory::Full, true) => (),
                    (QueryHistory::Limit(n), false) => (),
                    (QueryHistory::Limit(n), true) => (),
                    _ => (),
                }
                todo!()
            }

            match ob_responder.next_requset().await {
                None => return Err(StopError::RequesterDropped),
                Some(req) => {
                    let res = build_observer(db_routine, req.req(), link_client, &br_send).await;
                    req.response(res).ok();
                }
            }

            wait_send.send(()).ok();
            Ok(GraceType::CtrlC)
        });

        wait_recv.await.ok();

        let session_id = rid.unwrap().id;
        let client_name = self.host;
        let client_role = Role::host();
        let msg_format = None;
        let observer_options = None;
        let client_addr = None;
        let query_map = None;
        let proc_env = ProcEnv::create_async().await;

        match Stop::hello_internal(
            &db,
            &id_gen,
            &session_id,
            &formatted_timestamp,
            &client_name,
            client_role,
            msg_format,
            observer_options,
            client_addr,
            &query_map,
            &proc_env,
            link_client,
            &ob_requester,
        )
        .await
        {
            Err(err) => {
                shutdown_trigger.cancel();
                routine.await.ok();
                Err(err.into())
            }
            Ok(stop) => Ok((
                stop,
                ObserveRoutine {
                    shutdown_trigger,
                    routine,
                },
            )),
        }
    }
}

impl<C: Connection> Stop<C> {
    pub fn builder_default(db: Surreal<C>, app: &str) -> StopBuilder<C> {
        StopBuilder {
            db,
            app: app.into(),
            host: "host".into(),
            link_client: true,
            ctrlc_shutdown: true,
        }
    }

    pub async fn client_hello(
        &self,
        client_role: ClientRole,
        client_hello: HelloMsg,
        client_addr: SocketAddr,
        msg_format: MsgFormat,
        query_history: Option<QueryHistory>,
        query_map: Option<IndexMap<String, String>>,
    ) -> Result<Self, StopError> {
        if client_role.can_observe() && query_history.is_none() {
            return Err(StopError::MustFillQueryHistory);
        }

        Ok(Self::hello_internal(
            &self.db,
            &self.id_gen,
            &self.session_id,
            &self.formatted_timestamp,
            &client_hello.client_name,
            client_role.into(),
            Some(msg_format),
            query_history,
            Some(client_addr),
            &query_map,
            &client_hello.proc_env,
            self.link_client,
            &self.ob_requester,
        )
        .await?)
    }

    async fn hello_internal(
        db: &Surreal<C>,
        id_gen: &IdGen,
        session_id: &RecordId,
        formatted_timestamp: &str,
        client_name: &str,
        client_role: Role,
        msg_format: Option<MsgFormat>,
        query_history: Option<QueryHistory>,
        client_addr: Option<SocketAddr>,
        query_map: &Option<IndexMap<String, String>>,
        proc_env: &Option<ProcEnv>,
        link_client: bool,
        ob_requester: &ObserverRequester,
    ) -> surrealdb::Result<Self> {
        #[derive(Serialize)]
        struct ClientRecord {
            a_timestamp: DateTime<Local>,
            b_session_id: RecordId,
            c_client_name: String,
            d_client_role: Role,
            e_msg_format: Option<MsgFormat>,
            f_query_history: Option<QueryHistory>,
            g_client_addr: Option<SocketAddr>,
            h_query_map: Option<IndexMap<String, String>>,
            i_proc_env: Option<Value>,
        }

        let a_timestamp = Local::now();
        let b_session_id = session_id.clone();
        let c_client_name = client_name.into();
        let d_client_role = client_role;
        let e_msg_format = msg_format;
        let f_query_history = query_history;
        let g_client_addr = client_addr;
        let h_query_map = query_map.clone();
        let i_proc_env = proc_env.as_ref().and_then(|v| serde_json::to_value(v).ok());
        let record = ClientRecord {
            a_timestamp,
            b_session_id,
            c_client_name,
            d_client_role,
            e_msg_format,
            f_query_history,
            g_client_addr,
            h_query_map,
            i_proc_env,
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
        let is_client = client_role.is_client();
        let ob_requester = ob_requester.clone();

        Ok(Self {
            db,
            id_gen,
            session_id,
            formatted_timestamp,
            client_id,
            can_push,
            is_client,
            link_client,
            ob_requester,
        })
    }

    pub async fn query_last_n(&self, n: u8) -> Result<Vec<(String, String)>, surrealdb::Error> {
        #[derive(Deserialize)]
        struct ClientInfo {
            c_client_name: String,
        }

        #[derive(Deserialize)]
        struct Msg {
            message: String,
            client_id: ClientInfo,
        }

        let table_name = format!("{}-msg", self.formatted_timestamp);
        let msgs: Vec<Msg> = self
            .db
            .run("fn::last_n_desc_client")
            .args((table_name, n))
            .await?;

        // clients + disconnects & merge

        Ok(msgs
            .iter()
            .rev()
            .map(|m| (m.message.clone(), m.client_id.c_client_name.clone()))
            .collect())
    }

    pub async fn print(&self) {
        println!("{}", self.is_client);
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

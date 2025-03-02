use crate::{
    stop::Stop,
    tracing_msg::{query_map::QueryMap, ClientRole, GraceType, MsgFormat, QueryHistory},
};
use est::task::CloseAndWait;
use indexmap::IndexMap;
use std::{
    future::Future,
    io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use surrealdb::Connection;
use thiserror::Error;
use tokio::{
    net::{lookup_host, TcpListener, ToSocketAddrs},
    signal::ctrl_c,
    sync::oneshot,
    task::{JoinError, JoinHandle},
};
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        handshake::server::{ErrorResponse, Request, Response},
        http::StatusCode,
    },
};
use tokio_util::{sync::CancellationToken, task::TaskTracker, time::FutureExt};

#[derive(Debug, Clone)]
struct AuthArgs {
    pusher_path: String,
    pusher_token: Option<String>,
    observer_path: String,
    observer_token: Option<String>,
    director_path: String,
    director_token: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct FormatArgs {
    accept_json: bool,
    accept_bincode: bool,
    accept_msgpack: bool,
}

impl FormatArgs {
    fn no_msg_format(&self) -> bool {
        !self.accept_json && !self.accept_bincode && !self.accept_msgpack
    }

    fn allowed(&self, msg_format: MsgFormat) -> bool {
        match msg_format {
            MsgFormat::Json => self.accept_json,
            MsgFormat::Bincode => self.accept_bincode,
            MsgFormat::Msgpack => self.accept_msgpack,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ServerBuilder<C: Connection> {
    stop: Stop<C>,
    auth_args: AuthArgs,
    format_args: FormatArgs,
    fuck_off_on_damage: bool,
    fuck_off_on_observer_push: bool,
    ctrlc_shutdown: bool,
    ws_handshake_timeout: Duration,
    tmp_hello_timeout: Duration,
    bind_addrs: Vec<SocketAddr>,
}

#[derive(Error, Debug)]
pub enum StartError {
    #[error("no message format available")]
    NoMsgFormat,
    #[error("io error: `{0}`")]
    Io(#[from] io::Error),
}

impl<C: Connection + Clone> ServerBuilder<C> {
    pub fn pusher_path(self, path: &str) -> Self {
        Self {
            auth_args: AuthArgs {
                pusher_path: path.into(),
                ..self.auth_args
            },
            ..self
        }
    }

    pub fn pusher_token(self, token: &str) -> Self {
        Self {
            auth_args: AuthArgs {
                pusher_token: Some(token.into()),
                ..self.auth_args
            },
            ..self
        }
    }

    pub fn observer_path(self, path: &str) -> Self {
        Self {
            auth_args: AuthArgs {
                observer_path: path.into(),
                ..self.auth_args
            },
            ..self
        }
    }

    pub fn observer_token(self, token: &str) -> Self {
        Self {
            auth_args: AuthArgs {
                observer_token: Some(token.into()),
                ..self.auth_args
            },
            ..self
        }
    }

    pub fn director_path(self, path: &str) -> Self {
        Self {
            auth_args: AuthArgs {
                director_path: path.into(),
                ..self.auth_args
            },
            ..self
        }
    }

    pub fn director_token(self, token: &str) -> Self {
        Self {
            auth_args: AuthArgs {
                director_token: Some(token.into()),
                ..self.auth_args
            },
            ..self
        }
    }

    pub fn default_token(self, token: &str) -> Self {
        let optb = Some(token.into());
        Self {
            auth_args: AuthArgs {
                pusher_token: self.auth_args.pusher_token.or(optb.clone()),
                observer_token: self.auth_args.observer_token.or(optb.clone()),
                director_token: self.auth_args.director_token.or(optb),
                ..self.auth_args
            },
            ..self
        }
    }

    pub fn disable_json(self) -> Self {
        Self {
            format_args: FormatArgs {
                accept_json: false,
                ..self.format_args
            },
            ..self
        }
    }

    pub fn disable_bincode(self) -> Self {
        Self {
            format_args: FormatArgs {
                accept_bincode: false,
                ..self.format_args
            },
            ..self
        }
    }

    pub fn disable_msgpack(self) -> Self {
        Self {
            format_args: FormatArgs {
                accept_msgpack: false,
                ..self.format_args
            },
            ..self
        }
    }

    pub fn fuck_off_on_damage(self) -> Self {
        Self {
            fuck_off_on_damage: true,
            ..self
        }
    }

    pub fn fuck_off_on_observer_push(self) -> Self {
        Self {
            fuck_off_on_observer_push: true,
            ..self
        }
    }

    pub fn disable_ctrlc_shutdown(self) -> Self {
        Self {
            ctrlc_shutdown: false,
            ..self
        }
    }

    pub fn ws_handshake_timeout(self, timeout: Duration) -> Self {
        Self {
            ws_handshake_timeout: timeout,
            ..self
        }
    }

    pub fn tmp_hello_timeout(self, timeout: Duration) -> Self {
        Self {
            tmp_hello_timeout: timeout,
            ..self
        }
    }

    pub async fn bind_addrs<A: ToSocketAddrs>(self, host: A) -> io::Result<Self> {
        Ok(Self {
            bind_addrs: lookup_host(host).await?.collect(),
            ..self
        })
    }

    pub async fn start(self) -> Result<ServerHandle, StartError> {
        if self.format_args.no_msg_format() {
            return Err(StartError::NoMsgFormat);
        }

        let listener = TcpListener::bind(self.bind_addrs.as_slice()).await?;
        let builder = self;
        let local_addr = listener.local_addr().unwrap();
        let shutdown_trigger = CancellationToken::new();
        let shutdown_waiter = shutdown_trigger.clone();
        let routine = tokio::spawn(async move {
            builder.stop.print().await;
            println!("{}", builder.fuck_off_on_damage);
            println!("{}", builder.fuck_off_on_observer_push);
            println!("{:?}", builder.tmp_hello_timeout);
            // log safe builder info + local_addr into db

            let tracker = TaskTracker::new();

            loop {
                let (stream, client_addr) = tokio::select! {
                    res = ctrl_c(), if builder.ctrlc_shutdown => {
                        println!("Bye from ctrl_c");
                        shutdown_waiter.cancel();
                        tracker.close_and_wait().await;
                        return res.map(|_| GraceType::CtrlC);
                    }
                    _ = shutdown_waiter.cancelled() => {
                        println!("Bye from shutdown_waiter");
                        tracker.close_and_wait().await;
                        return Ok(GraceType::Explicit);
                    }
                    res = listener.accept() => {
                        if let Err(err) = &res {
                            println!("accept err: {}", err);
                        }

                        res?
                    }
                };

                println!("{:?}", stream);
                println!("{}", client_addr);

                let builder = builder.clone();
                let auth_args = builder.auth_args.clone();
                let format_args = builder.format_args;
                let shutdown_waiter = shutdown_waiter.clone();
                let (role_send, role_recv) = oneshot::channel();
                let (map_send, map_recv) = oneshot::channel();
                let (fmt_send, fmt_recv) = oneshot::channel();
                let (qh_send, qh_recv) = oneshot::channel();
                tracker.spawn(async move {
                    let (stream,
                        client_role,
                        msg_format,
                        query_history,
                        query_map
                    ) = tokio::select! {
                        _ = shutdown_waiter.cancelled() => {
                            println!("shutdown_waiter.cancelled()");
                            return;
                        }
                        res = accept_hdr_async(stream, move |req: &Request, resp| {
                            let uri = req.uri();
                            let path = uri.path();
                            let query: Option<Result<IndexMap<String, String>, serde_qs::Error>> =
                                uri.query().map(serde_qs::from_str);

                            println!("path: {}", path);
                            println!("query: {:?}", query);

                            let msg_format = match &query {
                                Some(Ok(map)) => {
                                    map_send.send(map.clone()).ok();
                                    map.get_msg_format()
                                }
                                _ => Default::default(),
                            };

                            match format_args.allowed(msg_format) {
                                true => fmt_send.send(msg_format).ok(),
                                false => {
                                    return Err(err_resp(
                                        &format!("{:?} not allowed!", msg_format),
                                        StatusCode::FORBIDDEN
                                    ));
                                },
                            };

                            if path == auth_args.pusher_path {
                                return query_auth(
                                    query,
                                    auth_args.pusher_token,
                                    qh_send,
                                    role_send,
                                    ClientRole::Pusher,
                                    resp,
                                );
                            }

                            if path == auth_args.observer_path {
                                return query_auth(
                                    query,
                                    auth_args.observer_token,
                                    qh_send,
                                    role_send,
                                    ClientRole::Observer,
                                    resp,
                                );
                            }

                            if path == auth_args.director_path {
                                return query_auth(
                                    query,
                                    auth_args.director_token,
                                    qh_send,
                                    role_send,
                                    ClientRole::Director,
                                    resp,
                                );
                            }

                            Err(err_resp("invalid path!", StatusCode::NOT_FOUND))
                        })
                        .timeout(builder.ws_handshake_timeout) => match res {
                            Err(err) => {
                                println!("outer_err: {}", err);
                                return;
                            }
                            Ok(Err(err)) => {
                                println!("inner_err: {}", err);
                                return;
                            }
                            Ok(Ok(stream)) => {
                                (
                                    stream,
                                    role_recv.await.unwrap(),
                                    fmt_recv.await.unwrap(),
                                    qh_recv.await.ok(),
                                    map_recv.await.ok()
                                )
                            }
                        }
                    };

                    println!("inner_stream: {:?}", stream);
                    println!("client_role: {:?}", client_role);
                    println!("client_addr: {}", client_addr);
                    println!("msg_format: {:?}", msg_format);
                    println!("query_history: {:?}", query_history);
                    println!("query_map: {:?}", query_map);

                    // todo: HelloMsg + Stop::client_hello + 传回Observer::link_client
                });
            }
        });

        Ok(ServerHandle {
            local_addr,
            shutdown_trigger,
            routine,
        })
    }
}

pub trait BuildServerDefault<C: Connection> {
    fn build_server_default(&self) -> ServerBuilder<C>;
}

impl<C: Connection + Clone> BuildServerDefault<C> for Stop<C> {
    fn build_server_default(&self) -> ServerBuilder<C> {
        ServerBuilder {
            stop: self.clone(),
            auth_args: AuthArgs {
                pusher_path: "/pusher".into(),
                pusher_token: None,
                observer_path: "/observer".into(),
                observer_token: None,
                director_path: "/director".into(),
                director_token: None,
            },
            format_args: FormatArgs {
                accept_json: true,
                accept_bincode: true,
                accept_msgpack: true,
            },
            fuck_off_on_damage: false,
            fuck_off_on_observer_push: false,
            ctrlc_shutdown: true,
            ws_handshake_timeout: Duration::from_secs_f64(1.5),
            tmp_hello_timeout: Duration::from_secs_f64(3.0),
            bind_addrs: vec![SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8192).into()],
        }
    }
}

fn query_auth(
    query: Option<Result<IndexMap<String, String>, serde_qs::Error>>,
    token_need: Option<String>,
    qh_send: oneshot::Sender<QueryHistory>,
    role_send: oneshot::Sender<ClientRole>,
    role: ClientRole,
    resp: Response,
) -> Result<Response, ErrorResponse> {
    if role.can_observe() {
        let qh = match &query {
            Some(Ok(map)) => map.get_query_history(),
            _ => Default::default(),
        };
        qh_send.send(qh).ok();
    }

    if let Some(token_need) = token_need {
        match query {
            None => {
                return Err(err_resp("need query!", StatusCode::BAD_REQUEST));
            }
            Some(Err(err)) => {
                // e.g.: "ws://127.0.0.1:8192/pusher?=&=&=&"
                return Err(err_resp(
                    &format!("query err: {}", err),
                    StatusCode::BAD_REQUEST,
                ));
            }
            Some(Ok(map)) => {
                println!("{:?}", map);

                match map.get_token() {
                    None => {
                        return Err(err_resp("need token!", StatusCode::BAD_REQUEST));
                    }
                    Some(token_req) => {
                        println!("token_req: {}", token_req);

                        if *token_req != token_need {
                            return Err(err_resp("wrong token!", StatusCode::FORBIDDEN));
                        }
                    }
                }
            }
        }
    }

    role_send.send(role).ok();
    return Ok(resp);
}

fn err_resp(text: &str, status: StatusCode) -> ErrorResponse {
    println!("{}", text);
    println!("{}", status);

    let mut resp = ErrorResponse::new(Some(text.into()));
    *resp.status_mut() = status;
    resp
}

type RoutineOutput = Result<GraceType, io::Error>;
type HandleOutput = Result<RoutineOutput, JoinError>;

#[derive(Debug)]
pub struct ServerHandle {
    local_addr: SocketAddr,
    shutdown_trigger: CancellationToken,
    routine: JoinHandle<RoutineOutput>,
}

impl ServerHandle {
    pub fn get_local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn trigger_graceful_shutdown(&self) {
        self.shutdown_trigger.cancel();
    }

    pub async fn graceful_shutdown(self) -> HandleOutput {
        self.trigger_graceful_shutdown();
        self.await
    }
}

impl Future for ServerHandle {
    type Output = HandleOutput;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.routine).poll(cx)
    }
}

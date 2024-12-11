use crate::stop::Stop;
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

#[derive(Debug, Copy, Clone)]
enum SendFormat {
    Json,
    Bincode,
    Msgpack,
}

#[derive(Clone, Debug)]
pub struct ServerBuilder<C: Connection> {
    stop: Stop<C>,
    pusher_path: String,
    pusher_token: Option<String>,
    observer_path: String,
    observer_token: Option<String>,
    director_path: String,
    director_token: Option<String>,
    recv_json: bool,
    recv_bincode: Option<bool>,
    fuck_off_on_damage: bool,
    send_format: SendFormat,
    ctrlc_shutdown: bool,
    handshake_timeout: Duration,
    bind_addrs: Vec<SocketAddr>,
}

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("no receivable format available")]
    NoRecvFormat,
    #[error("io error: `{0}`")]
    Io(#[from] io::Error),
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum GracefulType {
    CtrlC,
    Explicit,
}

impl<C: Connection + Clone> ServerBuilder<C> {
    pub fn from_stop_default(stop: &Stop<C>) -> Self {
        Self {
            stop: stop.clone(),
            pusher_path: "/pusher".into(),
            pusher_token: None,
            observer_path: "/observer".into(),
            observer_token: None,
            director_path: "/director".into(),
            director_token: None,
            recv_json: true,
            recv_bincode: Some(true),
            fuck_off_on_damage: false,
            send_format: SendFormat::Bincode,
            ctrlc_shutdown: true,
            handshake_timeout: Duration::from_secs_f64(3.0),
            bind_addrs: vec![SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8192).into()],
        }
    }

    pub fn pusher_path(self, path: &str) -> Self {
        Self {
            pusher_path: path.into(),
            ..self
        }
    }

    pub fn pusher_token(self, token: &str) -> Self {
        Self {
            pusher_token: Some(token.into()),
            ..self
        }
    }

    pub fn observer_path(self, path: &str) -> Self {
        Self {
            observer_path: path.into(),
            ..self
        }
    }

    pub fn observer_token(self, token: &str) -> Self {
        Self {
            observer_token: Some(token.into()),
            ..self
        }
    }

    pub fn director_path(self, path: &str) -> Self {
        Self {
            director_path: path.into(),
            ..self
        }
    }

    pub fn director_token(self, token: &str) -> Self {
        Self {
            director_token: Some(token.into()),
            ..self
        }
    }

    pub fn default_token(self, token: &str) -> Self {
        let optb = Some(token.into());
        Self {
            pusher_token: self.pusher_token.or(optb.clone()),
            observer_token: self.observer_token.or(optb.clone()),
            director_token: self.director_token.or(optb),
            ..self
        }
    }

    pub fn disable_recv_json(self) -> Self {
        Self {
            recv_json: false,
            ..self
        }
    }

    pub fn disable_recv_binary(self) -> Self {
        Self {
            recv_bincode: None,
            ..self
        }
    }

    pub fn binary_recv_msgpack(self) -> Self {
        Self {
            recv_bincode: Some(false),
            ..self
        }
    }

    pub fn fuck_off_on_damage(self) -> Self {
        Self {
            fuck_off_on_damage: true,
            ..self
        }
    }

    pub fn send_json(self) -> Self {
        Self {
            send_format: SendFormat::Json,
            ..self
        }
    }

    pub fn send_msgpack(self) -> Self {
        Self {
            send_format: SendFormat::Msgpack,
            ..self
        }
    }

    pub fn disable_ctrlc_shutdown(self) -> Self {
        Self {
            ctrlc_shutdown: false,
            ..self
        }
    }

    pub fn handshake_timeout(self, timeout: Duration) -> Self {
        Self {
            handshake_timeout: timeout,
            ..self
        }
    }

    pub async fn bind_addrs<A: ToSocketAddrs>(self, host: A) -> io::Result<Self> {
        Ok(Self {
            bind_addrs: lookup_host(host).await?.collect(),
            ..self
        })
    }

    pub async fn start(self) -> Result<ServerHandle, ServerError> {
        if !self.recv_json && self.recv_bincode.is_none() {
            return Err(ServerError::NoRecvFormat);
        }

        let listener = TcpListener::bind(self.bind_addrs.as_slice()).await?;
        let builder = self;
        let local_addr = listener.local_addr().unwrap();
        let (shutdown_s, mut shutdown_r) = oneshot::channel();
        let routine = tokio::spawn(async move {
            builder.stop.print().await;
            println!("{}", builder.pusher_path);
            println!("{}", builder.observer_path);
            println!("{}", builder.director_path);
            println!("{}", builder.fuck_off_on_damage);
            println!("{:?}", builder.send_format);
            println!("{:?}", builder.handshake_timeout);

            loop {
                let (stream, client) = tokio::select! {
                    res = ctrl_c(), if builder.ctrlc_shutdown => {
                        println!("Bye from ctrl_c");
                        return res.map(|_| GracefulType::CtrlC);
                    }
                    _ = &mut shutdown_r => {
                        println!("Bye from shutdown_r");
                        return Ok(GracefulType::Explicit);
                    }
                    res = listener.accept() => {
                        res?
                    }
                };

                println!("{:?}", stream);
                println!("{}", client);
            }
        });

        Ok(ServerHandle {
            local_addr,
            shutdown_s,
            routine,
        })
    }
}

type RoutineOutput = io::Result<GracefulType>;
type ServerOutput = Result<RoutineOutput, JoinError>;

#[derive(Debug)]
pub struct ServerHandle {
    local_addr: SocketAddr,
    shutdown_s: oneshot::Sender<()>,
    routine: JoinHandle<RoutineOutput>,
}

impl ServerHandle {
    pub fn get_local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub async fn graceful_shutdown(self) -> ServerOutput {
        self.shutdown_s.send(()).ok();
        self.routine.await
    }
}

impl Future for ServerHandle {
    type Output = ServerOutput;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.routine).poll(cx)
    }
}

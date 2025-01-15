use std::ops::{Deref, DerefMut};
use thiserror::Error;
use tokio::sync::{
    mpsc::{error::SendError, unbounded_channel, UnboundedReceiver, UnboundedSender},
    oneshot,
};

#[derive(Debug)]
pub struct Request<Q, S> {
    req: Q,
    res_send: oneshot::Sender<S>,
}

impl<Q, S> Deref for Request<Q, S> {
    type Target = Q;

    fn deref(&self) -> &Self::Target {
        &self.req
    }
}

impl<Q, S> DerefMut for Request<Q, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.req
    }
}

impl<Q, S> Request<Q, S> {
    pub fn req(&self) -> Q
    where
        Q: Copy,
    {
        self.req
    }

    pub fn req_cloned(&self) -> Q
    where
        Q: Clone,
    {
        self.req.clone()
    }

    pub fn response(self, res: S) -> Result<(), S> {
        self.res_send.send(res)
    }
}

#[derive(Error, Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum RequestError {
    #[error("send request error: Respondor closed or dropped")]
    SendReqErr,
    #[error("recv response error: Request handle dropped without sending response")]
    RecvResErr,
}

impl<T> From<SendError<T>> for RequestError {
    fn from(_value: SendError<T>) -> Self {
        Self::SendReqErr
    }
}

#[derive(Debug)]
pub struct Requester<Q, S>(UnboundedSender<Request<Q, S>>);

impl<Q, S> Requester<Q, S> {
    pub async fn request(&self, req: Q) -> Result<S, RequestError> {
        let (res_send, res_recv) = oneshot::channel();
        self.0.send(Request { req, res_send })?;
        res_recv.await.map_err(|_| RequestError::RecvResErr)
    }
}

impl<Q, S> Clone for Requester<Q, S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[derive(Debug)]
pub struct Respondor<Q, S>(UnboundedReceiver<Request<Q, S>>);

impl<Q, S> Respondor<Q, S> {
    pub async fn next_requset(&mut self) -> Option<Request<Q, S>> {
        self.0.recv().await
    }

    pub fn close(&mut self) {
        self.0.close();
    }
}

pub fn req_res<Q, S>() -> (Requester<Q, S>, Respondor<Q, S>) {
    let (req_send, req_recv) = unbounded_channel();
    (Requester(req_send), Respondor(req_recv))
}

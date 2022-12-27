pub use crate::enums::*;
pub use crate::errors::*;
pub use crate::py::*;
pub use crate::scope::*;
pub use anyhow::Result;
pub use async_channel::{unbounded, Receiver as AsyncReceiver, Sender as AsyncSender};
pub use futures::Future;
pub use hyper::http::HeaderValue;
pub use hyper::Body;
pub use hyper::Response;
pub use hyper_tungstenite::{tungstenite, HyperWebsocket};
pub use log::{debug, error, info, warn};
pub use pyo3::prelude::*;
pub use pyo3::types::*;
pub use pyo3_asyncio::TaskLocals;
pub use std::net::SocketAddr;
pub use std::pin::Pin;
pub use tokio::sync::mpsc::{
    channel as bounded_channel, unbounded_channel, Receiver as BoundedReceiver,
    Sender as BoundedSender, UnboundedReceiver, UnboundedSender,
};
pub use tokio::sync::oneshot::{channel, Receiver, Sender};
pub type FutureResponse =
    Pin<Box<dyn Future<Output = Result<Response<Body>, hyper::Error>> + Send>>;
pub type Headers = Vec<Vec<Vec<u8>>>;

#[macro_export]
macro_rules! server_header {
    () => {
        HeaderValue::from_str(
            format!("{}:{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")).as_str(),
        )
        .unwrap()
    };
}

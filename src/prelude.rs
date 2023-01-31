pub use crate::body::*;
pub use crate::config::*;
pub use crate::config::*;
pub use crate::enums::*;
pub use crate::errors::*;
pub use crate::py_future::*;
pub use crate::scope::*;
pub use crate::utils::*;
pub use anyhow::Result;
pub use futures::Future;
pub use hyper::body::Bytes;
pub use hyper::body::Incoming as IncomingBody;
pub use hyper::http::HeaderValue;
pub use hyper::Response;
pub use log::{debug, error, info, warn, trace};
pub use pyo3::prelude::*;
pub use pyo3::types::*;
pub use std::net::SocketAddr;
pub use std::pin::Pin;
pub type Headers = Vec<Vec<Vec<u8>>>;
pub type ServiceResponse = Response<HttpBody<Bytes>>;
pub type FutureResponse =
    Pin<Box<dyn Future<Output = Result<ServiceResponse, hyper::http::Error>> + Send>>;
pub type ScopeRecvSend = Option<(PyObject, PyObject, PyObject)>;

#[macro_export]
macro_rules! server_header {
    () => {
        HeaderValue::from_str(
            format!("{}:{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")).as_str(),
        )
        .unwrap()
    };
}

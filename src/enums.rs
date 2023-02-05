use pyo3::exceptions::PyTypeError;
use serde::Deserialize;
use std::fmt::Display;
use std::str::FromStr;
use std::string::ToString;

use crate::prelude::*;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
pub enum HttpVersion {
    HTTP1,
    HTTP2,
}

impl FromStr for HttpVersion {
    type Err = ();

    fn from_str(input: &str) -> Result<HttpVersion, Self::Err> {
        match input {
            "http1" => Ok(HttpVersion::HTTP1),
            "http2" => Ok(HttpVersion::HTTP2),
            _ => Err(()),
        }
    }
}

impl From<Option<String>> for HttpVersion {
    fn from(value: Option<String>) -> Self {
        if let Some(v) = value {
            return HttpVersion::from_str(v.as_str()).unwrap_or(Self::default());
        }
        Self::default()
    }
}

impl Default for HttpVersion {
    fn default() -> Self {
        HttpVersion::HTTP1
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScopeType {
    Http,
    HttpDisconect,
    HttpRequest,
    HttpResponseStart,
    HttpResponseBody,
    Ws,
    WsDisconenct,
    WsConnect,
    WsReceive,
    WsAccept,
    WsClose,
    WsSend,
}

impl IntoPy<PyObject> for ScopeType {
    fn into_py(self, py: pyo3::Python<'_>) -> PyObject {
        self.to_string().into_py(py)
    }
}

impl<'source> FromPyObject<'source> for ScopeType {
    fn extract(ob: &'source PyAny) -> PyResult<Self> {
        match ob.to_string().as_str() {
            "http" => Ok(ScopeType::Http),
            "http.request" => Ok(ScopeType::HttpRequest),
            "http.disconnect" => Ok(ScopeType::HttpDisconect),
            "http.response.body" => Ok(ScopeType::HttpResponseBody),
            "http.response.start" => Ok(ScopeType::HttpResponseStart),
            "websocket" => Ok(ScopeType::Ws),
            "websocket.connect" => Ok(ScopeType::WsConnect),
            "websocket.disconnect" => Ok(ScopeType::WsDisconenct),
            "websocket.receive" => Ok(ScopeType::WsReceive),
            "websocket.accept" => Ok(ScopeType::WsAccept),
            "websocket.close" => Ok(ScopeType::WsClose),
            "websocket.send" => Ok(ScopeType::WsSend),
            _ => Err(PyTypeError::new_err("unknown type")),
        }
    }
}

impl Display for ScopeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScopeType::Http => write!(f, "http"),
            ScopeType::HttpRequest => write!(f, "http.request"),
            ScopeType::HttpDisconect => write!(f, "http.disconnect"),
            ScopeType::HttpResponseBody => write!(f, "http.response.body"),
            ScopeType::HttpResponseStart => write!(f, "http.response.start"),
            ScopeType::Ws => write!(f, "websocket"),
            ScopeType::WsConnect => write!(f, "websocket.connect"),
            ScopeType::WsDisconenct => write!(f, "websocket.disconnect"),
            ScopeType::WsReceive => write!(f, "websocket.receive"),
            ScopeType::WsAccept => write!(f, "websocket.accept"),
            ScopeType::WsClose => write!(f, "websocket.close"),
            ScopeType::WsSend => write!(f, "websocket.send"),
        }
    }
}

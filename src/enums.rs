use pyo3::types::PyString;
use pyo3::{IntoPy, Py};
use std::str::FromStr;
use std::string::ToString;

#[derive(Debug, Clone)]
pub enum HttpVersion {
    Any,
    HTTP1,
    HTTP2,
}

impl FromStr for HttpVersion {
    type Err = ();

    fn from_str(input: &str) -> Result<HttpVersion, Self::Err> {
        match input {
            "any" => Ok(HttpVersion::Any),
            "http1" => Ok(HttpVersion::HTTP1),
            "http2" => Ok(HttpVersion::HTTP2),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScopeType {
    Http,
    Ws,
}

impl ToString for ScopeType {
    fn to_string(&self) -> String {
        match self {
            ScopeType::Http => "http".to_string(),
            ScopeType::Ws => "websocket".to_string(),
        }
    }
}

impl IntoPy<Py<PyString>> for ScopeType {
    fn into_py(self, py: pyo3::Python<'_>) -> Py<PyString> {
        match self {
            ScopeType::Http => "http".into_py(py),
            ScopeType::Ws => "websocket".into_py(py),
        }
    }
}

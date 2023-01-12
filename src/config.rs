use serde::Deserialize;

use crate::prelude::*;

fn default_bind() -> String {
    "127.0.0.1:8000".to_string()
}

fn default_ping_interval() -> f32 {
    20.0
}

fn default_ping_timeout() -> f32 {
    20.0
}

#[pyclass]
#[derive(Deserialize, Clone)]
pub struct Config {
    pub application: String,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default)]
    pub http_version: HttpVersion,
    #[serde(default)]
    pub tls: Option<Tls>,

    #[serde(default)]
    pub limit: Option<Limit>,

    pub ws: WebSocket,
}

#[derive(Deserialize, Clone)]
pub struct Tls {
    pub cert_path: String,
    pub private_path: String,
}

#[derive(Deserialize, Clone)]
pub struct Limit {
    #[serde(default)]
    pub concurrency: Option<u32>,
    #[serde(default)]
    pub python_execution: Option<u32>,
}

#[derive(Deserialize, Clone)]
pub struct WebSocket {
    #[serde(default = "default_ping_interval")]
    pub ping_interval: f32,
    #[serde(default = "default_ping_timeout")]
    pub ping_timeout: f32,
}

#[pymethods]
impl Config {
    #[new]
    fn new(
        application: String,
        bind: String,
        http_version: Option<String>,
        cert_path: Option<String>,
        private_path: Option<String>,
        limit_concurrency: Option<u32>,
        limit_python_execution: Option<u32>,
        ws_ping_interval: Option<f32>,
        ws_ping_timeout: Option<f32>,
    ) -> Self {
        let http_version = HttpVersion::from(http_version);
        let mut tls = None;
        if cert_path.is_some() && private_path.is_some() {
            tls = Some(Tls {
                cert_path: cert_path.unwrap(),
                private_path: private_path.unwrap(),
            });
        }
        let limit = Some(Limit {
            concurrency: limit_concurrency,
            python_execution: limit_python_execution,
        });
        let ws = WebSocket {
            ping_timeout: ws_ping_timeout.unwrap_or_else(default_ping_timeout),
            ping_interval: ws_ping_interval.unwrap_or_else(default_ping_interval),
        };

        Self {
            application,
            bind,
            http_version,
            tls,
            limit,
            ws,
        }
    }
}

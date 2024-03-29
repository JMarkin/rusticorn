use crate::prelude::*;
use hyper::server::conn::{http1, http2};
use hyper_tungstenite::is_upgrade_request;
use std::fs;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::mpsc::{Receiver, Sender, UnboundedReceiver, UnboundedSender};
use tokio::sync::Mutex;
use tokio_rustls::rustls::{Certificate, PrivateKey, ServerConfig};
use tokio_rustls::TlsAcceptor;

use hyper::service::Service;
use hyper::Request;
use std::pin::Pin;

#[derive(Clone, Copy, Debug)]
struct LocalExec;

impl<F> hyper::rt::Executor<F> for LocalExec
where
    F: std::future::Future + 'static,
{
    fn execute(&self, fut: F) {
        tokio::task::spawn_local(fut);
    }
}

struct Svc {
    addr: SocketAddr,
    tx: UnboundedSender<ScopeRecvSend>,
    cfg: Config,
}

impl Service<Request<IncomingBody>> for Svc {
    type Response = ServiceResponse;
    type Error = hyper::http::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&mut self, req: Request<IncomingBody>) -> Self::Future {
        let cfg = self.cfg.clone();
        let addr = self.addr;
        let tx = self.tx.clone();

        if is_upgrade_request(&req) {
            crate::protocol::ws::handle(addr, req, tx, cfg.ws.ping_timeout, cfg.ws.ping_interval)
        } else {
            crate::protocol::http::handle(addr, req, tx)
        }
    }
}

macro_rules! spawn_service {
    ($http_version:expr, $stream:expr, $service:expr) => {
        tokio::task::spawn_local(async move {
            if let Err(err) = match $http_version {
                HttpVersion::HTTP1 => {
                    http1::Builder::new()
                        .keep_alive(true)
                        .serve_connection($stream, $service)
                        .with_upgrades()
                        .await
                }
                HttpVersion::HTTP2 => {
                    http2::Builder::new(LocalExec)
                        .keep_alive_interval(Some(Duration::from_millis(100)))
                        .serve_connection($stream, $service)
                        .await
                }
            } {
                error!("Failed to serve connection: {:?}", err);
            }
        })
    };
}

#[pyclass]
pub struct ASGIServer {
    rx: Arc<Mutex<UnboundedReceiver<ScopeRecvSend>>>,
    stop_rx: Arc<Mutex<Receiver<bool>>>,
}

impl ASGIServer {
    pub fn new(rx: UnboundedReceiver<ScopeRecvSend>, stop_rx: Receiver<bool>) -> Self {
        ASGIServer {
            rx: Arc::new(Mutex::new(rx)),
            stop_rx: Arc::new(Mutex::new(stop_rx)),
        }
    }
}

#[pymethods]
impl ASGIServer {
    fn req(&self) -> PyAsync<Option<(PyObject, PyObject, PyObject)>> {
        let rx = self.rx.clone();
        async move {
            let mut lock = rx.lock().await;
            let data = lock.recv().await.unwrap();
            data
        }
        .into()
    }
    fn stop(&self) -> PyResult<bool> {
        let rx = self.stop_rx.clone();
        let is_locked = rx.try_lock();
        match is_locked {
            Ok(mut rx) => {
                let is_stopped = rx.try_recv();
                match is_stopped {
                    Ok(v) => Ok(v),
                    Err(_e) => Ok(false),
                }
            }
            Err(_e) => Ok(false),
        }
    }
}

const HTTP2_ALPN: [u8; 2] = [104, 50];

pub async fn start_server(
    cfg: Config,
    tx: UnboundedSender<ScopeRecvSend>,
    stop_tx: Sender<bool>,
) -> Result<()> {
    let mut acceptor = None;
    if let Some(tls) = cfg.tls.clone() {
        acceptor = Some({
            let cert_pem = fs::read(tls.cert_path.as_str())?;
            let key_pem = fs::read(tls.private_path.as_str())?;

            let certs: Vec<Certificate> = rustls_pemfile::certs(&mut &*cert_pem)
                .map(|mut certs| certs.drain(..).map(Certificate).collect())?;
            if certs.is_empty() {
                return Err(anyhow!("No certificates found."));
            }

            let mut keys: Vec<PrivateKey> = rustls_pemfile::pkcs8_private_keys(&mut &*key_pem)
                .map(|mut keys| keys.drain(..).map(PrivateKey).collect())?;
            if keys.is_empty() {
                return Err(anyhow!("No private keys found."));
            }

            let mut tls_config = ServerConfig::builder()
                .with_safe_defaults()
                .with_no_client_auth()
                .with_single_cert(certs, keys.remove(0))?;

            if cfg.http_version == HttpVersion::HTTP2 {
                tls_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
            } else {
                tls_config.alpn_protocols = vec![b"http/1.1".to_vec()];
            }
            TlsAcceptor::from(Arc::new(tls_config))
        });
    }

    let addr: SocketAddr = cfg.bind.parse()?;
    let listener = TcpListener::bind(addr).await?;
    info!(
        "Listening on http{}://{}",
        if cfg.tls.is_some() { "s" } else { "" },
        cfg.bind
    );

    let (shutdown_send, mut shutdown_recv) = tokio::sync::mpsc::channel::<bool>(1);

    tokio::task::spawn_local(async move {
        loop {
            let (stream, peer_addr) = listener.accept().await.expect("can't listen");

            let service = Svc {
                addr: peer_addr,
                tx: tx.clone(),
                cfg: cfg.clone(),
            };

            if cfg.tls.is_some() {
                let acceptor = acceptor.clone();
                let stream = acceptor.unwrap().accept(stream).await;
                if stream.is_err() {
                    error!("can't tls accept: {:?}", stream);
                    continue;
                }
                let stream = stream.unwrap();

                let alpn = stream.get_ref().1.deref().alpn_protocol();

                if alpn.is_none() {
                    error!("alpn proto empty");
                    continue;
                }

                if alpn.unwrap() == HTTP2_ALPN {
                    spawn_service!(HttpVersion::HTTP2, stream, service);
                } else {
                    spawn_service!(HttpVersion::HTTP1, stream, service);
                }
            } else {
                spawn_service!(cfg.http_version, stream, service);
            }

            let shutdown = shutdown_recv.try_recv();
            if shutdown.is_ok() {
                break;
            };
        }
    });

    match signal::ctrl_c().await {
        Ok(()) => {
            stop_tx.send(true).await?;
            shutdown_send.send(true).await?
        }
        Err(err) => {
            eprintln!("Unable to listen for shutdown signal: {}", err);
        }
    };

    Ok(())
}

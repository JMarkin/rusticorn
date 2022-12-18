use crate::prelude::*;
use crate::protocol::http::ReceiveRequest;
use crate::protocol::http::ReceiveTypes;
use crate::protocol::http::SendStart;
use crate::protocol::http::SendTypes;
use core::task::{Context, Poll};
use futures::channel::mpsc;
use futures_util::ready;
use hyper::body::HttpBody;
use hyper::server::accept::Accept;
use hyper::server::conn::{AddrIncoming, AddrStream};
use hyper::service::make_service_fn;
use hyper::service::service_fn;
use hyper::Body;
use hyper::Request;
use hyper::Response;
use hyper::Server;
use hyper::StatusCode;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::vec::Vec;
use std::{fs, io, sync};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_rustls::rustls::ServerConfig;

use crate::scope::configure_scope;

#[derive(Debug)]
pub struct ASGIRequest {
    pub send: AsyncSender<SendTypes>,
    pub receive: AsyncSender<AsyncSender<ReceiveTypes>>,
    pub scope: Py<PyAny>,
}

fn response_start<T>(resp: &mut Response<T>, send_start: SendStart) -> Result<()> {
    *resp.status_mut() = StatusCode::from_u16(send_start.status).unwrap();
    for header in send_start.headers {
        let name = header.first().unwrap();
        let value = header.last().unwrap();
        let name = hyper::http::header::HeaderName::from_bytes(name.as_slice())?;
        let value = hyper::http::header::HeaderValue::from_bytes(value.as_slice())?;

        resp.headers_mut().insert(name, value);
    }
    Ok(())
}

async fn call(
    req_tx: Arc<AsyncSender<ASGIRequest>>,
    addr: SocketAddr,
    req: Request<Body>,
) -> Result<Response<Body>, Infallible> {
    let (body_tx, body_rx) = mpsc::unbounded::<Result<Vec<u8>>>();
    let (send_tx, send_rx) = unbounded::<SendTypes>();
    let (recv_tx, recv_rx) = unbounded::<AsyncSender<ReceiveTypes>>();

    let mut response = Response::builder()
        .body(Body::wrap_stream(body_rx))
        .unwrap();

    let scope = Python::with_gil(|py| configure_scope(py, &req, addr).unwrap().to_object(py));
    req_tx
        .send(ASGIRequest {
            scope,
            send: send_tx,
            receive: recv_tx,
        })
        .await
        .unwrap();

    let wait_parse_request = tokio::spawn(async move {
        let mut chunk_stream = req.into_body();

        while let Ok(tx) = recv_rx.recv().await {
            let chunk = chunk_stream.data().await;
            if let Some(chunk) = chunk {
                match chunk {
                    Err(e) => {
                        warn!("{}", e.to_string());
                        tx.send(ReceiveTypes::HttpDisconect).await.unwrap();
                        break;
                    }
                    Ok(data) => {
                        let more_body = !chunk_stream.is_end_stream();
                        tx.send(ReceiveTypes::HttpRequst(ReceiveRequest {
                            body: data.into(),
                            more_body,
                        }))
                        .await
                        .unwrap();
                    }
                }
            } else {
                break;
            }
        }
    });

    while let Ok(_type) = send_rx.recv().await {
        let result = match _type {
            SendTypes::HttpResponseStart(send_start) => response_start(&mut response, send_start),
            SendTypes::HttpResponseBody(send_body) => {
                body_tx.unbounded_send(Ok(send_body.body)).unwrap();
                if !send_body.more_body {
                    break;
                }
                Ok(())
            }
        };

        if let Err(e) = result {
            response = HttpError::internal_error(e);
            break;
        }
    }

    wait_parse_request.abort();
    Ok(response)
}

pub async fn start_server(
    tx: AsyncSender<ASGIRequest>,
    addr: SocketAddr,
    tls: bool,
    cert_path: Option<String>,
    private_path: Option<String>,
    http_version: HttpVersion,
) -> Result<()> {
    let atx = Arc::new(tx);

    let tls_cfg;
    if tls {
        tls_cfg = {
            // Load public certificate.
            let certs = load_certs(cert_path.unwrap().as_str())?;
            // Load private key.
            let key = load_private_key(private_path.unwrap().as_str())?;
            // Do not use client certificate authentication.
            let mut cfg = rustls::ServerConfig::builder()
                .with_safe_defaults()
                .with_no_client_auth()
                .with_single_cert(certs, key)
                .map_err(|e| error(format!("{}", e)))?;
            // Configure ALPN to accept HTTP/2, HTTP/1.1 in that order.
            cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
            sync::Arc::new(cfg)
        };

        let incoming = AddrIncoming::bind(&addr)?;

        let make_svc = make_service_fn(|conn: &TlsStream| {
            let tx = atx.clone();
            let addr = conn.remote_addr;
            let service = service_fn(move |req| call(tx.clone(), addr, req));
            async move { Ok::<_, Infallible>(service) }
        });
        let server = Server::builder(TlsAcceptor::new(tls_cfg, incoming));

        let server = match http_version {
            HttpVersion::HTTP1 => server.http1_only(true),
            HttpVersion::HTTP2 => server.http2_only(true),
            HttpVersion::ANY => server,
        };
        let server = server.serve(make_svc);

        server.await?;
    } else {
        let make_svc = make_service_fn(|conn: &AddrStream| {
            let tx = atx.clone();
            let addr = conn.remote_addr();
            let service = service_fn(move |req| call(tx.clone(), addr, req));
            async move { Ok::<_, Infallible>(service) }
        });
        let server = Server::bind(&addr);

        let server = match http_version {
            HttpVersion::HTTP1 => server.http1_only(true),
            HttpVersion::HTTP2 => server.http2_only(true),
            HttpVersion::ANY => server,
        };
        let server = server.serve(make_svc);

        server.await?;
    }

    Ok(())
}

// currently copy/paste from https://github.com/rustls/hyper-rustls/blob/main/examples/server.rs

enum State {
    Handshaking(tokio_rustls::Accept<AddrStream>),
    Streaming(tokio_rustls::server::TlsStream<AddrStream>),
}

// tokio_rustls::server::TlsStream doesn't expose constructor methods,
// so we have to TlsAcceptor::accept and handshake to have access to it
// TlsStream implements AsyncRead/AsyncWrite handshaking tokio_rustls::Accept first
pub struct TlsStream {
    state: State,
    remote_addr: SocketAddr,
}

impl TlsStream {
    fn new(stream: AddrStream, config: Arc<ServerConfig>) -> TlsStream {
        let remote_addr = stream.remote_addr();
        let accept = tokio_rustls::TlsAcceptor::from(config).accept(stream);
        TlsStream {
            state: State::Handshaking(accept),
            remote_addr,
        }
    }
}

impl AsyncRead for TlsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<()>> {
        let pin = self.get_mut();
        match pin.state {
            State::Handshaking(ref mut accept) => match ready!(Pin::new(accept).poll(cx)) {
                Ok(mut stream) => {
                    let result = Pin::new(&mut stream).poll_read(cx, buf);
                    pin.state = State::Streaming(stream);
                    result
                }
                Err(err) => Poll::Ready(Err(err)),
            },
            State::Streaming(ref mut stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for TlsStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let pin = self.get_mut();
        match pin.state {
            State::Handshaking(ref mut accept) => match ready!(Pin::new(accept).poll(cx)) {
                Ok(mut stream) => {
                    let result = Pin::new(&mut stream).poll_write(cx, buf);
                    pin.state = State::Streaming(stream);
                    result
                }
                Err(err) => Poll::Ready(Err(err)),
            },
            State::Streaming(ref mut stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.state {
            State::Handshaking(_) => Poll::Ready(Ok(())),
            State::Streaming(ref mut stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.state {
            State::Handshaking(_) => Poll::Ready(Ok(())),
            State::Streaming(ref mut stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

pub struct TlsAcceptor {
    config: Arc<ServerConfig>,
    incoming: AddrIncoming,
}

impl TlsAcceptor {
    pub fn new(config: Arc<ServerConfig>, incoming: AddrIncoming) -> TlsAcceptor {
        TlsAcceptor { config, incoming }
    }
}

impl Accept for TlsAcceptor {
    type Conn = TlsStream;
    type Error = io::Error;

    fn poll_accept(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        let pin = self.get_mut();
        match ready!(Pin::new(&mut pin.incoming).poll_accept(cx)) {
            Some(Ok(sock)) => Poll::Ready(Some(Ok(TlsStream::new(sock, pin.config.clone())))),
            Some(Err(e)) => Poll::Ready(Some(Err(e))),
            None => Poll::Ready(None),
        }
    }
}

// Load public certificate from file.
fn load_certs(filename: &str) -> io::Result<Vec<rustls::Certificate>> {
    // Open certificate file.
    let certfile = fs::File::open(filename)
        .map_err(|e| error(format!("failed to open {}: {}", filename, e)))?;
    let mut reader = io::BufReader::new(certfile);

    // Load and return certificate.
    let certs = rustls_pemfile::certs(&mut reader)
        .map_err(|_| error("failed to load certificate".into()))?;
    Ok(certs.into_iter().map(rustls::Certificate).collect())
}

// Load private key from file.
fn load_private_key(filename: &str) -> io::Result<rustls::PrivateKey> {
    // Open keyfile.
    let keyfile = fs::File::open(filename)
        .map_err(|e| error(format!("failed to open {}: {}", filename, e)))?;
    let mut reader = io::BufReader::new(keyfile);

    // Load and return a single private key.
    let keys = rustls_pemfile::rsa_private_keys(&mut reader)
        .map_err(|_| error("failed to load private key".into()))?;
    if keys.len() != 1 {
        return Err(error("expected a single private key".into()));
    }

    Ok(rustls::PrivateKey(keys[0].clone()))
}

fn error(err: String) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err)
}

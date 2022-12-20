use std::{
    fs, io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{channel::mpsc, Future};
use futures_util::ready;
use hyper::{
    body::HttpBody,
    server::{
        accept::Accept,
        conn::{AddrIncoming, AddrStream},
    },
    service::Service,
    Body, Request, Response, StatusCode,
};
use rustls::ServerConfig;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::{
    prelude::*,
    protocol::http::{Receive, SendMethod},
    scope::configure_scope,
};

use crate::protocol::http::ReceiveRequest;
use crate::protocol::http::ReceiveTypes;
use crate::protocol::http::SendStart;
use crate::protocol::http::SendTypes;

pub struct Svc {
    app: PyObject,
    locals: TaskLocals,
    addr: SocketAddr,
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

impl Service<Request<Body>> for Svc {
    type Response = Response<Body>;
    type Error = hyper::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let (body_tx, body_rx) = mpsc::unbounded::<Result<Vec<u8>>>();
        let (send_tx, mut send_rx) = unbounded_channel::<SendTypes>();
        let (recv_tx, mut recv_rx) = unbounded_channel::<AsyncSender<ReceiveTypes>>();
        let send = SendMethod { tx: send_tx };
        let receive = Receive { tx: recv_tx };

        let scope =
            Python::with_gil(|py| configure_scope(py, &req, self.addr).unwrap().to_object(py));
        let mut response = Response::builder()
            .body(Body::wrap_stream(body_rx))
            .unwrap();
        let fut = Python::with_gil(|py| {
            let args = (scope, receive.into_py(py), send.into_py(py));
            let coro = self.app.call1(py, args).unwrap();
            pyo3_asyncio::into_future_with_locals(&self.locals, coro.as_ref(py))
        })
        .unwrap();

        let wait_parse_request = tokio::spawn(async move {
            let mut chunk_stream = req.into_body();

            while let Some(tx) = recv_rx.recv().await {
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
        let call_app = tokio::spawn(fut);

        Box::pin(async move {
            while let Some(_type) = send_rx.recv().await {
                let result = match _type {
                    SendTypes::HttpResponseStart(send_start) => {
                        response_start(&mut response, send_start)
                    }
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
            call_app.abort();
            Ok(response)
        })
    }
}

pub struct MakeSvc {
    app: PyObject,
    locals: TaskLocals,
}

impl MakeSvc {
    pub fn new(app: PyObject, locals: TaskLocals) -> Self {
        Self { app, locals }
    }
}

impl<'a> Service<&'a Stream> for MakeSvc {
    type Response = Svc;
    type Error = hyper::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, stream: &'a Stream) -> Self::Future {
        let app = self.app.clone();
        let locals = self.locals.clone();

        let addr = match stream {
            Stream::Http(addr_stream) => addr_stream.remote_addr(),
            Stream::Https(tls_stream) => tls_stream.remote_addr,
        };

        let fut = async move { Ok(Svc { app, locals, addr }) };
        Box::pin(fut)
    }
}

pub enum Stream {
    Http(AddrStream),
    Https(TlsStream),
}

impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let pin = self.get_mut();
        match pin {
            Stream::Http(v) => Pin::new(v).poll_read(cx, buf),
            Stream::Https(v) => Pin::new(v).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        let pin = self.get_mut();
        match pin {
            Stream::Http(v) => Pin::new(v).poll_write(cx, buf),
            Stream::Https(v) => Pin::new(v).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let pin = self.get_mut();
        match pin {
            Stream::Http(v) => Pin::new(v).poll_flush(cx),
            Stream::Https(v) => Pin::new(v).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        let pin = self.get_mut();
        match pin {
            Stream::Http(v) => Pin::new(v).poll_shutdown(cx),
            Stream::Https(v) => Pin::new(v).poll_shutdown(cx),
        }
    }
}

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

pub struct Acceptor {
    config: Option<Arc<ServerConfig>>,
    incoming: AddrIncoming,
}

impl Acceptor {
    pub fn new(config: Option<Arc<ServerConfig>>, incoming: AddrIncoming) -> Acceptor {
        Acceptor { config, incoming }
    }
}

impl Accept for Acceptor {
    type Conn = Stream;
    type Error = io::Error;

    fn poll_accept(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        let pin = self.get_mut();
        match ready!(Pin::new(&mut pin.incoming).poll_accept(cx)) {
            Some(Ok(sock)) => match pin.config.clone() {
                Some(cfg) => Poll::Ready(Some(Ok(Stream::Https(TlsStream::new(sock, cfg))))),
                None => Poll::Ready(Some(Ok(Stream::Http(sock)))),
            },
            Some(Err(e)) => Poll::Ready(Some(Err(e))),
            None => Poll::Ready(None),
        }
    }
}

// Load public certificate from file.
pub fn load_certs(filename: &str) -> io::Result<Vec<rustls::Certificate>> {
    // Open certificate file.
    let certfile = fs::File::open(filename).map_err(|e| {
        let err = format!("failed to open {}: {}", filename, e);
        io::Error::new(io::ErrorKind::Other, err)
    })?;
    let mut reader = io::BufReader::new(certfile);

    // Load and return certificate.
    let certs = rustls_pemfile::certs(&mut reader)
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "failed to load certificate"))?;
    Ok(certs.into_iter().map(rustls::Certificate).collect())
}

// Load private key from file.
pub fn load_private_key(filename: &str) -> io::Result<rustls::PrivateKey> {
    // Open keyfile.
    let keyfile = fs::File::open(filename).map_err(|e| {
        let err = format!("failed to open {}: {}", filename, e);
        io::Error::new(io::ErrorKind::Other, err)
    })?;
    let mut reader = io::BufReader::new(keyfile);

    // Load and return a single private key.
    let keys = rustls_pemfile::rsa_private_keys(&mut reader)
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "failed to load private key"))?;
    if keys.len() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "expected a single private key",
        ));
    }

    Ok(rustls::PrivateKey(keys[0].clone()))
}

use anyhow::Result;
use async_channel::{unbounded, Receiver as AsyncReceiver, Sender as AsyncSender};
use hyper::body::HttpBody;
use hyper::server::conn::AddrStream;
use hyper::service::make_service_fn;
use hyper::service::service_fn;
use hyper::StatusCode;
use hyper::Version;
use hyper::{Body, Request, Response, Server};
use log::debug;
use log::error;
use log::warn;
use pyo3::prelude::*;
use pyo3::types::*;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use urlencoding::decode;

mod errors;
use errors::HttpError;

#[derive(Debug, Clone)]
struct ReceiveRequest {
    body: Vec<u8>,
    more_body: bool,
}

#[derive(Debug, Clone)]
enum ReceiveTypes {
    HttpRequst(ReceiveRequest),
    HttpDisconect,
}

#[pyclass]
struct Receive {
    rx: AsyncReceiver<ReceiveTypes>,
    tx: AsyncSender<bool>,
}

#[pymethods]
impl Receive {
    fn __call__<'a>(&self, py: Python<'a>) -> PyResult<&'a PyAny> {
        let _rx = self.rx.clone();
        let _tx = self.tx.clone();

        pyo3_asyncio::tokio::future_into_py(py, async move {
            _tx.send(true).await.unwrap();
            let result = _rx.recv().await;
            let _type: ReceiveTypes;
            if let Ok(val) = result {
                _type = val;
            } else {
                debug!("Body not exists but `receive` called");
                _type = ReceiveTypes::HttpRequst(ReceiveRequest {
                    body: vec![],
                    more_body: false,
                });
            }
            debug!("{:?}", _type);
            let _d = Python::with_gil(|py| {
                let _d = match _type {
                    ReceiveTypes::HttpDisconect => {
                        PyDict::from_sequence(py, [("type", "http.disconnect")].into_py(py))
                            .unwrap()
                    }
                    ReceiveTypes::HttpRequst(req) => {
                        let mut _d =
                            PyDict::from_sequence(py, [("type", "http.request")].into_py(py))
                                .unwrap();
                        let body: &PyBytes = PyBytes::new(py, req.body.as_slice());
                        _d.set_item("body", body).unwrap();
                        _d.set_item("more_body", req.more_body.into_py(py)).unwrap();
                        _d
                    }
                };
                _d.to_object(py)
            });
            Ok(_d)
        })
    }
}

type Headers = Vec<Vec<Vec<u8>>>;

#[derive(Debug, Clone)]
struct SendStart {
    status: u16,
    headers: Headers,
    trailers: bool,
}

#[derive(Debug, Clone)]
struct SendBody {
    body: Vec<u8>,
    more_body: bool,
}

#[derive(Debug, Clone)]
enum SendTypes {
    HttpResponseStart(SendStart),
    HttpResponseBody(SendBody),
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

#[pyclass]
struct SendMethod {
    tx: AsyncSender<SendTypes>,
}

#[pymethods]
impl SendMethod {
    fn __call__<'a>(&self, py: Python<'a>, scope: &'a PyDict) -> PyResult<&'a PyAny> {
        let _type = scope.get_item("type").unwrap().to_string();
        let tx = self.tx.clone();
        match _type.as_str() {
            "http.response.start" => {
                let status: u16 = scope.get_item("status").unwrap().extract::<u16>()?;
                let headers: Headers = scope.get_item("headers").unwrap().extract::<Headers>()?;
                pyo3_asyncio::tokio::future_into_py(py, async move {
                    tx.send(SendTypes::HttpResponseStart(SendStart {
                        status,
                        headers,
                        trailers: false,
                    }))
                    .await
                    .unwrap();
                    Ok(())
                })
            }
            "http.response.body" => {
                let body: Vec<u8> = scope.get_item("body").unwrap().extract::<Vec<u8>>()?;
                let py_more_body = scope.get_item("more_body");
                let more_body: bool;
                if let Some(py_more_body) = py_more_body {
                    more_body = py_more_body.extract::<bool>().unwrap_or(false);
                } else {
                    more_body = false;
                }

                pyo3_asyncio::tokio::future_into_py(py, async move {
                    tx.send(SendTypes::HttpResponseBody(SendBody { body, more_body }))
                        .await
                        .unwrap();
                    Ok(())
                })
            }
            _ => panic!("unknown type {}", _type),
        }
    }
}

#[derive(Debug)]
struct ASGIRequest {
    send: AsyncSender<SendTypes>,
    receive: (AsyncReceiver<ReceiveTypes>, AsyncSender<bool>),
    scope: Py<PyAny>,
}

async fn call(
    req_tx: Arc<AsyncSender<ASGIRequest>>,
    addr: SocketAddr,
    req: Request<Body>,
) -> Result<Response<Body>, Infallible> {
    let mut response = Response::default();
    let mut buf: Vec<u8> = vec![];
    let (send_tx, send_rx) = unbounded::<SendTypes>();
    let (recv_tx, recv_rx) = unbounded::<ReceiveTypes>();
    let (recv_enabled_tx, recv_enabled_rx) = unbounded::<bool>();

    let scope = Python::with_gil(|py| configure_scope(py, &req, addr).unwrap().to_object(py));
    req_tx
        .send(ASGIRequest {
            scope,
            send: send_tx,
            receive: (recv_rx, recv_enabled_tx),
        })
        .await
        .unwrap();

    let wait_parse_request = tokio::spawn(async move {
        let mut chunk_stream = req.into_body();

        while recv_enabled_rx.recv().await.is_ok() {
            let chunk = chunk_stream.data().await;
            if let Some(chunk) = chunk {
                match chunk {
                    Err(e) => {
                        warn!("{}", e.to_string());
                        recv_tx.send(ReceiveTypes::HttpDisconect).await.unwrap();
                        break;
                    }
                    Ok(data) => {
                        let more_body = !chunk_stream.is_end_stream();
                        recv_tx
                            .send(ReceiveTypes::HttpRequst(ReceiveRequest {
                                body: data.into(),
                                more_body,
                            }))
                            .await
                            .unwrap();
                    }
                }
            } else {
                debug!("end request body");
                break;
            }
        }
    });

    while let Ok(_type) = send_rx.recv().await {
        debug!("{:?}", _type);

        let result = match _type {
            SendTypes::HttpResponseStart(send_start) => response_start(&mut response, send_start),
            SendTypes::HttpResponseBody(mut send_body) => {
                buf.append(&mut send_body.body);
                if !send_body.more_body {
                    debug!("End response body");
                    *response.body_mut() = buf.into();
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

async fn start_server(tx: AsyncSender<ASGIRequest>) {
    let addr = SocketAddr::from(([127, 0, 0, 1], 8000));

    let atx = Arc::new(tx);

    let make_svc = make_service_fn(move |conn: &AddrStream| {
        let tx = atx.clone();
        let addr = conn.remote_addr();
        let service = service_fn(move |req| call(tx.clone(), addr, req));
        async move { Ok::<_, Infallible>(service) }
    });

    let server = Server::bind(&addr).serve(make_svc);

    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
}

fn configure_scope<'a>(
    py: Python<'a>,
    req: &Request<Body>,
    addr: SocketAddr,
) -> PyResult<&'a PyDict> {
    let mut scope: HashMap<&str, &str> = HashMap::from([
        ("type", "http"),
        (
            "http_version",
            match req.version() {
                Version::HTTP_10 => "1.0",
                Version::HTTP_11 => "1.1",
                Version::HTTP_2 => "2.0",
                v => panic!("Unsupported http version {:?}", v),
            },
        ),
    ]);
    let method = req.method().as_str().to_uppercase();
    scope.insert("method", &method);
    debug!("{:?}", req.uri().scheme_str());
    scope.insert("scheme", req.uri().scheme_str().unwrap_or(""));
    scope.insert("root_path", "");
    let path: String = decode(req.uri().path()).expect("UTF-8").into();
    let raw_path = req.uri().path().as_bytes();
    let query_string = match req.uri().query() {
        Some(s) => decode(s).expect("UTF-8").to_string(),
        None => "".to_string(),
    };
    let client = (addr.ip().to_string(), addr.port());

    let mut headers = vec![];

    for (key, val) in req.headers().iter() {
        headers.push((key.as_str().as_bytes(), val.as_bytes()));
    }

    let dict = scope.into_py_dict(py);

    dict.set_item("headers", headers.into_py(py))?;
    dict.set_item("client", (client.0.into_py(py), client.1.into_py(py)))?;
    dict.set_item("path", path.into_py(py))?;
    dict.set_item("raw_path", raw_path.into_py(py))?;
    dict.set_item("query_string", query_string.as_bytes().into_py(py))?;

    debug!("{:?}", dict);
    Ok(dict)
}

#[pyfunction]
fn start_app<'a>(py: Python<'a>, app: &'a PyAny) -> PyResult<&'a PyAny> {
    let (req_tx, req_rx) = unbounded::<ASGIRequest>();

    let app: PyObject = app.into();

    pyo3_asyncio::tokio::future_into_py(py, async move {
        tokio::spawn(async move {
            start_server(req_tx).await;
        });
        while let Ok(areq) = req_rx.recv().await {
            let send = SendMethod { tx: areq.send };
            let receive = Receive {
                rx: areq.receive.0,
                tx: areq.receive.1,
            };

            let fut = Python::with_gil(|py| {
                let scope = areq.scope;
                let args = (scope, receive.into_py(py), send.into_py(py));
                let coro = app.call1(py, args).unwrap();
                pyo3_asyncio::tokio::into_future(coro.as_ref(py))
            })?;

            fut.await?;
        }
        Ok(())
    })
}

#[pymodule]
fn rusticorn(_py: Python, m: &PyModule) -> PyResult<()> {
    pyo3_log::init();
    m.add_function(wrap_pyfunction!(start_app, m)?)?;
    Ok(())
}

use anyhow::Result;
use hyper::http::uri::Scheme;
use hyper::server::conn::AddrStream;
use hyper::service::make_service_fn;
use hyper::service::service_fn;
use hyper::StatusCode;
use hyper::Version;
use hyper::{Body, Request, Response, Server};
use log::debug;
use log::info;
use pyo3::prelude::*;
use pyo3::types::*;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::mpsc::UnboundedSender;
use urlencoding::decode;

#[pyclass]
struct Receive;

#[pymethods]
impl Receive {
    fn __call__<'a>(&self, py: Python<'a>, scope: &'a PyDict) -> PyResult<&'a PyAny> {
        debug!("{:?}", scope);
        pyo3_asyncio::tokio::future_into_py(py, async { Ok(()) })
    }
}

#[derive(Debug, Clone)]
struct SendStart {
    status: u16,
    headers: Vec<Vec<Vec<u8>>>,
    trailers: bool,
}

#[derive(Debug, Clone)]
struct SendBody {
    body: Vec<u8>,
    more_body: bool,
}

#[derive(Debug, Clone)]
enum Types {
    HttpResponseStart(SendStart),
    HttpResponseBody(SendBody),
}

fn response_start<T>(resp: &mut Response<T>, send_start: SendStart) {
    *resp.status_mut() = StatusCode::from_u16(send_start.status).unwrap();
    for header in send_start.headers {
        let name = header.first().unwrap();
        let value = header.last().unwrap();
        let name = hyper::http::header::HeaderName::from_bytes(name).unwrap();
        let value = hyper::http::header::HeaderValue::from_bytes(value).unwrap();

        resp.headers_mut().insert(name, value);
    }
}

#[pyclass]
struct SendMethod {
    tx: UnboundedSender<Types>,
}

#[pymethods]
impl SendMethod {
    fn __call__<'a>(&self, py: Python<'a>, scope: &'a PyDict) -> PyResult<&'a PyAny> {
        let _type = scope.get_item("type").unwrap().to_string();
        match _type.as_str() {
            "http.response.start" => {
                let status: u16 = scope.get_item("status").unwrap().extract::<u16>().unwrap();
                let headers: Vec<Vec<Vec<u8>>> = scope
                    .get_item("headers")
                    .unwrap()
                    .extract::<Vec<Vec<Vec<u8>>>>()
                    .unwrap();
                self.tx
                    .send(Types::HttpResponseStart(SendStart {
                        status,
                        headers,
                        trailers: false,
                    }))
                    .unwrap();
            }
            "http.response.body" => {
                let body: Vec<u8> = scope
                    .get_item("body")
                    .unwrap()
                    .extract::<Vec<u8>>()
                    .unwrap();

                self.tx
                    .send(Types::HttpResponseBody(SendBody {
                        body,
                        more_body: false,
                    }))
                    .unwrap();
            }
            _ => panic!("unknown type {}", _type),
        };
        pyo3_asyncio::tokio::future_into_py(py, async move { Ok(()) })
    }
}

#[derive(Debug)]
struct ASGIRequest<T> {
    req: Request<T>,
    tx: UnboundedSender<Types>,
    addr: SocketAddr,
}

async fn call(
    req_tx: Arc<UnboundedSender<ASGIRequest<Body>>>,
    addr: SocketAddr,
    req: Request<Body>,
) -> Result<Response<Body>, Infallible> {
    let (mut btx, body) = Body::channel();
    let mut response = Response::new(body);

    let (tx, mut rx) = unbounded_channel::<Types>();

    req_tx.send(ASGIRequest { addr, req, tx }).unwrap();
    while let Some(_type) = rx.recv().await {
        debug!("{:?}", _type);

        match _type {
            Types::HttpResponseStart(send_start) => response_start(&mut response, send_start),
            Types::HttpResponseBody(send_body) => {
                btx.send_data(send_body.body.into()).await.unwrap();
                if !send_body.more_body {
                    break;
                }
            }
        }
    }

    Ok(response)
}

async fn start_server(tx: UnboundedSender<ASGIRequest<Body>>) {
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));

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
    let method = req.method().as_str().to_lowercase();
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
    dict.set_item("path", path.as_bytes().into_py(py))?;
    dict.set_item("raw_path", raw_path.into_py(py))?;
    dict.set_item("query_string", query_string.as_bytes().into_py(py))?;

    Ok(dict)
}

#[pyfunction]
fn start_app<'a>(py: Python<'a>, app: &'a PyFunction) -> PyResult<&'a PyAny> {
    let (req_tx, mut req_rx) = unbounded_channel::<ASGIRequest<Body>>();

    let app: PyObject = app.into();

    pyo3_asyncio::tokio::future_into_py(py, async move {
        tokio::spawn(async move {
            start_server(req_tx).await;
        });
        while let Some(areq) = req_rx.recv().await {
            let send = SendMethod { tx: areq.tx };
            let receive = Receive {};

            let fut = Python::with_gil(|py| {
                let scope = configure_scope(py, &areq.req, areq.addr)?;
                let args = (scope, receive.into_py(py), send.into_py(py));
                let coro = app.call1(py, args).unwrap();
                pyo3_asyncio::tokio::into_future(coro.as_ref(py))
            })
            .unwrap();

            fut.await.unwrap();
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

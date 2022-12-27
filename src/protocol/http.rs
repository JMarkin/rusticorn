use futures::channel::mpsc;

use hyper::{body::HttpBody, Request, StatusCode};

use crate::{configure_scope, prelude::*, server_header};

use super::common::insert_headers;

#[derive(Debug, Clone)]
pub struct ReceiveRequest {
    pub body: Vec<u8>,
    pub more_body: bool,
}

#[derive(Debug, Clone)]
pub enum ReceiveTypes {
    HttpRequst(ReceiveRequest),
    HttpDisconect,
}

#[pyclass]
pub struct ReceiveFunc {
    pub tx: UnboundedSender<Sender<ReceiveTypes>>,
}

#[pymethods]
impl ReceiveFunc {
    fn __call__<'a>(&self, py: Python<'a>) -> PyResult<&'a PyAny> {
        let _tx = self.tx.clone();

        // TODO: change to AwaitableObj
        pyo3_asyncio::tokio::future_into_py(py, async move {
            let (tx, rx) = channel::<ReceiveTypes>();
            _tx.send(tx).unwrap();
            let result = rx.await;
            let _type: ReceiveTypes;
            if let Ok(val) = result {
                _type = val;
            } else {
                _type = ReceiveTypes::HttpRequst(ReceiveRequest {
                    body: vec![],
                    more_body: false,
                });
            }
            let _d = Python::with_gil(|py| {
                let dict = PyDict::new(py);
                match _type {
                    ReceiveTypes::HttpDisconect => {
                        dict.set_item("type", "http.disconnect").unwrap();
                    }
                    ReceiveTypes::HttpRequst(req) => {
                        dict.set_item("type", "http.request").unwrap();
                        let body: &PyBytes = PyBytes::new(py, req.body.as_slice());
                        dict.set_item("body", body).unwrap();
                        dict.set_item("more_body", req.more_body.into_py(py))
                            .unwrap();
                    }
                };
                dict.to_object(py)
            });
            Ok(_d)
        })
    }
}

#[derive(Debug, Clone)]
pub struct SendStart {
    pub status: u16,
    pub headers: Headers,
    pub trailers: bool,
}

#[derive(Debug, Clone)]
pub struct SendBody {
    pub body: Vec<u8>,
    pub more_body: bool,
}

#[derive(Debug, Clone)]
pub enum SendTypes {
    HttpResponseStart(SendStart),
    HttpResponseBody(SendBody),
}

#[pyclass]
pub struct SendFunc {
    pub tx: UnboundedSender<SendTypes>,
}

fn bool_from_scope(scope: &PyDict, name: &str) -> bool {
    let py_bool = scope.get_item(name);
    let _bool: bool;
    if let Some(py_more_body) = py_bool {
        _bool = py_more_body.extract::<bool>().unwrap_or(false);
    } else {
        _bool = false;
    }
    _bool
}

#[pymethods]
impl SendFunc {
    fn __call__(&self, scope: &PyDict) -> PyResult<AwaitableObj> {
        let _type = scope.get_item("type").unwrap().to_string();
        let result = match _type.as_str() {
            "http.response.start" => {
                let status: u16 = scope.get_item("status").unwrap().extract::<u16>()?;
                let headers: Headers = scope.get_item("headers").unwrap().extract::<Headers>()?;
                let trailers = bool_from_scope(scope, "trailers");
                self.tx.send(SendTypes::HttpResponseStart(SendStart {
                    status,
                    headers,
                    trailers,
                }))
            }
            "http.response.body" => {
                let body: Vec<u8> = scope.get_item("body").unwrap().extract::<Vec<u8>>()?;
                let more_body = bool_from_scope(scope, "more_body");

                self.tx
                    .send(SendTypes::HttpResponseBody(SendBody { body, more_body }))
            }
            _ => panic!("unknown type {}", _type),
        };

        match result {
            Ok(_) => Ok(AwaitableObj::default()),
            Err(e) => {
                error!("{}", e.to_string());
                Ok(AwaitableObj::default())
            }
        }
    }
}

pub fn handle(
    app: PyObject,
    locals: TaskLocals,
    addr: SocketAddr,
    req: Request<Body>,
) -> FutureResponse {
    let (body_tx, body_rx) = mpsc::unbounded::<Result<Vec<u8>>>();
    let (send_tx, mut send_rx) = unbounded_channel::<SendTypes>();
    let (recv_tx, mut recv_rx) = unbounded_channel::<Sender<ReceiveTypes>>();
    let send = SendFunc { tx: send_tx };
    let receive = ReceiveFunc { tx: recv_tx };

    let scope = Python::with_gil(|py| {
        configure_scope(ScopeType::Http, py, &req, addr)
            .unwrap()
            .to_object(py)
    });
    let mut response = Response::builder()
        .body(Body::wrap_stream(body_rx))
        .unwrap();
    let fut = Python::with_gil(|py| {
        let args = (scope, receive.into_py(py), send.into_py(py));
        let coro = app.call1(py, args).unwrap();
        pyo3_asyncio::into_future_with_locals(&locals, coro.as_ref(py))
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
                        tx.send(ReceiveTypes::HttpDisconect).unwrap();
                        break;
                    }
                    Ok(data) => {
                        let more_body = !chunk_stream.is_end_stream();
                        tx.send(ReceiveTypes::HttpRequst(ReceiveRequest {
                            body: data.into(),
                            more_body,
                        }))
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
                    *response.status_mut() = StatusCode::from_u16(send_start.status).unwrap();
                    insert_headers(&mut response, send_start.headers)
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
        (*response.headers_mut()).append("server", server_header!());
        Ok(response)
    })
}

use std::sync::Arc;

use futures::channel::mpsc::{self, UnboundedSender};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender as TokioUnboundedSender};

use http_body_util::{BodyExt, StreamBody};
use hyper::{
    body::{Bytes, Frame},
    Request, StatusCode,
};
use tokio::sync::Mutex;

use crate::{prelude::*, server_header};

use super::common::insert_headers;

#[pyclass]
#[derive(Clone)]
struct Receiver {
    body: Arc<Mutex<IncomingBody>>,
}

#[pymethods]
impl Receiver {
    fn __call__(&self) -> PyAsync<Result<PyObject>> {
        debug!("recv called");
        let _body = self.body.clone();
        async move {
            let body = _body.lock().await.frame().await;

            match body {
                None => Python::with_gil(|py| {
                    let dict = PyDict::new(py);
                    dict.set_item("type", ScopeType::HttpRequest.into_py(py))?;
                    dict.set_item("more_body", false.into_py(py))?;
                    dict.set_item("body", PyBytes::new(py, &[]))?;
                    Ok(dict.into())
                }),
                Some(v) => {
                    let body = v;

                    match body {
                        Err(e) => {
                            error!("{}", e);
                            Python::with_gil(|py| {
                                let dict = PyDict::new(py);
                                dict.set_item("type", ScopeType::HttpDisconect.into_py(py))?;
                                Ok(dict.into())
                            })
                        }
                        Ok(frame) => {
                            let bytes = frame.into_data();
                            let bytes = match bytes {
                                Ok(bytes) => Some(bytes),
                                Err(prev) => {
                                    error!("error on recv bytes; previous: {:?}", prev);
                                    None
                                }
                            };
                            Python::with_gil(|py| {
                                let dict = PyDict::new(py);
                                dict.set_item("type", ScopeType::HttpRequest.into_py(py))?;
                                dict.set_item("more_body", bytes.is_some().into_py(py))?;

                                if bytes.is_some() {
                                    dict.set_item("body", bytes.unwrap().as_ref().into_py(py))?;
                                } else {
                                    dict.set_item("body", PyBytes::new(py, &[]))?;
                                }

                                Ok(dict.into())
                            })
                        }
                    }
                }
            }
        }
        .into()
    }
}

#[derive(Debug, Clone)]
struct ResponseStart {
    status: u16,
    headers: Headers,
    trailers: bool,
}

#[pyclass]
#[derive(Clone)]
struct Sender {
    writer: UnboundedSender<Result<Frame<Bytes>>>,
    response_start: TokioUnboundedSender<ResponseStart>,
}

#[pymethods]
impl Sender {
    fn __call__(&self, scope: &PyDict) -> PyAsync<Result<()>> {
        debug!("send called {:?}", scope);
        let _type = scope
            .get_item("type")
            .expect("type required")
            .extract::<ScopeType>()
            .unwrap();

        let result = match _type {
            ScopeType::HttpResponseStart => {
                let status: u16 = scope.get_item("status").unwrap().extract::<u16>().unwrap();
                let headers: Headers = scope
                    .get_item("headers")
                    .unwrap()
                    .extract::<Headers>()
                    .unwrap();
                let trailers = bool_from_scope(scope, "trailers");
                self.response_start
                    .send(ResponseStart {
                        trailers,
                        status,
                        headers,
                    })
                    .unwrap();
                Ok(())
            }
            ScopeType::HttpResponseBody => {
                let body: Vec<u8> = scope
                    .get_item("body")
                    .unwrap()
                    .extract::<Vec<u8>>()
                    .unwrap();
                let more_body = bool_from_scope(scope, "more_body");

                let result = self
                    .writer
                    .unbounded_send(Ok(Frame::data(Bytes::from(body))));
                if !more_body {
                    self.writer.close_channel();
                };

                result
            }
            _ => panic!(
                "unsupported type {}; available {} {}",
                _type,
                ScopeType::HttpResponseStart,
                ScopeType::HttpResponseBody
            ),
        };
        async {
            match result {
                Ok(_) => Ok(()),
                Err(e) => {
                    error!("{}", e);
                    Ok(())
                }
            }
        }
        .into()
    }
}

pub fn handle(
    addr: SocketAddr,
    req: Request<IncomingBody>,
    tx: TokioUnboundedSender<ScopeRecvSend>,
) -> FutureResponse {
    let (body_tx, body_rx) = mpsc::unbounded();
    let (response_start_tx, mut response_start_rx) = unbounded_channel();

    let sender = Sender {
        writer: body_tx,
        response_start: response_start_tx,
    };
    let (parts, req_body) = req.into_parts();
    let receiver = Receiver {
        body: Arc::new(Mutex::new(req_body)),
    };

    let scope = Scope {
        req: parts,
        addr,
        _type: ScopeType::Http,
        _dict: Python::with_gil(|py| PyDict::new(py).into()),
    };

    let args = Python::with_gil(|py| (scope.into_py(py), receiver.into_py(py), sender.into_py(py)));

    Box::pin(async move {
        let result = tx.send(Some(args));
        if let Err(e) = result {
            return Ok(HttpError::internal_error(e.into()));
        }

        let result = Response::builder().body(HttpBody::Streamning {
            body: StreamBody::new(body_rx),
        });

        let mut response = match result {
            Ok(r) => r,
            Err(e) => {
                return Ok(HttpError::internal_error(e.into()));
            }
        };

        match response_start_rx.recv().await {
            Some(start) => {
                *response.status_mut() = StatusCode::from_u16(start.status).unwrap();
                insert_headers(&mut response, start.headers).unwrap();
            }
            None => {
                return Ok(HttpError::internal_error(anyhow!(
                    "first send type must to be a 'http.response.start'"
                )));
            }
        }

        (*response.headers_mut()).append("server", server_header!());
        Ok(response)
    })
}

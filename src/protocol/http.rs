use crate::prelude::*;

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
pub struct Receive {
    pub tx: AsyncSender<AsyncSender<ReceiveTypes>>,
}

#[pymethods]
impl Receive {
    fn __call__<'a>(&self, py: Python<'a>) -> PyResult<&'a PyAny> {
        let _tx = self.tx.clone();

        pyo3_asyncio::tokio::future_into_py(py, async move {
            let (tx, rx) = unbounded::<ReceiveTypes>();
            _tx.send(tx).await.unwrap();
            let result = rx.recv().await;
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

type Headers = Vec<Vec<Vec<u8>>>;

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
pub struct SendMethod {
    pub tx: AsyncSender<SendTypes>,
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
impl SendMethod {
    fn __call__<'a>(&self, py: Python<'a>, scope: &'a PyDict) -> PyResult<&'a PyAny> {
        let _type = scope.get_item("type").unwrap().to_string();
        let tx = self.tx.clone();
        match _type.as_str() {
            "http.response.start" => {
                let status: u16 = scope.get_item("status").unwrap().extract::<u16>()?;
                let headers: Headers = scope.get_item("headers").unwrap().extract::<Headers>()?;
                let trailers = bool_from_scope(scope, "trailers");
                pyo3_asyncio::tokio::future_into_py(py, async move {
                    let result = tx
                        .send(SendTypes::HttpResponseStart(SendStart {
                            status,
                            headers,
                            trailers,
                        }))
                        .await;

                    match result {
                        Ok(_) => Ok(()),
                        Err(e) => {
                            error!("{}", e.to_string());
                            Ok(())
                        }
                    }
                })
            }
            "http.response.body" => {
                let body: Vec<u8> = scope.get_item("body").unwrap().extract::<Vec<u8>>()?;
                let more_body = bool_from_scope(scope, "more_body");

                pyo3_asyncio::tokio::future_into_py(py, async move {
                    let result = tx
                        .send(SendTypes::HttpResponseBody(SendBody { body, more_body }))
                        .await;

                    match result {
                        Ok(_) => Ok(()),
                        Err(e) => {
                            error!("{}", e.to_string());
                            Ok(())
                        }
                    }
                })
            }
            _ => panic!("unknown type {}", _type),
        }
    }
}

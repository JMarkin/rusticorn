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
    pub rx: AsyncReceiver<ReceiveTypes>,
    pub tx: AsyncSender<bool>,
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

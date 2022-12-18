mod enums;
mod errors;
mod prelude;
mod protocol;
mod scope;
mod server;
use crate::prelude::*;
use protocol::http::{Receive, SendMethod};
use server::{start_server, ASGIRequest};

#[pyfunction]
fn start_app<'a>(
    py: Python<'a>,
    app: &'a PyAny,
    bind: Option<&'a str>,
    tls: Option<bool>,
    cert_path: Option<String>,
    private_path: Option<String>,
    http_version: Option<String>,
) -> PyResult<&'a PyAny> {
    let (req_tx, req_rx) = unbounded::<ASGIRequest>();

    let app: PyObject = app.into();

    let addr = bind.unwrap_or("127.0.0.1:8000").parse()?;
    let tls = tls.unwrap_or(false);
    let _http_version: HttpVersion;
    if let Some(_version) = http_version {
        _http_version = _version.parse().unwrap_or(HttpVersion::ANY);
    } else {
        _http_version = HttpVersion::ANY;
    }

    pyo3_asyncio::tokio::future_into_py(py, async move {
        tokio::spawn(async move {
            start_server(req_tx, addr, tls, cert_path, private_path, _http_version)
                .await
                .unwrap();
        });
        while let Ok(areq) = req_rx.recv().await {
            let send = SendMethod { tx: areq.send };
            let receive = Receive { tx: areq.receive };

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

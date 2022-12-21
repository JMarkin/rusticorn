mod enums;
mod errors;
mod prelude;
mod protocol;
mod scope;
mod server;
mod service;
mod py;
use crate::prelude::*;
use server::start_server;

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
    let app: PyObject = app.into();

    let addr = bind.unwrap_or("127.0.0.1:8000").parse()?;
    let tls = tls.unwrap_or(false);
    let _http_version: HttpVersion;
    if let Some(_version) = http_version {
        _http_version = _version.parse().unwrap_or(HttpVersion::Any);
    } else {
        _http_version = HttpVersion::Any;
    }

    let locals = Python::with_gil(|py| pyo3_asyncio::tokio::get_current_locals(py).unwrap());
    pyo3_asyncio::tokio::future_into_py(py, async move {
        match start_server(
            locals,
            app,
            addr,
            tls,
            cert_path,
            private_path,
            _http_version,
        )
        .await
        {
            Ok(_v) => Ok(()),
            Err(e) => panic!("{e}"),
        }
    })
}

#[pymodule]
fn rusticorn(_py: Python, m: &PyModule) -> PyResult<()> {
    pyo3_log::init();
    m.add_function(wrap_pyfunction!(start_app, m)?)?;
    Ok(())
}

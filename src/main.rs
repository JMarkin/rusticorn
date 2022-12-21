use std::env;

use crate::prelude::*;
use crate::server::start_server;
mod enums;
mod errors;
mod prelude;
mod protocol;
mod scope;
mod server;
mod service;
mod py;

#[pyo3_asyncio::tokio::main]
async fn main() -> PyResult<()> {
    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    let path = env::current_dir().unwrap();

    let app: PyObject = Python::with_gil(|py| {
        let syspath: &PyList = py
            .import("sys")
            .unwrap()
            .getattr("path")
            .unwrap()
            .downcast::<PyList>()
            .unwrap();
        syspath.insert(0, &path).unwrap();
        let module = PyModule::import(py, "example.app").unwrap();
        module.getattr("app").unwrap().into()
    });

    let locals = Python::with_gil(|py| pyo3_asyncio::tokio::get_current_locals(py).unwrap());

    start_server(locals, app, addr, false, None, None, HttpVersion::Any)
        .await
        .unwrap();
    Ok(())
}

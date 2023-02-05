#[macro_use]
extern crate anyhow;
mod body;
mod config;
mod enums;
mod errors;
mod prelude;
mod protocol;
mod py_future;
mod scope;
mod server;
mod utils;

use crate::prelude::*;

#[pyfunction]
fn start_server(cfg: Config) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(cfg.workers)
        .enable_all()
        .build()
        .expect("build runtime");

    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, crate::server::start_server(cfg))
}

#[pymodule]
fn rusticorn(_py: Python, m: &PyModule) -> PyResult<()> {
    pyo3_log::init();
    m.add_function(wrap_pyfunction!(start_server, m)?)?;
    m.add_class::<Config>()?;
    m.add_class::<Scope>()?;
    Ok(())
}

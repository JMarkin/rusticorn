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

use std::thread;

use server::ASGIServer;
use tokio::sync::mpsc::{channel, unbounded_channel};

use crate::prelude::*;

#[pyfunction]
fn start_server(py: Python, cfg: Config) -> Result<ASGIServer> {
    let (tx, rx) = unbounded_channel();
    let (stop_tx, stop_rx) = channel::<bool>(1);
    py.allow_threads(move || {
        thread::spawn(move || {
            pyo3::prepare_freethreaded_python();
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build runtime");

            // Combine it with a `LocalSet,  which means it can spawn !Send futures...
            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, crate::server::start_server(cfg, tx, stop_tx)) //, _event_loop, _asyncio))
        })
    });

    Ok(ASGIServer::new(rx, stop_rx))
}

#[pymodule]
fn rusticorn(_py: Python, m: &PyModule) -> PyResult<()> {
    pyo3_log::init();
    m.add_function(wrap_pyfunction!(start_server, m)?)?;
    m.add_class::<Config>()?;
    Ok(())
}

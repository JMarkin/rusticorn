use crate::{configure_scope, prelude::*};
use hyper::Request;

async fn serve_ws(ws: HyperWebsocket) -> Result<()> {
    Ok(())
}

pub fn handle(
    app: PyObject,
    locals: TaskLocals,
    addr: SocketAddr,
    mut req: Request<Body>,
) -> FutureResponse {
    let (body_tx, body_rx) = mpsc::unbounded::<Result<Vec<u8>>>();
    let (send_tx, mut send_rx) = unbounded_channel::<SendTypes>();
    let (recv_tx, mut recv_rx) = unbounded_channel::<Sender<ReceiveTypes>>();
    let send = SendMethod { tx: send_tx };
    let receive = Receive { tx: recv_tx };

    let scope = Python::with_gil(|py| {
        configure_scope(ScopeType::Ws, py, &req, addr)
            .unwrap()
            .to_object(py)
    });
    let fut = Python::with_gil(|py| {
        let args = (scope, receive.into_py(py), send.into_py(py));
        let coro = app.call1(py, args).unwrap();
        pyo3_asyncio::into_future_with_locals(&locals, coro.as_ref(py))
    })
    .unwrap();

    let (response, ws) = hyper_tungstenite::upgrade(&mut req, None).unwrap();
    tokio::spawn(async move {
        if let Err(e) = serve_ws(ws).await {
            eprintln!("Error in websocket connection: {}", e);
        }
    });

    Box::pin(async move { Ok(response) })
}

use std::sync::Arc;

use crate::{configure_scope, prelude::*};
use anyhow::Error;
use futures::{sink::SinkExt, stream::FusedStream, StreamExt};
use hyper::{Request, StatusCode};
use hyper_tungstenite::tungstenite::{
    protocol::{frame::coding::CloseCode, CloseFrame},
    Message,
};
use tokio::sync::Mutex;

use super::common::insert_headers;

#[derive(Debug, Clone)]
pub enum ReceiveType {
    Disconnect(u16),
    Receive(Message),
    Connect,
}

#[pyclass]
pub struct ReceiveFunc {
    pub tx: UnboundedSender<Sender<ReceiveType>>,
}

#[pymethods]
impl ReceiveFunc {
    fn __call__<'a>(&self, py: Python<'a>) -> PyResult<&'a PyAny> {
        let _tx = self.tx.clone();

        // TODO: change to AwaitableObj
        pyo3_asyncio::tokio::future_into_py(py, async move {
            let (tx, rx) = channel::<ReceiveType>();
            _tx.send(tx).unwrap();

            let result = rx.await;
            let _type: ReceiveType;
            if let Ok(val) = result {
                _type = val;
            } else {
                _type = ReceiveType::Disconnect(1005);
            }
            let _d = Python::with_gil(|py| -> PyResult<PyObject> {
                let dict = PyDict::new(py);
                match _type {
                    ReceiveType::Disconnect(code) => {
                        dict.set_item("type", "websocket.disconnect")?;
                        dict.set_item("code", code)?;
                    }
                    ReceiveType::Connect => {
                        dict.set_item("type", "websocket.connect")?;
                    }
                    ReceiveType::Receive(message) => {
                        dict.set_item("type", "websocket.receive")?;
                        if message.is_text() {
                            dict.set_item("text", message.clone().into_text().unwrap())?;
                        }

                        if message.is_binary() {
                            dict.set_item("bytes", message.into_data())?;
                        }
                    }
                };
                Ok(dict.to_object(py))
            })?;
            debug!("receive {}", _d);
            Ok(_d)
        })
    }
}

#[derive(Debug, Clone)]
pub enum SendType {
    Send(Message),
    Close((u16, String)),
    Accept(Headers),
}

#[pyclass]
pub struct SendFunc {
    pub tx: UnboundedSender<SendType>,
}

#[pymethods]
impl SendFunc {
    fn __call__(&self, scope: &PyDict) -> PyResult<AwaitableObj> {
        debug!("send {}", scope);
        let _type = scope.get_item("type").unwrap().to_string();
        let result: Result<(), Error> = match _type.as_str() {
            "websocket.accept" => {
                let subprotocol = scope.get_item("subprotocol");
                let mut headers = scope.get_item("headers").unwrap().extract::<Headers>()?;

                if let Some(subprotocol) = subprotocol {
                    let subprotocol = subprotocol.extract::<String>()?;
                    headers.push(vec![b"sec-websocket-protocol".to_vec(), subprotocol.into()]);
                }

                debug!("sended0");
                self.tx.send(SendType::Accept(headers)).unwrap();
                debug!("sended");
                Ok(())
            }
            "websocket.close" => {
                let mut code = 1000;
                let code_none = scope.get_item("code");
                if let Some(_code) = code_none {
                    code = _code.extract::<u16>()?;
                }
                let mut reason = "".to_string();
                let reason_none = scope.get_item("reason");
                if let Some(_reason) = reason_none {
                    reason = _reason.extract::<String>()?;
                }
                self.tx.send(SendType::Close((code, reason))).unwrap();
                Ok(())
            }
            "websocket.send" => {
                let bytes = scope.get_item("bytes");
                let text = scope.get_item("text");
                if bytes.is_none() && text.is_none() {
                    panic!("bytes or text must exists");
                }

                if let Some(bytes) = bytes {
                    self.tx
                        .send(SendType::Send(Message::Binary(bytes.extract::<Vec<u8>>()?)))
                        .unwrap();
                }

                if let Some(text) = text {
                    self.tx
                        .send(SendType::Send(Message::Text(text.extract::<String>()?)))
                        .unwrap();
                }
                Ok(())
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

async fn serve_ws(
    ws: HyperWebsocket,
    mut send_rx: UnboundedReceiver<SendType>,
    mut recv_rx: UnboundedReceiver<Sender<ReceiveType>>,
) -> Result<()> {
    let websocket = ws.await?;
    let (mut tx, mut rx) = websocket.split();

    let recv_fut = tokio::spawn(async move {
        while let Some(recv) = recv_rx.recv().await {
            debug!("serve recv {:?}", recv);
            if let Some(message) = rx.next().await {
                match message.unwrap() {
                    Message::Text(msg) => {
                        recv.send(ReceiveType::Receive(Message::Text(msg))).unwrap();
                    }
                    Message::Binary(msg) => {
                        recv.send(ReceiveType::Receive(Message::Binary(msg)))
                            .unwrap();
                    }
                    Message::Close(msg) => {
                        if let Some(msg) = &msg {
                            recv.send(ReceiveType::Disconnect(u16::from(msg.code)))
                                .unwrap();
                        } else {
                            recv.send(ReceiveType::Disconnect(1005)).unwrap();
                        }
                        break;
                    }
                    _ => {
                        unimplemented!();
                    }
                }
            }
        }
    });

    let send_fut = tokio::spawn(async move {
        while let Some(_type) = send_rx.recv().await {
            debug!("serve send {:?}", _type);
            match _type {
                SendType::Close((code, reason)) => {
                    tx.send(Message::Close(Some(CloseFrame {
                        code: CloseCode::from(code),
                        reason: reason.into(),
                    })))
                    .await
                    .unwrap();
                    break;
                }
                SendType::Send(msg) => tx.send(msg).await.unwrap(),
                _ => error!("Accept again not working"),
            }
        }
    });

    tokio::select! {
        _val = recv_fut => {
            info!("terminated by client");
        }
        _val = send_fut => {
            info!("terminated by server");
        }
    }

    Ok(())
}

pub fn handle(
    app: PyObject,
    locals: TaskLocals,
    addr: SocketAddr,
    mut req: Request<Body>,
) -> FutureResponse {
    let (send_tx, mut send_rx) = unbounded_channel::<SendType>();
    let (recv_tx, mut recv_rx) = unbounded_channel::<Sender<ReceiveType>>();
    let send = SendFunc { tx: send_tx };
    let receive = ReceiveFunc { tx: recv_tx };

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
    let call_app = tokio::spawn(fut);

    Box::pin(async move {
        let result = recv_rx.recv().await;
        let recv_send = result.unwrap();
        recv_send.send(ReceiveType::Connect).unwrap();
        debug!("wait accept");

        let result = send_rx.recv().await;
        let accept_headers = result.unwrap();

        let response = match accept_headers {
            SendType::Accept(headers) => {
                debug!("accept {:?}", headers);
                let (mut response, ws) = hyper_tungstenite::upgrade(&mut req, None).unwrap();

                insert_headers(&mut response, headers).unwrap();
                let serve_fut = serve_ws(ws, send_rx, recv_rx);

                tokio::spawn(async move {
                    if let Err(e) = serve_fut.await {
                        error!("Error in websocket connection: {}", e);
                    }
                    call_app.abort();
                });
                response
            }
            _ => {
                let mut r = Response::default();
                *r.status_mut() = StatusCode::FORBIDDEN;
                r
            }
        };
        Ok(response)
    })
}

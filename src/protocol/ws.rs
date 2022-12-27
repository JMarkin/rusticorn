use std::sync::Arc;

use crate::{configure_scope, prelude::*, server_header};
use anyhow::Error;
use futures::{
    sink::SinkExt,
    stream::{FusedStream, SplitSink, SplitStream},
    StreamExt,
};
use hyper::{Request, StatusCode};
use hyper_tungstenite::{
    tungstenite::{
        protocol::{frame::coding::CloseCode, CloseFrame},
        Message,
    },
    WebSocketStream,
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
    pub rx: AsyncReceiver<ReceiveType>,
}

#[pymethods]
impl ReceiveFunc {
    fn __call__<'a>(&self, py: Python<'a>) -> PyResult<&'a PyAny> {
        let _rx = self.rx.clone();

        // TODO: change to AwaitableObj
        pyo3_asyncio::tokio::future_into_py(py, async move {
            let _type = _rx.recv().await.unwrap();

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
                            dict.set_item("bytes", PyBytes::new(py, message.into_data().as_ref()))?;
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

                if subprotocol.is_some() {
                    let subprotocol = subprotocol.unwrap();

                    if !subprotocol.is_none() {
                        let subprotocol = subprotocol.extract::<String>().unwrap();
                        headers.push(vec![b"sec-websocket-protocol".to_vec(), subprotocol.into()]);
                    }
                }

                self.tx.send(SendType::Accept(headers)).unwrap();
                Ok(())
            }
            "websocket.close" => {
                let mut code = 1000;
                let code_none = scope.get_item("code");
                if let Some(_code) = code_none {
                    code = _code.extract::<u16>().unwrap_or(1000);
                }
                let mut reason = "".to_string();
                let reason_none = scope.get_item("reason");
                if let Some(_reason) = reason_none {
                    reason = _reason.extract::<String>().unwrap_or("".to_string());
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

enum ClosedReason {
    Client,
    Server,
}

async fn receive_loop(
    recv_tx: AsyncSender<ReceiveType>,
    mut rx: SplitStream<WebSocketStream<hyper::upgrade::Upgraded>>,
    tx: UnboundedSender<Message>,
) -> Result<ClosedReason> {
    while let Some(message) = rx.next().await {
        match message? {
            Message::Text(msg) => {
                recv_tx
                    .send(ReceiveType::Receive(Message::Text(msg)))
                    .await
                    .unwrap();
            }
            Message::Binary(msg) => {
                recv_tx
                    .send(ReceiveType::Receive(Message::Binary(msg)))
                    .await
                    .unwrap();
            }
            Message::Close(msg) => {
                if let Some(msg) = &msg {
                    recv_tx
                        .send(ReceiveType::Disconnect(u16::from(msg.code)))
                        .await
                        .unwrap();
                } else {
                    recv_tx.send(ReceiveType::Disconnect(1005)).await.unwrap();
                }
                return Ok(ClosedReason::Client);
            }
            Message::Ping(msg) => {
                tx.send(Message::Pong(msg)).unwrap();
            }
            _ => {}
        }
    }
    Ok(ClosedReason::Server)
}

async fn send_loop(
    mut send_rx: UnboundedReceiver<SendType>,
    tx: UnboundedSender<Message>,
) -> Result<ClosedReason> {
    loop {
        let connected = tx.send(Message::Ping(vec![]));
        if let Err(_e) = connected {
            break;
        }
        if let Ok(_type) = send_rx.try_recv() {
            debug!("serve send {:?}", _type);
            match _type {
                SendType::Close((code, reason)) => {
                    tx.send(Message::Close(Some(CloseFrame {
                        code: CloseCode::from(code),
                        reason: reason.into(),
                    })))
                    .unwrap();
                    return Ok(ClosedReason::Server);
                }
                SendType::Send(msg) => tx.send(msg).unwrap(),
                _ => error!("Accept again not working"),
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_micros(100)).await;
    }
    Ok(ClosedReason::Client)
}

async fn serve_ws(
    ws: HyperWebsocket,
    send_rx: UnboundedReceiver<SendType>,
    recv_tx: AsyncSender<ReceiveType>,
) -> Result<()> {
    let websocket = ws.await?;
    let (mut tx, rx) = websocket.split();
    let (message_tx, mut message_rx) = unbounded_channel::<Message>();

    let recv_fut = tokio::spawn(receive_loop(recv_tx, rx, message_tx.clone()));

    let send_fut = tokio::spawn(send_loop(send_rx, message_tx.clone()));

    let sender_fut = tokio::spawn(async move {
        while let Some(msg) = message_rx.recv().await {
            let connected = tx.send(msg).await;
            if let Err(e) = connected {
                info!("close websocket {}", e.to_string());
                break;
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
        _val = sender_fut => {
            info!("terminated");
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
    let (recv_tx, mut recv_rx) = unbounded::<ReceiveType>();
    let send = SendFunc { tx: send_tx };
    let receive = ReceiveFunc { rx: recv_rx };

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
        recv_tx.send(ReceiveType::Connect).await.unwrap();
        debug!("wait accept");

        let accept = send_rx.recv().await.unwrap();

        let mut response = match accept {
            SendType::Accept(headers) => {
                debug!("accept {:?}", headers);
                let (mut response, ws) = hyper_tungstenite::upgrade(&mut req, None).unwrap();

                insert_headers(&mut response, headers).unwrap();
                let serve_fut = serve_ws(ws, send_rx, recv_tx);

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
        (*response.headers_mut()).append("server", server_header!());
        Ok(response)
    })
}

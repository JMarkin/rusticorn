use std::{
    cell::RefCell,
    sync::{Arc, RwLock},
    time::SystemTime,
};

use crate::{prelude::*, server_header};
use anyhow::Error;
use futures::{stream::SplitStream, SinkExt, StreamExt};
use hyper::{Request, StatusCode};
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    Mutex,
};
use tungstenite::{
    protocol::{frame::coding::CloseCode, CloseFrame},
    Message,
};

use super::common::insert_headers;

const DISCONENCT_CODE: u16 = 1005;

#[derive(Debug)]
enum ASGIMessage {
    Text(String),
    Binary(Vec<u8>),
    Close(Option<CloseFrame<'static>>),
}

#[derive(Debug)]
enum ReceiveEvent {
    Connect,
    Message(ASGIMessage),
}

#[pyclass]
struct Receiver {
    ws: Arc<Mutex<UnboundedReceiver<ReceiveEvent>>>,
}

fn disconnect_evnet(close_frame: Option<CloseFrame<'static>>) -> PyObject {
    Python::with_gil(|py| {
        let _d = PyDict::new(py);
        _d.set_item("type", ScopeType::WsDisconenct.into_py(py));
        _d.set_item(
            "code",
            close_frame.map_or(DISCONENCT_CODE, |v| u16::from(v.code)),
        );
        _d.into()
    })
}

#[pymethods]
impl Receiver {
    fn __call__(&mut self) -> PyAsync<Result<PyObject>> {
        debug!("ws: recv called");
        let ws = self.ws.clone();
        async move {
            let event = ws.lock().await.recv().await;
            if event.is_none() {
                return Ok(disconnect_evnet(None));
            }

            match event.unwrap() {
                ReceiveEvent::Connect => Ok(Python::with_gil(|py| {
                    let _d = PyDict::new(py);
                    _d.set_item("type", ScopeType::WsConnect.into_py(py));
                    _d.into()
                })),
                ReceiveEvent::Message(msg) => match msg {
                    ASGIMessage::Text(txt) => Ok(Python::with_gil(|py| {
                        let _d = PyDict::new(py);
                        _d.set_item("type", ScopeType::WsReceive.into_py(py));
                        _d.set_item("text", txt);
                        _d.into()
                    })),
                    ASGIMessage::Binary(bin) => Ok(Python::with_gil(|py| {
                        let _d = PyDict::new(py);
                        _d.set_item("type", ScopeType::WsReceive.into_py(py));
                        _d.set_item("bytes", bin);
                        _d.into()
                    })),
                    ASGIMessage::Close(frame) => Ok(disconnect_evnet(frame)),
                },
            }
        }
        .into()
    }
}

async fn receive_loop(
    cfg: &'static Config,
    mut ws: SplitStream<WebSocketStream<hyper::upgrade::Upgraded>>,
    recv_tx: UnboundedSender<ReceiveEvent>,
    ws_tx: UnboundedSender<Message>,
) -> Result<()> {
    while let Some(message) = ws.next().await {
        let message = message?;

        if message.is_ping() {
            let msg = message.into_data();
            ws_tx.send(Message::Pong(msg)).unwrap();
            continue;
        }

        if message.is_pong() {
            let msg = message.into_data();
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let started = u128::from_le_bytes(msg[0..16].try_into().unwrap());
            let timeout: u128 = (cfg.ws.ping_timeout * 1000.0) as u128;
            if now - started > timeout {
            }
            continue;
        }

        match message {
            Message::Text(msg) => recv_tx.send(ReceiveEvent::Message(ASGIMessage::Text(msg)))?,
            Message::Binary(msg) => {
                recv_tx.send(ReceiveEvent::Message(ASGIMessage::Binary(msg)))?
            }
            Message::Close(msg) => recv_tx.send(ReceiveEvent::Message(ASGIMessage::Close(msg)))?,
            _ => {}
        }
    }
    Ok(())
}

#[derive(Debug)]
enum SendEvent {
    Accept(Headers),
    Message(ASGIMessage),
}

#[pyclass]
struct Sender {
    ws: UnboundedSender<SendEvent>,
}

#[pymethods]
impl Sender {
    fn __call__(&self, scope: &PyDict) -> PyAsync<Result<()>> {
        debug!("send called {:?}", scope);
        let _type = scope
            .get_item("type")
            .expect("type required")
            .extract::<ScopeType>()
            .unwrap();

        let result: Result<()> = match _type {
            ScopeType::WsAccept => {
                let subprotocol = scope.get_item("subprotocol");
                let mut headers = scope
                    .get_item("headers")
                    .unwrap()
                    .extract::<Headers>()
                    .unwrap();

                if subprotocol.is_some() {
                    let subprotocol = subprotocol.unwrap();

                    if !subprotocol.is_none() {
                        let subprotocol = subprotocol.extract::<String>().unwrap();
                        headers.push(vec![b"sec-websocket-protocol".to_vec(), subprotocol.into()]);
                    }
                }

                self.ws.send(SendEvent::Accept(headers)).unwrap();
                Ok(())
            }
            ScopeType::WsSend => {
                let bytes = scope.get_item("bytes");
                let text = scope.get_item("text");
                if bytes.is_none() && text.is_none() {
                    panic!("bytes or text must exists");
                }

                if let Some(bytes) = bytes {
                    self.ws
                        .send(SendEvent::Message(ASGIMessage::Binary(
                            bytes.extract::<Vec<u8>>().unwrap(),
                        )))
                        .unwrap();
                }

                if let Some(text) = text {
                    self.ws
                        .send(SendEvent::Message(ASGIMessage::Text(
                            text.extract::<String>().unwrap(),
                        )))
                        .unwrap();
                }
                Ok(())
            }
            ScopeType::WsClose => {
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
                self.ws
                    .send(SendEvent::Message(ASGIMessage::Close(Some(CloseFrame {
                        code: CloseCode::from(code),
                        reason: reason.into(),
                    }))))
                    .unwrap();
                Ok(())
            }
            _ => panic!(
                "unsupported type {}; available {} {} {}",
                _type,
                ScopeType::WsAccept,
                ScopeType::WsSend,
                ScopeType::WsClose,
            ),
        };
        async {
            match result {
                Ok(_) => Ok(()),
                Err(e) => {
                    error!("{}", e);
                    Ok(())
                }
            }
        }
        .into()
    }
}

async fn send_loop(
    mut send_rx: UnboundedReceiver<SendEvent>,
    ws_tx: UnboundedSender<Message>,
) -> Result<()> {
    while let Some(message) = send_rx.recv().await {
        match message {
            SendEvent::Message(amsg) => match amsg {
                ASGIMessage::Text(msg) => ws_tx.send(Message::Text(msg))?,
                ASGIMessage::Binary(msg) => ws_tx.send(Message::Binary(msg))?,
                ASGIMessage::Close(msg) => ws_tx.send(Message::Close(msg))?,
                _ => {}
            },
            _ => return Err(anyhow!("called accept twice",)),
        };
    }
    Ok(())
}

async fn ping_loop(cfg: &'static Config, ws: UnboundedSender<Message>) -> Result<()> {
    loop {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        ws.send(Message::Ping(now.to_le_bytes().to_vec()))?;
        tokio::time::sleep(tokio::time::Duration::from_secs_f32(cfg.ws.ping_interval)).await;
    }
}

async fn serve_ws(
    cfg: &'static Config,
    ws: HyperWebsocket,
    send_rx: UnboundedReceiver<SendEvent>,
    recv_tx: UnboundedSender<ReceiveEvent>,
) -> Result<()> {
    let websocket = ws.await?;
    let (mut ws_tx, ws_rx) = websocket.split();
    let (message_tx, mut message_rx) = unbounded_channel::<Message>();

    let recv_fut = tokio::spawn(receive_loop(ws_rx, recv_tx, message_tx.clone()));

    let send_fut = tokio::spawn(send_loop(send_rx, message_tx.clone()));

    let ping_fut = tokio::spawn(ping_loop(cfg, message_tx.clone()));

    let sender_fut = tokio::spawn(async move {
        while let Some(msg) = message_rx.recv().await {
            let connected = ws_tx.send(msg).await;
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
        _val = ping_fut => {
            info!("terminated");
        }
    }

    Ok(())
}

pub fn handle(
    cfg: &'static Config,
    addr: SocketAddr,
    mut req: Request<IncomingBody>,
    tx: UnboundedSender<ScopeRecvSend>,
) -> FutureResponse {
    let (send_tx, mut send_rx) = unbounded_channel();
    let (recv_tx, recv_rx) = unbounded_channel();

    let sender = Sender { ws: send_tx };

    let upgrade_result = upgrade(&mut req, None);

    let (parts, _body) = req.into_parts();
    let receiver = Receiver {
        ws: Arc::new(Mutex::new(recv_rx)),
    };

    let scope = Scope::new(parts, addr, ScopeType::Ws);
    debug!("{:?}", scope);

    let args = Python::with_gil(|py| (scope.into_py(py), receiver.into_py(py), sender.into_py(py)));

    Box::pin(async move {
        let result = tx.send(Some(args));
        if let Err(e) = result {
            return Ok(HttpError::internal_error(e.into()));
        }

        let result = recv_tx.send(ReceiveEvent::Connect);
        if let Err(e) = result {
            return Ok(HttpError::internal_error(e.into()));
        }

        debug!("wait accept");
        let accept = send_rx.recv().await;
        if accept.is_none() {
            return Ok(HttpError::forbidden("not accept"));
        }
        match accept.unwrap() {
            SendEvent::Accept(headers) => {
                if let Err(e) = upgrade_result {
                    return Ok(HttpError::internal_error(e.into()));
                }

                let (mut response, ws) = upgrade_result.unwrap();

                insert_headers(&mut response, headers).unwrap();
                (*response.headers_mut()).append("server", server_header!());
                Ok(response)
            }
            _ => Ok(HttpError::forbidden("not accept")),
        }
    })
}

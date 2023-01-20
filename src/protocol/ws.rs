use std::{sync::Arc, time::SystemTime};

use crate::{prelude::*, server_header};
use futures::{stream::SplitStream, SinkExt, StreamExt};
use hyper::Request;
use once_cell::sync::OnceCell;
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

static PY_RECEIVER: &str = "
import asyncio

def create_recv(recv):
    async def f():
        while 1:
            resp = await recv()
            if resp:
                return resp
            await asyncio.sleep(0.001)
    return f
";

fn create_recv(py: Python) -> PyResult<&PyObject> {
    static CREATE_RECV: OnceCell<PyObject> = OnceCell::new();
    CREATE_RECV.get_or_try_init(|| {
        let create_recv: PyObject = PyModule::from_code(py, PY_RECEIVER, "", "")?
            .getattr("create_recv")?
            .into();

        Ok(create_recv)
    })
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

fn disconnect_event(close_frame: Option<CloseFrame<'static>>) -> Result<PyObject> {
    Python::with_gil(|py| {
        let _d = PyDict::new(py);
        _d.set_item("type", ScopeType::WsDisconenct.into_py(py))?;
        _d.set_item(
            "code",
            close_frame.map_or(DISCONENCT_CODE, |v| u16::from(v.code)),
        )?;
        Ok(_d.into())
    })
}

#[pymethods]
impl Receiver {
    fn __call__(&mut self) -> PyAsync<Result<PyObject>> {
        let ws = self.ws.clone();
        async move {
            let event = ws.lock().await.recv().await;
            if event.is_none() {
                return Python::with_gil(|py| Ok(().into_py(py)));
            }

            let event = event.unwrap();
            debug!("ws: recv event {:?}", event);
            match event {
                ReceiveEvent::Connect => Python::with_gil(|py| {
                    let _d = PyDict::new(py);
                    _d.set_item("type", ScopeType::WsConnect.into_py(py))?;
                    Ok(_d.into())
                }),
                ReceiveEvent::Message(msg) => match msg {
                    ASGIMessage::Text(txt) => Python::with_gil(|py| {
                        let _d = PyDict::new(py);
                        _d.set_item("type", ScopeType::WsReceive.into_py(py))?;
                        _d.set_item("text", txt)?;
                        Ok(_d.into())
                    }),
                    ASGIMessage::Binary(bin) => Python::with_gil(|py| {
                        let _d = PyDict::new(py);
                        _d.set_item("type", ScopeType::WsReceive.into_py(py))?;
                        _d.set_item("bytes", bin)?;
                        Ok(_d.into())
                    }),
                    ASGIMessage::Close(frame) => disconnect_event(frame),
                },
            }
        }
        .into()
    }
}

async fn receive_loop(
    ping_timeout: f32,
    mut ws: SplitStream<WebSocketStream<hyper::upgrade::Upgraded>>,
    recv_tx: UnboundedSender<ReceiveEvent>,
    ws_tx: UnboundedSender<Message>,
) -> Result<()> {
    let timeout: u128 = (ping_timeout * 1000.0) as u128;
    while let Some(message) = ws.next().await {
        let message = message?;

        if message.is_ping() {
            let msg = message.into_data();
            ws_tx.send(Message::Pong(msg)).unwrap();
            continue;
        }

        match message {
            Message::Pong(msg) => {
                let now = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                let started = u128::from_le_bytes(msg[0..16].try_into().unwrap());
                if now - started > timeout {
                    return Err(anyhow!("pong receive {} > {}: timeout", now, started));
                }
            }
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

        let result: Result<()> = (|| match _type {
            ScopeType::WsAccept => {
                let subprotocol = scope.get_item("subprotocol");
                let mut headers = scope.get_item("headers").unwrap().extract::<Headers>()?;

                if let Some(subprotocol) = subprotocol {
                    if !subprotocol.is_none() {
                        let subprotocol = subprotocol.extract::<String>()?;
                        headers.push(vec![b"sec-websocket-protocol".to_vec(), subprotocol.into()]);
                    }
                }

                self.ws.send(SendEvent::Accept(headers))?;
                Ok(())
            }
            ScopeType::WsSend => {
                let bytes = scope.get_item("bytes");
                let text = scope.get_item("text");
                if bytes.is_none() && text.is_none() {
                    return Err(anyhow!("bytes or text must exists"));
                }

                if let Some(bytes) = bytes {
                    self.ws.send(SendEvent::Message(ASGIMessage::Binary(
                        bytes.extract::<Vec<u8>>()?,
                    )))?;
                }

                if let Some(text) = text {
                    self.ws.send(SendEvent::Message(ASGIMessage::Text(
                        text.extract::<String>()?,
                    )))?;
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
                    reason = _reason.extract::<String>().unwrap_or(reason);
                }
                debug!("close websocket code: {} reason: {}", code, reason);
                self.ws
                    .send(SendEvent::Message(ASGIMessage::Close(Some(CloseFrame {
                        code: CloseCode::from(code),
                        reason: reason.into(),
                    }))))?;
                Ok(())
            }
            _ => Err(anyhow!(
                "unsupported type {}; available {} {} {}",
                _type,
                ScopeType::WsAccept,
                ScopeType::WsSend,
                ScopeType::WsClose,
            )),
        })();
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
        debug!("try send {:?} in send loop", message);
        match message {
            SendEvent::Message(amsg) => match amsg {
                ASGIMessage::Text(msg) => ws_tx.send(Message::Text(msg))?,
                ASGIMessage::Binary(msg) => ws_tx.send(Message::Binary(msg))?,
                ASGIMessage::Close(msg) => ws_tx.send(Message::Close(msg))?,
            },
            _ => return Err(anyhow!("called accept twice",)),
        };
    }
    Ok(())
}

async fn ping_loop(ping_interval: f32, ws: UnboundedSender<Message>) -> Result<()> {
    loop {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        ws.send(Message::Ping(now.to_le_bytes().to_vec()))?;
        tokio::time::sleep(tokio::time::Duration::from_secs_f32(ping_interval)).await;
    }
}

async fn serve_ws(
    ws: HyperWebsocket,
    send_rx: UnboundedReceiver<SendEvent>,
    recv_tx: UnboundedSender<ReceiveEvent>,
    ping_timeout: f32,
    ping_interval: f32,
) -> Result<()> {
    let websocket = ws.await?;
    let (mut ws_tx, ws_rx) = websocket.split();
    let (message_tx, mut message_rx) = unbounded_channel::<Message>();

    let sender_fut = tokio::task::spawn_local(async move {
        while let Some(msg) = message_rx.recv().await {
            debug!("try send {}", msg);
            let connected = ws_tx.send(msg).await;
            if let Err(e) = connected {
                info!("close websocket {}", e.to_string());
                break;
            }
        }
    });

    let recv_fut = tokio::task::spawn_local(receive_loop(
        ping_timeout,
        ws_rx,
        recv_tx,
        message_tx.clone(),
    ));

    let send_fut = tokio::task::spawn_local(send_loop(send_rx, message_tx.clone()));

    let ping_fut = tokio::task::spawn_local(ping_loop(ping_interval, message_tx.clone()));

    tokio::select! {
        _val = recv_fut => {
            info!("terminated by client");
        }
        _val = send_fut => {
            info!("terminated by server");
        }
        _val = sender_fut => {
            info!("terminated by sender");
        }
        _val = ping_fut => {
            info!("terminated by ping");
        }
    }

    Ok(())
}

pub fn handle(
    addr: SocketAddr,
    mut req: Request<IncomingBody>,
    tx: UnboundedSender<ScopeRecvSend>,
    ping_timeout: f32,
    ping_interval: f32,
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

    let args = Python::with_gil(|py| {
        let recv = create_recv(py)
            .unwrap()
            .call1(py, (receiver.into_py(py),))
            .unwrap();
        (scope.into_py(py), recv, sender.into_py(py))
    });

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
                debug!("accepted");
                tokio::task::spawn_local(serve_ws(
                    ws,
                    send_rx,
                    recv_tx,
                    ping_timeout,
                    ping_interval,
                ));

                insert_headers(&mut response, headers).unwrap();
                (*response.headers_mut()).append("server", server_header!());
                Ok(response)
            }
            _ => Ok(HttpError::forbidden("not accept")),
        }
    })
}

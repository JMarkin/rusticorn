use crate::prelude::*;
use crate::protocol::http::ReceiveRequest;
use crate::protocol::http::ReceiveTypes;
use crate::protocol::http::SendStart;
use crate::protocol::http::SendTypes;
use hyper::body::HttpBody;
use hyper::server::conn::AddrStream;
use hyper::service::make_service_fn;
use hyper::service::service_fn;
use hyper::Body;
use hyper::Request;
use hyper::Response;
use hyper::Server;
use hyper::StatusCode;
use std::convert::Infallible;
use std::sync::Arc;

use crate::scope::configure_scope;

#[derive(Debug)]
pub struct ASGIRequest {
    pub send: AsyncSender<SendTypes>,
    pub receive: (AsyncReceiver<ReceiveTypes>, AsyncSender<bool>),
    pub scope: Py<PyAny>,
}

fn response_start<T>(resp: &mut Response<T>, send_start: SendStart) -> Result<()> {
    *resp.status_mut() = StatusCode::from_u16(send_start.status).unwrap();
    for header in send_start.headers {
        let name = header.first().unwrap();
        let value = header.last().unwrap();
        let name = hyper::http::header::HeaderName::from_bytes(name.as_slice())?;
        let value = hyper::http::header::HeaderValue::from_bytes(value.as_slice())?;

        resp.headers_mut().insert(name, value);
    }
    Ok(())
}

async fn call(
    req_tx: Arc<AsyncSender<ASGIRequest>>,
    addr: SocketAddr,
    req: Request<Body>,
) -> Result<Response<Body>, Infallible> {
    let mut response = Response::default();
    let mut buf: Vec<u8> = vec![];
    let (send_tx, send_rx) = unbounded::<SendTypes>();
    let (recv_tx, recv_rx) = unbounded::<ReceiveTypes>();
    let (recv_enabled_tx, recv_enabled_rx) = unbounded::<bool>();

    let scope = Python::with_gil(|py| configure_scope(py, &req, addr).unwrap().to_object(py));
    req_tx
        .send(ASGIRequest {
            scope,
            send: send_tx,
            receive: (recv_rx, recv_enabled_tx),
        })
        .await
        .unwrap();

    let wait_parse_request = tokio::spawn(async move {
        let mut chunk_stream = req.into_body();

        while recv_enabled_rx.recv().await.is_ok() {
            let chunk = chunk_stream.data().await;
            if let Some(chunk) = chunk {
                match chunk {
                    Err(e) => {
                        warn!("{}", e.to_string());
                        recv_tx.send(ReceiveTypes::HttpDisconect).await.unwrap();
                        break;
                    }
                    Ok(data) => {
                        let more_body = !chunk_stream.is_end_stream();
                        recv_tx
                            .send(ReceiveTypes::HttpRequst(ReceiveRequest {
                                body: data.into(),
                                more_body,
                            }))
                            .await
                            .unwrap();
                    }
                }
            } else {
                debug!("end request body");
                break;
            }
        }
    });

    while let Ok(_type) = send_rx.recv().await {
        debug!("{:?}", _type);

        let result = match _type {
            SendTypes::HttpResponseStart(send_start) => response_start(&mut response, send_start),
            SendTypes::HttpResponseBody(mut send_body) => {
                buf.append(&mut send_body.body);
                if !send_body.more_body {
                    debug!("End response body");
                    *response.body_mut() = buf.into();
                    break;
                }
                Ok(())
            }
        };

        if let Err(e) = result {
            response = HttpError::internal_error(e);
            break;
        }
    }

    wait_parse_request.abort();
    Ok(response)
}

pub async fn start_server(tx: AsyncSender<ASGIRequest>, addr: SocketAddr) {
    let atx = Arc::new(tx);
    let make_svc = make_service_fn(move |conn: &AddrStream| {
        let tx = atx.clone();
        let addr = conn.remote_addr();
        let service = service_fn(move |req| call(tx.clone(), addr, req));
        async move { Ok::<_, Infallible>(service) }
    });

    let server = Server::bind(&addr).serve(make_svc);

    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
}

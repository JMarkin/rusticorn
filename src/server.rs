use std::io;
use std::sync::Arc;

use crate::prelude::*;
use crate::service::load_certs;
use crate::service::load_private_key;
use crate::service::Acceptor;
use crate::service::MakeSvc;
use hyper::server::conn::AddrIncoming;
use hyper::Server;

pub async fn start_server(
    locals: TaskLocals,
    app: PyObject,
    addr: SocketAddr,
    tls: bool,
    cert_path: Option<String>,
    private_path: Option<String>,
    http_version: HttpVersion,
) -> Result<()> {
    let mut tls_cfg = None;
    if tls {
        tls_cfg = Some({
            let certs = load_certs(cert_path.unwrap().as_str())?;
            let key = load_private_key(private_path.unwrap().as_str())?;
            let mut cfg = rustls::ServerConfig::builder()
                .with_safe_defaults()
                .with_no_client_auth()
                .with_single_cert(certs, key)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("{}", e)))?;
            // Configure ALPN to accept HTTP/2, HTTP/1.1 in that order.
            cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
            Arc::new(cfg)
        });
    }

    let incoming = AddrIncoming::bind(&addr)?;
    let server = Server::builder(Acceptor::new(tls_cfg, incoming));

    let server = match http_version {
        HttpVersion::HTTP1 => server.http1_only(true),
        HttpVersion::HTTP2 => server.http2_only(true),
        HttpVersion::Any => server,
    };
    let server = server.serve(MakeSvc::new(app, locals));

    server.await?;

    Ok(())
}

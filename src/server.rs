use std::fs;
use std::sync::Arc;

use crate::prelude::*;
use crate::service::Acceptor;
use crate::service::MakeSvc;
use hyper::server::conn::AddrIncoming;
use hyper::Server;
use rustls::Certificate;
use rustls::PrivateKey;
use rustls::ServerConfig;

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
            let cert_pem = fs::read(cert_path.unwrap().as_str())?;
            let key_pem = fs::read(private_path.unwrap().as_str())?;

            let certs: Vec<Certificate> = rustls_pemfile::certs(&mut &*cert_pem)
                .map(|mut certs| certs.drain(..).map(Certificate).collect())?;
            if certs.is_empty() {
                return Err(anyhow!("No certificates found."));
            }

            let mut keys: Vec<PrivateKey> = rustls_pemfile::pkcs8_private_keys(&mut &*key_pem)
                .map(|mut keys| keys.drain(..).map(PrivateKey).collect())?;
            if keys.is_empty() {
                return Err(anyhow!("No private keys found."));
            }

            let mut tls_config = ServerConfig::builder()
                .with_safe_defaults()
                .with_no_client_auth()
                .with_single_cert(certs, keys.remove(0))?;

            tls_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
            Arc::new(tls_config)
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

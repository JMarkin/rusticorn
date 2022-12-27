use crate::prelude::*;
use hyper::{
    header::{HOST, SEC_WEBSOCKET_PROTOCOL},
    Body, Request, Version,
};
use log::debug;
use std::collections::HashMap;
use urlencoding::decode;

pub fn configure_scope<'a>(
    _type: ScopeType,
    py: Python<'a>,
    req: &Request<Body>,
    addr: SocketAddr,
) -> PyResult<&'a PyDict> {
    let mut scope: HashMap<&str, &str> = HashMap::from([
        ("version", "3.0"),
        ("spec_version", "2.3"),
        (
            "http_version",
            match req.version() {
                Version::HTTP_10 => "1.0",
                Version::HTTP_11 => "1.1",
                Version::HTTP_2 => "2.0",
                v => panic!("Unsupported http version {:?}", v),
            },
        ),
    ]);
    let method = req.method().as_str().to_uppercase();
    if _type == ScopeType::Http {
        scope.insert("method", &method);
    }

    debug!("{:?}", req.uri());
    if _type == ScopeType::Http {
        scope.insert("scheme", req.uri().scheme_str().unwrap_or("http"));
    } else {
        scope.insert("scheme", req.uri().scheme_str().unwrap_or("ws"));
    }
    scope.insert("root_path", "");
    let path: String = decode(req.uri().path()).expect("UTF-8").into();
    let raw_path = req.uri().path().as_bytes();
    let query_string = match req.uri().query() {
        Some(s) => decode(s).expect("UTF-8").to_string(),
        None => "".to_string(),
    };
    let client = (addr.ip().to_string(), addr.port());

    let mut headers = vec![];

    if let Some(auth) = req.uri().authority() {
        headers.push((HOST.as_str().as_bytes(), auth.as_str().as_bytes()));
    }

    for (key, val) in req.headers().iter() {
        if _type == ScopeType::Ws && key == SEC_WEBSOCKET_PROTOCOL {
            continue;
        }
        headers.push((key.as_str().as_bytes(), val.as_bytes()));
    }

    let dict = scope.into_py_dict(py);

    if _type == ScopeType::Ws {
        let mut subprotocols = vec![];
        for val in req.headers().get_all(SEC_WEBSOCKET_PROTOCOL) {
            let mut s = val.to_str().unwrap().split(',').collect::<Vec<&str>>();
            subprotocols.append(&mut s);
        }
        dict.set_item("subprotocols", subprotocols.into_py(py))?;
    }

    dict.set_item("headers", headers.into_py(py))?;
    dict.set_item("client", (client.0.into_py(py), client.1.into_py(py)))?;
    dict.set_item("path", path.into_py(py))?;
    dict.set_item("raw_path", raw_path.into_py(py))?;
    dict.set_item("query_string", query_string.as_bytes().into_py(py))?;
    dict.set_item("type", _type.into_py(py))?;

    debug!("scope {dict}");
    Ok(dict)
}

use crate::{prelude::*, server_header};
use anyhow::Error;
use hyper::{body::Bytes, Response};
use log::error;

pub struct HttpError;

impl HttpError {
    pub fn internal_error(e: Error) -> Response<HttpBody<Bytes>> {
        error!("{}", e.to_string());
        let mut response = Response::builder()
            .status(500)
            .body("Internal Server Error".into())
            .unwrap();
        (*response.headers_mut()).append("server", server_header!());
        response
    }
}

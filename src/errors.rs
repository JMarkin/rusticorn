use anyhow::Error;
use hyper::{Body, Response};
use log::error;

pub struct HttpError;

impl HttpError {
    pub fn internal_error(e: Error) -> Response<Body> {
        error!("{}", e.to_string());
        Response::builder()
            .status(500)
            .body("Internal Server Error".into())
            .unwrap()
    }
}

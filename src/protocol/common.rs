use crate::prelude::*;
use std::mem;

pub fn insert_headers(resp: &mut ServiceResponse, mut headers: Headers) -> Result<()> {
    for header in mem::take(&mut headers) {
        let name = header.first().unwrap();
        let value = header.last().unwrap();
        let name = hyper::http::header::HeaderName::from_bytes(name.as_slice())?;
        let value = hyper::http::header::HeaderValue::from_bytes(value.as_slice())?;

        resp.headers_mut().insert(name, value);
    }
    Ok(())
}

use crate::prelude::*;

pub fn insert_headers(resp: &mut Response<Body>, headers: Headers) -> Result<()> {
    for header in headers {
        let name = header.first().unwrap();
        let value = header.last().unwrap();
        let name = hyper::http::header::HeaderName::from_bytes(name.as_slice())?;
        let value = hyper::http::header::HeaderValue::from_bytes(value.as_slice())?;

        resp.headers_mut().insert(name, value);
    }
    Ok(())
}

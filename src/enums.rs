use std::str::FromStr;

#[derive(Debug, Clone)]
pub enum HttpVersion {
    Any,
    HTTP1,
    HTTP2,
}

impl FromStr for HttpVersion {
    type Err = ();

    fn from_str(input: &str) -> Result<HttpVersion, Self::Err> {
        match input {
            "any" => Ok(HttpVersion::Any),
            "http1" => Ok(HttpVersion::HTTP1),
            "http2" => Ok(HttpVersion::HTTP2),
            _ => Err(()),
        }
    }
}

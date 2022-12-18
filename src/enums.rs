use std::str::FromStr;

#[derive(Debug, Clone)]
pub enum HttpVersion {
    ANY,
    HTTP1,
    HTTP2,
}

impl FromStr for HttpVersion {
    type Err = ();

    fn from_str(input: &str) -> Result<HttpVersion, Self::Err> {
        match input {
            "any" => Ok(HttpVersion::ANY),
            "http1" => Ok(HttpVersion::HTTP1),
            "http2" => Ok(HttpVersion::HTTP2),
            _ => Err(()),
        }
    }
}

use crate::prelude::*;
use hyper::{
    header::{HOST, SEC_WEBSOCKET_PROTOCOL},
    http::request::Parts,
    Version,
};
use urlencoding::decode;

#[pyclass(mapping)]
#[derive(Debug)]
pub struct Scope {
    pub req: Parts,
    pub addr: SocketAddr,
    pub _type: ScopeType,
    pub _dict: PyObject,
}

const SCOPE_KEYS: [&str; 11] = [
    "version",
    "spec_version",
    "http_version",
    "method",
    "scheme",
    "root_path",
    "headers",
    "client",
    "path",
    "raw_path",
    "query_string",
];

#[pymethods]
impl Scope {
    fn version(slf: PyRef<Self>) -> Result<PyObject> {
        Ok("3.0".into_py(slf.py()))
    }

    fn spec_version(slf: PyRef<Self>) -> Result<PyObject> {
        Ok("2.3".into_py(slf.py()))
    }

    fn http_version(slf: PyRef<Self>) -> Result<PyObject> {
        Ok(match slf.req.version {
            Version::HTTP_10 => "1.0",
            Version::HTTP_11 => "1.1",
            Version::HTTP_2 => "2.0",
            v => panic!("Unsupported http version {:?}", v),
        }
        .into_py(slf.py()))
    }

    fn method(slf: PyRef<Self>) -> Result<PyObject> {
        Ok(match slf._type {
            ScopeType::Http => Some(slf.req.method.as_str().to_uppercase()),
            _ => None,
        }
        .into_py(slf.py()))
    }

    fn scheme(slf: PyRef<Self>) -> Result<PyObject> {
        let scheme = slf.req.uri.scheme_str();

        Ok(match scheme {
            Some(v) => v.into_py(slf.py()),
            None => match slf._type {
                ScopeType::Http => "http",
                ScopeType::Ws => "ws",
                _ => panic!("failed to parse scheme"),
            }
            .into_py(slf.py()),
        })
    }

    fn root_path(slf: PyRef<Self>) -> Result<PyObject> {
        Ok("".into_py(slf.py()))
    }

    fn headers(slf: PyRef<Self>) -> Result<PyObject> {
        let mut headers = vec![];

        if let Some(auth) = slf.req.uri.authority() {
            headers.push((HOST.as_str().as_bytes(), auth.as_str().as_bytes()));
        }

        for (key, val) in slf.req.headers.iter() {
            if slf._type == ScopeType::Ws && key == SEC_WEBSOCKET_PROTOCOL {
                continue;
            }
            headers.push((key.as_str().as_bytes(), val.as_bytes()));
        }

        Ok(headers.into_py(slf.py()))
    }

    fn client(slf: PyRef<Self>) -> Result<PyObject> {
        Ok((slf.addr.ip().to_string(), slf.addr.port()).into_py(slf.py()))
    }

    fn path(slf: PyRef<Self>) -> Result<PyObject> {
        Ok(decode(slf.req.uri.path()).expect("UTF-8").into_py(slf.py()))
    }

    fn raw_path(slf: PyRef<Self>) -> Result<PyObject> {
        Ok(slf.req.uri.path().as_bytes().into_py(slf.py()))
    }

    fn query_string(slf: PyRef<Self>) -> Result<PyObject> {
        Ok(match slf.req.uri.query() {
            Some(s) => decode(s).expect("UTF-8").to_string(),
            None => "".to_string(),
        }
        .into_py(slf.py()))
    }

    fn __contains__(slf: PyRef<Self>, key: &str) -> PyResult<bool> {
        let mut exists = SCOPE_KEYS.iter().any(|&x| x == key);

        if !exists {
            exists = slf._dict.cast_as::<PyDict>(slf.py())?.contains(key)?;
        }

        Ok(exists)
    }

    fn get(slf: PyRef<Self>, key: &str, value: PyObject) -> Result<PyObject> {
        Ok(Scope::__getitem__(slf, key)?.unwrap_or(value))
    }

    fn __getitem__(slf: PyRef<Self>, key: &str) -> Result<Option<PyObject>> {
        Ok(match key {
            "version" => Some(Scope::version(slf)?),
            "spec_version" => Some(Scope::spec_version(slf)?),
            "http_version" => Some(Scope::http_version(slf)?),
            "method" => Some(Scope::method(slf)?),
            "scheme" => Some(Scope::scheme(slf)?),
            "root_path" => Some(Scope::root_path(slf)?),
            "headers" => Some(Scope::headers(slf)?),
            "client" => Some(Scope::client(slf)?),
            "path" => Some(Scope::path(slf)?),
            "raw_path" => Some(Scope::raw_path(slf)?),
            "query_string" => Some(Scope::query_string(slf)?),
            "type" => Some(slf._type.into_py(slf.py()).into()),
            _ => match slf._dict.cast_as::<PyDict>(slf.py()) {
                Ok(_d) => _d.get_item(key).map(|v| v.into()),
                Err(_) => None,
            },
        })
    }

    fn __setitem__(slf: PyRefMut<Self>, key: &str, value: PyObject) -> PyResult<()> {
        slf._dict.cast_as::<PyDict>(slf.py())?.set_item(key, value)
    }

    fn __delitem__(slf: PyRefMut<Self>, key: &str) -> PyResult<()> {
        slf._dict.cast_as::<PyDict>(slf.py())?.del_item(key)
    }

    fn update(slf: PyRefMut<Self>, val: PyObject) -> PyResult<()> {
        let d = slf._dict.cast_as::<PyDict>(slf.py())?;
        d.call_method1("update", (val,))?;
        Ok(())
    }

    fn __str__(&self) -> Result<PyObject> {
        Ok(Python::with_gil(|py| {
            format!("{:?} {:?}", self, self._dict.cast_as::<PyDict>(py)).into_py(py)
        }))
    }
    fn __repr__(&self) -> Result<PyObject> {
        self.__str__()
    }

}

#[pyclass]
struct IterScope {
    inner: std::vec::IntoIter<String>,
}

#[pymethods]
impl IterScope {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>) -> Option<String> {
        slf.inner.next()
    }
}

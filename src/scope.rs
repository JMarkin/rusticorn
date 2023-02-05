use crate::prelude::*;
use hyper::{
    header::{HOST, SEC_WEBSOCKET_PROTOCOL},
    http::request::Parts,
    Version,
};
use once_cell::sync::OnceCell;
use urlencoding::decode;

#[pyclass(mapping, module = "rusticorn")]
#[derive(Debug)]
pub struct Scope {
    pub parts: Parts,
    pub addr: SocketAddr,
    pub _type: ScopeType,
    pub _dict: PyObject,
}

impl Scope {
    pub fn new(parts: Parts, addr: SocketAddr, _type: ScopeType) -> Self {
        Self {
            parts,
            addr,
            _type,
            _dict: Python::with_gil(|py| PyDict::new(py).into()),
        }
    }
}

const SCOPE_KEYS: [&str; 13] = [
    "type",
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
    "subprotocols",
];

fn builtins(py: Python) -> PyResult<&Py<PyModule>> {
    static BUILTINS: OnceCell<Py<PyModule>> = OnceCell::new();
    BUILTINS.get_or_try_init(|| Ok(PyModule::import(py, "builtins")?.into()))
}

#[pymethods]
impl Scope {
    fn version(&self) -> Result<PyObject> {
        Ok(Python::with_gil(|py| "3.0".into_py(py)))
    }

    fn spec_version(&self) -> Result<PyObject> {
        Ok(Python::with_gil(|py| "2.3".into_py(py)))
    }

    fn http_version(&self) -> Result<PyObject> {
        Ok(Python::with_gil(|py| {
            match self.parts.version {
                Version::HTTP_10 => "1.0",
                Version::HTTP_11 => "1.1",
                Version::HTTP_2 => "2.0",
                v => panic!("Unsupported http version {:?}", v),
            }
            .into_py(py)
        }))
    }

    fn method(&self) -> Result<PyObject> {
        Ok(Python::with_gil(|py| {
            match self._type {
                ScopeType::Http => Some(self.parts.method.as_str().to_uppercase()),
                _ => None,
            }
            .into_py(py)
        }))
    }

    fn scheme(&self) -> Result<PyObject> {
        let scheme = self.parts.uri.scheme_str();

        Ok(Python::with_gil(|py| match scheme {
            Some(v) => v.into_py(py),
            None => match self._type {
                ScopeType::Http => "http",
                ScopeType::Ws => "ws",
                _ => panic!("failed to parse scheme"),
            }
            .into_py(py),
        }))
    }

    fn root_path(&self) -> Result<PyObject> {
        Ok(Python::with_gil(|py| "".into_py(py)))
    }

    fn headers(&self) -> Result<PyObject> {
        let mut headers = vec![];

        if let Some(auth) = self.parts.uri.authority() {
            headers.push((HOST.as_str().as_bytes(), auth.as_str().as_bytes()));
        }

        for (key, val) in self.parts.headers.iter() {
            if self._type == ScopeType::Ws && key == SEC_WEBSOCKET_PROTOCOL {
                continue;
            }
            headers.push((key.as_str().as_bytes(), val.as_bytes()));
        }

        Ok(Python::with_gil(|py| headers.into_py(py)))
    }

    fn client(&self) -> Result<PyObject> {
        Ok(Python::with_gil(|py| {
            (self.addr.ip().to_string(), self.addr.port()).into_py(py)
        }))
    }

    fn path(&self) -> Result<PyObject> {
        Ok(Python::with_gil(|py| {
            decode(self.parts.uri.path()).expect("UTF-8").into_py(py)
        }))
    }

    fn raw_path(&self) -> Result<PyObject> {
        Ok(Python::with_gil(|py| {
            self.parts.uri.path().as_bytes().into_py(py)
        }))
    }

    fn query_string(&self) -> Result<PyObject> {
        Ok(Python::with_gil(|py| {
            match self.parts.uri.query() {
                Some(s) => decode(s).expect("UTF-8").to_string(),
                None => "".to_string(),
            }
            .into_py(py)
        }))
    }

    fn subprotocols(&self) -> Result<PyObject> {
        Ok(Python::with_gil(|py| {
            match self._type {
                ScopeType::Ws => {
                    let mut subprotocols = vec![];
                    for val in self.parts.headers.get_all(SEC_WEBSOCKET_PROTOCOL) {
                        let mut s = val.to_str().unwrap().split(',').collect::<Vec<&str>>();
                        subprotocols.append(&mut s);
                    }
                    Some(subprotocols)
                }
                _ => None,
            }
            .into_py(py)
        }))
    }

    fn __contains__(slf: PyRef<Self>, key: &str) -> PyResult<bool> {
        let mut exists = SCOPE_KEYS.iter().any(|&x| x == key);

        if !exists {
            exists = slf._dict.cast_as::<PyDict>(slf.py())?.contains(key)?;
        }

        Ok(exists)
    }

    fn get(&self, key: &str, value: Option<PyObject>) -> Result<PyObject> {
        let resp = self.__getitem__(key)?;
        Ok(match resp {
            Some(value) => value,
            None => value.unwrap_or_else(|| Python::with_gil(|py| ().into_py(py))),
        })
    }

    fn __getitem__(&self, key: &str) -> Result<Option<PyObject>> {
        Ok(match key {
            "type" => Some(Python::with_gil(|py| self._type.into_py(py))),
            "version" => Some(self.version()?),
            "spec_version" => Some(self.spec_version()?),
            "http_version" => Some(self.http_version()?),
            "method" => Some(self.method()?),
            "scheme" => Some(self.scheme()?),
            "root_path" => Some(self.root_path()?),
            "headers" => Some(self.headers()?),
            "client" => Some(self.client()?),
            "path" => Some(self.path()?),
            "raw_path" => Some(self.raw_path()?),
            "query_string" => Some(self.query_string()?),
            "subprotocols" => Some(self.subprotocols()?),
            _ => {
                let resp = Python::with_gil(|py| match self._dict.cast_as::<PyDict>(py) {
                    Ok(_d) => _d.get_item(key).map(|v| v.into()),
                    Err(_) => None,
                });
                debug!("custom key {} -> {:?}", key, resp);
                resp
            }
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

    fn __iter__(slf: PyRef<Self>) -> PyResult<Py<IterKeyScope>> {
        let iter = IterKeyScope::new(slf.py(), &slf._dict)?;
        Py::new(slf.py(), iter)
    }

    fn keys(slf: PyRef<Self>) -> Result<Vec<String>> {
        let mut v: Vec<String> = SCOPE_KEYS.into_iter().map(|v| v.to_string()).collect();
        let mut dict_keys = builtins(slf.py())?
            .getattr(slf.py(), "list")?
            .call1(slf.py(), (slf._dict.clone(),))?
            .extract::<Vec<String>>(slf.py())?;

        v.append(&mut dict_keys);

        Ok(v)
    }

    fn items(&self) -> Result<Vec<(String, PyObject)>> {
        let mut v: Vec<(String, PyObject)> = vec![];
        for key in SCOPE_KEYS {
            let value = Scope::__getitem__(self, key);
            match value {
                Ok(value) => v.push((key.to_string(), value.unwrap())),
                Err(e) => {
                    error!("scope.items(): can't get value for {}, {:?}", key, e);
                    continue;
                }
            }
        }
        let mut dict_keys = Python::with_gil(|py| {
            builtins(py)?
                .getattr(py, "list")?
                .call1(py, (self._dict.call_method0(py, "items")?,))?
                .extract::<Vec<(String, PyObject)>>(py)
        })?;

        v.append(&mut dict_keys);

        Ok(v)
    }
}

#[pyclass(module = "rusticorn")]
struct IterKeyScope {
    scope_keys_idx: usize,
    inner: PyObject,
}

impl IterKeyScope {
    pub fn new(py: Python, _dict: &PyObject) -> PyResult<Self> {
        let builtins = PyModule::import(py, "builtins")?;
        Ok(Self {
            inner: builtins
                .getattr("iter")?
                .call1((_dict,))?
                .cast_as::<PyIterator>()?
                .iter()?
                .into(),
            scope_keys_idx: 0,
        })
    }
}

#[pymethods]
impl IterKeyScope {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>) -> PyResult<Option<PyObject>> {
        if slf.scope_keys_idx < SCOPE_KEYS.len() {
            let resp = Ok(Some(SCOPE_KEYS[slf.scope_keys_idx].into_py(slf.py())));
            slf.scope_keys_idx += 1;
            return resp;
        }
        let nxt = slf
            .inner
            .cast_as::<PyIterator>(slf.py())?
            .call_method0("__next__");
        if let Ok(nxt) = nxt {
            return Ok(Some(nxt.into()));
        }
        Ok(None)
    }
}

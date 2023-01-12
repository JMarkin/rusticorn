use crate::prelude::*;

pub fn bool_from_scope(scope: &PyDict, name: &str) -> bool {
    let py_bool = scope.get_item(name);
    let _bool: bool;
    if let Some(py_more_body) = py_bool {
        _bool = py_more_body.extract::<bool>().unwrap_or(false);
    } else {
        _bool = false;
    }
    _bool
}

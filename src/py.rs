use pyo3::pyclass::IterNextOutput;

use crate::prelude::*;
#[pyclass]
pub struct AwaitableObj {
    data: Option<PyObject>,
}
#[pymethods]
impl AwaitableObj {
    fn __await__(slf: PyRef<Self>) -> PyRef<Self> {
        slf // We're saying that we are the iterable part of the coroutine.
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(slf: PyRefMut<'_, Self>) -> IterNextOutput<Option<PyObject>, Option<PyObject>> {
        IterNextOutput::Return(slf.data.clone())
    }
}

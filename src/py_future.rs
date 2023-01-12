// original code https://gitlab.com/ThibaultLemaire/pyo3-futures
use {
    futures::{
        future::{BoxFuture, FutureExt},
        stream::{BoxStream, Stream, StreamExt},
        task::{waker_ref, ArcWake},
    },
    once_cell::sync::OnceCell,
    pyo3::{
        callback::IntoPyCallbackOutput,
        exceptions::{asyncio::CancelledError, PyStopAsyncIteration},
        ffi,
        iter::IterNextOutput,
        prelude::*,
        pyasync::IterANextOutput,
        types::IntoPyDict,
        PyTypeInfo,
    },
    std::{
        future::Future,
        marker::PhantomData,
        mem,
        sync::Arc,
        task::{Context, Poll},
    },
};

#[pyclass]
#[derive(Default)]
pub struct AwaitableObj {}

#[pymethods]
impl AwaitableObj {
    fn __await__(slf: PyRef<Self>) -> PyRef<Self> {
        slf // We're saying that we are the iterable part of the coroutine.
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(_slf: PyRefMut<'_, Self>) -> IterNextOutput<Option<PyObject>, Option<PyObject>> {
        IterNextOutput::Return(None)
    }
}

fn monkey_patch_into_accepted_coro_types<T: PyTypeInfo>(py: Python) -> PyResult<&()> {
    static MONKEY_PATCHED: OnceCell<()> = OnceCell::new();
    MONKEY_PATCHED.get_or_try_init(|| {
        let coroutines = PyModule::import(py, "asyncio.coroutines")?;
        let typecache_set: &pyo3::types::PySet =
            coroutines.getattr("_iscoroutine_typecache")?.extract()?;
        typecache_set.add(T::type_object(py))?;
        Ok(())
    })
}

fn asyncio(py: Python) -> PyResult<&Py<PyModule>> {
    static ASYNCIO: OnceCell<Py<PyModule>> = OnceCell::new();
    ASYNCIO.get_or_try_init(|| Ok(PyModule::import(py, "asyncio")?.into()))
}

fn get_running_loop(py: Python) -> PyResult<PyObject> {
    static GET_RUNNING_LOOP: OnceCell<PyObject> = OnceCell::new();
    GET_RUNNING_LOOP
        .get_or_try_init::<_, PyErr>(|| asyncio(py)?.getattr(py, "get_running_loop"))?
        .call0(py)
}

fn cancelled_error() -> PyErr {
    CancelledError::new_err("PyFuture cancelled")
}

enum FutureState {
    Cancelled,
    Pending {
        future: BoxFuture<'static, PyResult<PyObject>>,
        waker: Arc<AsyncioWaker>,
    },
    Executing,
}

/// Rust Coroutine
#[pyclass(weakref)]
pub struct PyFuture {
    future: FutureState,
    aio_loop: PyObject,
    callbacks: Vec<(PyObject, Option<PyObject>)>,
    #[pyo3(get, set)]
    _asyncio_future_blocking: bool,
}

#[pyclass]
#[derive(Clone)]
struct AsyncioWaker {
    aio_loop: PyObject,
    py_future: Py<PyFuture>,
}

impl ArcWake for AsyncioWaker {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        let closure = (**arc_self).clone();
        Python::with_gil(|py| {
            arc_self
                .aio_loop
                .call_method1(py, "call_soon_threadsafe", (closure,))
        })
        .expect("exception thrown by the event loop (probably closed)");
    }
}

#[pymethods]
impl AsyncioWaker {
    fn __call__(slf: PyRef<Self>) -> PyResult<()> {
        PyFuture::schedule_callbacks(slf.py_future.try_borrow_mut(slf.py())?)
    }
}

#[pymethods]
impl PyFuture {
    #[new]
    fn new(r#loop: Option<PyObject>) -> Self {
        PyFuture {
            future: FutureState::Cancelled,
            aio_loop: if let Some(ll) = r#loop {
                ll
            } else {
                Python::with_gil(|py| get_running_loop(py).unwrap())
            },
            callbacks: vec![],
            _asyncio_future_blocking: false,
        }
    }

    fn __await__(slf: PyRef<Self>) -> PyRef<Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<Self>) -> PyResult<IterNextOutput<PyRefMut<Self>, PyObject>> {
        let mut execution_slot = FutureState::Executing;
        mem::swap(&mut execution_slot, &mut slf.future);
        let result = match &mut execution_slot {
            FutureState::Pending { future, waker } => slf.py().allow_threads(|| {
                let waker_ref = waker_ref(waker);
                let context = &mut Context::from_waker(&waker_ref);
                future.as_mut().poll(context)
            }),
            FutureState::Cancelled => Poll::Ready(Err(cancelled_error())),
            _ => unimplemented!(),
        };
        mem::swap(&mut execution_slot, &mut slf.future);
        match result {
            Poll::Pending => {
                slf._asyncio_future_blocking = true;
                Ok(IterNextOutput::Yield(slf))
            }
            Poll::Ready(result) => Ok(IterNextOutput::Return(result?)),
        }
    }

    fn get_loop(&self) -> &PyObject {
        &self.aio_loop
    }

    fn add_done_callback(&mut self, callback: PyObject, context: Option<PyObject>) {
        self.callbacks.push((callback, context));
    }

    /// https://docs.python.org/3/reference/datamodel.html#coroutine.send
    fn send(slf: PyRefMut<Self>, value: Option<&PyAny>) -> PyResult<PyObject> {
        if value.is_some() {
            unimplemented!();
        };

        Python::with_gil(|py| slf.into_py(py).call_method0(py, "__next__"))
    }

    /// https://docs.python.org/3/reference/datamodel.html#coroutine.throw
    fn throw(_slf: Py<Self>, type_: &PyAny, exc: Option<&PyAny>, traceback: Option<&PyAny>) {
        panic!("throw({:?}, {:?}, {:?})", type_, exc, traceback);
    }

    /// https://docs.python.org/3/library/asyncio-future.html?highlight=asyncio%20future#asyncio.Future.result
    fn result(&self) -> PyResult<Option<PyObject>> {
        match self.future {
            FutureState::Cancelled => Err(cancelled_error()),
            FutureState::Pending { .. } => Ok(None),
            _ => unimplemented!(),
        }
    }

    /// https://docs.python.org/3/library/asyncio-future.html?highlight=asyncio%20future#asyncio.Future.cancel
    fn cancel(mut slf: PyRefMut<Self>, _msg: Option<&PyAny>) -> PyResult<()> {
        slf.future = FutureState::Cancelled;
        PyFuture::schedule_callbacks(slf)
    }

    /// https://docs.python.org/3/library/asyncio-future.html?highlight=asyncio%20future#asyncio.Future.cancelled
    fn cancelled(&self) -> bool {
        matches!(self.future, FutureState::Cancelled)
    }
}

impl PyFuture {
    /// https://github.com/python/cpython/blob/17ef4319a34f5a2f95e7823dfb5f5b8cff11882d/Lib/asyncio/futures.py#L159
    fn schedule_callbacks(mut slf: PyRefMut<Self>) -> PyResult<()> {
        if slf.callbacks.is_empty() {
            panic!("nothing to call back")
        }
        let callbacks = std::mem::take(&mut slf.callbacks);
        let py = slf.py();
        for (callback, context) in callbacks {
            slf.aio_loop.call_method(
                py,
                "call_soon",
                (callback, &slf),
                Some(vec![("context", context)].into_py_dict(py)),
            )?;
        }
        Ok(())
    }
}

struct PySendableFuture(BoxFuture<'static, PyResult<PyObject>>);

impl<TFuture, TOutput> From<TFuture> for PySendableFuture
where
    TFuture: Future<Output = TOutput> + Send + 'static,
    TOutput: IntoPyCallbackOutput<PyObject>,
{
    fn from(future: TFuture) -> Self {
        Self(
            async move {
                let result = future.await;
                Python::with_gil(move |py| result.convert(py))
            }
            .boxed(),
        )
    }
}

impl From<PySendableFuture> for BoxFuture<'static, PyResult<PyObject>> {
    fn from(wrapper: PySendableFuture) -> Self {
        wrapper.0
    }
}

pub struct PyAsync<T>(PySendableFuture, PhantomData<T>);

impl<T> PyAsync<T> {
    pub fn assert_into_py_result(&self) {
        panic!("assert_into_py_result unimplement");
    }
}

impl<TFuture, TOutput> From<TFuture> for PyAsync<TOutput>
where
    TFuture: Future<Output = TOutput> + Send + 'static,
    TOutput: IntoPyCallbackOutput<PyObject>,
{
    fn from(future: TFuture) -> Self {
        Self(future.into(), PhantomData)
    }
}

impl<TOutput> IntoPyCallbackOutput<*mut ffi::PyObject> for PyAsync<TOutput>
where
    TOutput: IntoPyCallbackOutput<PyObject>,
{
    fn convert(self, py: Python) -> PyResult<*mut ffi::PyObject> {
        monkey_patch_into_accepted_coro_types::<PyFuture>(py)?;
        let aio_loop = get_running_loop(py)?;
        let rustine = Py::new(
            py,
            PyFuture {
                future: FutureState::Cancelled,
                aio_loop: aio_loop.clone(),
                callbacks: vec![],
                _asyncio_future_blocking: false,
            },
        )?;
        let clone = rustine.clone();
        rustine.try_borrow_mut(py)?.future = FutureState::Pending {
            future: self.0.into(),
            waker: Arc::new(AsyncioWaker {
                aio_loop,
                py_future: clone,
            }),
        };
        rustine.convert(py)
    }
}

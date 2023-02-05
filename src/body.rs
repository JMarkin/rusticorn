use std::{
    pin::Pin,
    task::{Context, Poll},
};

use crate::prelude::*;
use futures::channel::mpsc::UnboundedReceiver;
use http_body_util::{Full, StreamBody};
use hyper::body::{Body, Buf, Bytes, Frame, SizeHint};
use pin_project_lite::pin_project;

pin_project! {
    /// A body that consists of a single chunk.
    #[derive(Clone, Copy, Debug)]
    pub struct FullBody<D> {
        data: Option<D>,
    }
}

impl<D> FullBody<D>
where
    D: Buf,
{
    /// Create a new `Full`.
    pub fn new(data: D) -> Self {
        let data = if data.has_remaining() {
            Some(data)
        } else {
            None
        };
        FullBody { data }
    }
}

impl<D> Body for FullBody<D>
where
    D: Buf,
{
    type Data = D;
    type Error = anyhow::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<D>, Self::Error>>> {
        Poll::Ready(self.data.take().map(|d| Ok(Frame::data(d))))
    }

    fn is_end_stream(&self) -> bool {
        self.data.is_none()
    }

    fn size_hint(&self) -> SizeHint {
        self.data
            .as_ref()
            .map(|data| SizeHint::with_exact(u64::try_from(data.remaining()).unwrap()))
            .unwrap_or_else(|| SizeHint::with_exact(0))
    }
}

pin_project! {
    #[project = HttpBodyProj]
    pub enum HttpBody<T> {
        Full { #[pin] body: FullBody<T> },
        Streamning{ #[pin] body: StreamBody<UnboundedReceiver<Result<Frame<T>>>> },
    }
}

impl<T: Buf> Body for HttpBody<T> {
    type Data = T;
    type Error = anyhow::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.project() {
            HttpBodyProj::Full { body } => body.poll_frame(cx),
            HttpBodyProj::Streamning { body } => body.poll_frame(cx),
        }
    }
}

impl From<&'static str> for HttpBody<Bytes> {
    fn from(value: &'static str) -> Self {
        Self::Full {
            body: FullBody::new(Bytes::from(value)),
        }
    }
}


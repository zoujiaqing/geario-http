//! `hyper::rt::Read` and `Write` over a geario `Io`.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use geario::io::{Filter, Io};
use hyper::rt::{Read, ReadBufCursor, Write};

/// Carries a geario `Io` across hyper's runtime boundary.
///
/// Not `Send`: geario's handles are `Rc`-based and belong to the worker that
/// accepted the connection. hyper does not require `Send` for its HTTP/1
/// connection tasks, so this is a real constraint being respected rather than
/// one being worked around.
#[derive(Debug)]
pub struct GearioTransport<F> {
    io: Io<F>,
}

impl<F> GearioTransport<F> {
    pub fn new(io: Io<F>) -> Self {
        Self { io }
    }

    pub fn into_inner(self) -> Io<F> {
        self.io
    }
}

impl<F: Filter> Read for GearioTransport<F> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut buf: ReadBufCursor<'_>,
    ) -> Poll<io::Result<()>> {
        // geario reads into its own buffer, so this drains that buffer into
        // hyper's. One copy per read, and the only one: geario has already
        // read from the socket by the time we get here, and hyper needs the
        // bytes in the cursor it supplied.
        //
        // Draining first means a poll that has data never touches the
        // readiness path at all.
        let taken = self.io.get_ref().with_read_buf(|src| {
            if src.is_empty() {
                return 0;
            }
            let n = std::cmp::min(src.len(), buf.remaining());
            if n == 0 {
                return 0;
            }
            buf.put_slice(&src[..n]);
            let _ = src.split_to(n);
            n
        });
        if taken > 0 {
            return Poll::Ready(Ok(()));
        }

        match ready!(self.io.poll_read_ready(cx)) {
            // Closed. Returning Ok without filling the cursor is how hyper
            // is told about EOF.
            Ok(None) => Poll::Ready(Ok(())),
            Ok(Some(())) => {
                let taken = self.io.get_ref().with_read_buf(|src| {
                    if src.is_empty() {
                        return 0;
                    }
                    let n = std::cmp::min(src.len(), buf.remaining());
                    if n == 0 {
                        return 0;
                    }
                    buf.put_slice(&src[..n]);
                    let _ = src.split_to(n);
                    n
                });
                if taken > 0 {
                    Poll::Ready(Ok(()))
                } else {
                    // Ready but nothing arrived: wait for the next wakeup
                    // rather than reporting a spurious EOF.
                    Poll::Pending
                }
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

impl<F: Filter> Write for GearioTransport<F> {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        // Writes go into geario's outgoing buffer and are flushed by the IO
        // task, so this always accepts the whole slice. Backpressure is
        // applied at flush, which is where geario tracks it.
        let res = self
            .io
            .get_ref()
            .with_write_buf(|dst| dst.extend_from_slice(buf));
        match res {
            Ok(()) => Poll::Ready(Ok(buf.len())),
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.io.poll_flush(cx, false)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.io.poll_shutdown(cx)
    }
}

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
    /// Set once hyper has asked to shut the connection down.
    ///
    /// A `Cell` because the write methods take `Pin<&mut Self>` and geario's
    /// handles are not `Unpin`-friendly to move through.
    ///
    /// After that, geario reports a closed stream as an error from flush,
    /// while hyper expects flushing a finished connection to succeed. Without
    /// this, a transfer that completed cleanly still ends with the connection
    /// future returning Disconnected.
    shutting_down: std::cell::Cell<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::poll_fn;
    use geario::io::{IoConfig, testing::IoTest};
    use geario::service::cfg::SharedCfg;
    use geario::util::future::lazy;
    use hyper::rt::ReadBuf;

    #[geario::test]
    async fn reads_partial_buffers_and_empty_cursor() {
        let (peer, stream) = IoTest::create();
        let mut io = GearioTransport::new(Io::new(stream, SharedCfg::new("READ")));
        let mut empty = [];
        let mut empty = ReadBuf::new(&mut empty);
        assert!(lazy(|cx| Pin::new(&mut io).poll_read(cx, empty.unfilled())).await.is_ready());

        peer.write(b"abcdef");
        let mut actual = Vec::new();
        for _ in 0..3 {
            let mut bytes = [0; 2];
            let mut buf = ReadBuf::new(&mut bytes);
            poll_fn(|cx| Pin::new(&mut io).poll_read(cx, buf.unfilled())).await.unwrap();
            actual.extend_from_slice(buf.filled());
        }
        assert_eq!(actual, b"abcdef");
        let mut bytes = [0; 8];
        let mut buf = ReadBuf::new(&mut bytes);
        assert!(lazy(|cx| Pin::new(&mut io).poll_read(cx, buf.unfilled())).await.is_pending());
        peer.write(b"next");
        poll_fn(|cx| Pin::new(&mut io).poll_read(cx, buf.unfilled())).await.unwrap();
        assert_eq!(buf.filled(), b"next");
        peer.close().await;
        let mut buf = ReadBuf::new(&mut bytes);
        poll_fn(|cx| Pin::new(&mut io).poll_read(cx, buf.unfilled())).await.unwrap();
        assert!(buf.filled().is_empty());
    }

    #[geario::test]
    async fn vectored_write_takes_every_slice_then_backpressures() {
        let (peer, stream) = IoTest::create();
        peer.remote_buffer_cap(0);
        let mut io = GearioTransport::new(Io::new(
            stream,
            SharedCfg::new("WRITE").add(IoConfig::default().set_write_buf(16, 8, 12)),
        ));
        // Initialize the test stream's IO tasks before writing.
        let _ = lazy(|cx| io.io.poll_read_ready(cx)).await;

        // Every slice is taken in one call. Splitting a response across calls
        // is what leaves geario with a single page per wakeup, and it only
        // reaches for writev when more than one page is queued: an earlier
        // version capped each call at the watermark and cost an extra write
        // syscall on any response that straddled it.
        let slices = [
            io::IoSlice::new(b""),
            io::IoSlice::new(b"header"),
            io::IoSlice::new(b"0123456789abcdef"),
        ];
        let total: usize = slices.iter().map(|s| s.len()).sum();
        let n = poll_fn(|cx| Pin::new(&mut io).poll_write_vectored(cx, &slices))
            .await
            .unwrap();
        assert_eq!(n, total, "a slice was dropped or truncated");

        // Backpressure is applied before accepting, not by truncating: with
        // the peer refusing to read, further writes have to stop.
        let mut extra = 0;
        for _ in 0..8 {
            match lazy(|cx| Pin::new(&mut io).poll_write(cx, b"tail")).await {
                Poll::Ready(Ok(n)) => extra += n,
                Poll::Pending => break,
                other => panic!("unexpected write: {other:?}"),
            }
        }
        assert!(
            lazy(|cx| Pin::new(&mut io).poll_write(cx, b"blocked")).await.is_pending(),
            "writes never stopped even though the peer reads nothing"
        );
        assert!(lazy(|cx| Pin::new(&mut io).poll_flush(cx)).await.is_pending());

        peer.remote_buffer_cap(1024);
        poll_fn(|cx| Pin::new(&mut io).poll_flush(cx)).await.unwrap();
        let bytes = peer.read_any();
        assert_eq!(&bytes[..total], b"header0123456789abcdef");
        assert_eq!(bytes.len(), total + extra);
        assert!(io.is_write_vectored());
    }

    #[geario::test]
    async fn flush_drains_small_writes_and_shutdown_delivers_tail() {
        let (peer, stream) = IoTest::create();
        peer.remote_buffer_cap(0);
        let mut io = GearioTransport::new(Io::new(stream, SharedCfg::new("FLUSH")));
        let _ = lazy(|cx| io.io.poll_read_ready(cx)).await;
        poll_fn(|cx| Pin::new(&mut io).poll_write(cx, b"small")).await.unwrap();
        // Below the low watermark still is not a completed flush.
        assert!(lazy(|cx| Pin::new(&mut io).poll_flush(cx)).await.is_pending());
        peer.remote_buffer_cap(1024);
        poll_fn(|cx| Pin::new(&mut io).poll_flush(cx)).await.unwrap();
        assert_eq!(&peer.read_any()[..], b"small");
        poll_fn(|cx| Pin::new(&mut io).poll_write(cx, b"tail")).await.unwrap();
        poll_fn(|cx| Pin::new(&mut io).poll_shutdown(cx)).await.unwrap();
        assert_eq!(&peer.read_any()[..], b"tail");
        assert!(poll_fn(|cx| Pin::new(&mut io).poll_write(cx, b"late")).await.is_err());
    }
}

impl<F> GearioTransport<F> {
    pub fn new(io: Io<F>) -> Self {
        Self {
            io,
            shutting_down: std::cell::Cell::new(false),
        }
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
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
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
            src.advance_to(n);
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
                    src.advance_to(n);
                    n
                });
                if taken > 0 {
                    Poll::Ready(Ok(()))
                } else {
                    // Ready but nothing arrived: wait for the next wakeup
                    // rather than reporting a spurious EOF.
                    // Accessing the buffer clears geario's read-ready flag.
                    // Register again so a Pending result always has a wakeup.
                    match self.io.poll_read_ready(cx) {
                        Poll::Ready(Ok(None)) => Poll::Ready(Ok(())),
                        Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                        Poll::Pending => Poll::Pending,
                        Poll::Ready(Ok(Some(()))) => {
                            cx.waker().wake_by_ref();
                            Poll::Pending
                        }
                    }
                }
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }
}

impl<F: Filter> Write for GearioTransport<F> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        if self.io.is_wr_backpressure() {
            ready!(self.io.poll_flush(cx, false))?;
        }
        // Hand it to the socket first. Buffering copies the bytes once here
        // and again into the socket, and that second copy is proportional to
        // the response: at 16 KB it showed up as the whole remaining gap
        // against tokio, at 1 KB as nothing.
        match self.io.get_ref().try_write_vectored(&[io::IoSlice::new(buf)]) {
            Ok(0) => {}
            Ok(n) => return Poll::Ready(Ok(n)),
            Err(e) => return Poll::Ready(Err(e)),
        }

        // The socket would not take it: buffer, so the write interest gets
        // armed and the connection parks instead of spinning.
        let len = buf.len();
        let res = self
            .io
            .get_ref()
            .with_write_buf(|dst| dst.extend_from_slice(buf));
        match res {
            Ok(()) => Poll::Ready(Ok(len)),
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn is_write_vectored(&self) -> bool {
        // Toggleable so an A/B run can compare the two paths in one binary,
        // which removes build differences from the comparison.
        !matches!(std::env::var("GEARIO_NO_VECTORED").as_deref(), Ok("1"))
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        if bufs.iter().all(|buf| buf.is_empty()) {
            return Poll::Ready(Ok(0));
        }
        if self.io.is_wr_backpressure() {
            ready!(self.io.poll_flush(cx, false))?;
        }
        // Advertising vectored writes makes hyper queue header and body
        // slices instead of flattening them into an intermediate buffer.
        // Taking them all in one call is what lets geario reach for writev
        // rather than issuing a write per slice.
        match self.io.get_ref().try_write_vectored(bufs) {
            Ok(0) => {}
            Ok(n) => return Poll::Ready(Ok(n)),
            Err(e) => return Poll::Ready(Err(e)),
        }

        Poll::Ready(self.io.get_ref().with_write_buf(|dst| {
            let mut written = 0;
            for buf in bufs {
                dst.extend_from_slice(buf);
                written += buf.len();
            }
            written
        }))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Try geario's direct-write path before parking the connection. A
        // successful hyper flush must drain the buffer, not merely reach the
        // low watermark (the meaning of geario's `full = false`).
        if let Err(e) = self.io.send_buf() {
            return Poll::Ready(self.finished_or(e));
        }
        match self.io.poll_flush(cx, true) {
            Poll::Ready(Err(e)) => Poll::Ready(self.finished_or(e)),
            other => other,
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.shutting_down.set(true);
        self.io.poll_shutdown(cx)
    }

}

impl<F> GearioTransport<F> {
    /// Once shutdown has been asked for, a closed stream is the expected
    /// outcome rather than a failure. Reporting it as an error makes a
    /// completed transfer look like a broken one.
    fn finished_or(&self, e: io::Error) -> io::Result<()> {
        if self.shutting_down.get() && e.kind() == io::ErrorKind::NotConnected {
            Ok(())
        } else {
            Err(e)
        }
    }
}

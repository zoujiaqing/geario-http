//! Serving a connection with hyper when the protocol is not known in advance.
//!
//! HTTP/2 over cleartext with prior knowledge starts with a fixed 24-byte
//! preface. Anything else on a plain socket is HTTP/1. geario has already
//! read the first bytes into its own buffer by the time a connection is
//! handed over, so the protocol can be decided by looking at that buffer
//! without consuming anything; hyper's h2 server expects to read the preface
//! itself.

use std::error::Error as StdError;
use std::io;

use geario::io::{Filter, Io};
use hyper::body::{Body, Incoming};
use hyper::server::conn::{http1, http2};
use hyper::service::HttpService;

use super::{GearioExecutor, GearioTimer, GearioTransport};

const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

/// What the first bytes on the connection say it is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Protocol {
    Http1,
    Http2,
}

/// Decide the protocol from the first bytes, consuming none of them.
///
/// Waits until the buffer either holds the whole preface, or holds something
/// the preface does not start with. A peer that sends the preface in pieces
/// is still HTTP/2; a peer that closes before saying anything is an error.
pub async fn detect<F: Filter>(io: &Io<F>) -> io::Result<Protocol> {
    loop {
        let verdict = io.with_read_buf(|buf| {
            if buf.len() >= PREFACE.len() {
                Some(&buf[..PREFACE.len()] == PREFACE)
            } else if PREFACE.starts_with(buf) {
                None
            } else {
                Some(false)
            }
        });
        match verdict {
            Some(true) => return Ok(Protocol::Http2),
            Some(false) => return Ok(Protocol::Http1),
            None => {}
        }
        if io.read_ready().await?.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "peer closed before the protocol could be determined",
            ));
        }
    }
}

/// Serve one connection with hyper, as HTTP/1 or HTTP/2 according to what the
/// peer sends first.
pub async fn serve_auto<F, S, B>(
    io: Io<F>,
    service: S,
) -> Result<(), Box<dyn StdError + Send + Sync>>
where
    F: Filter + Unpin,
    S: HttpService<Incoming, ResBody = B>,
    S::Error: Into<Box<dyn StdError + Send + Sync>>,
    B: Body + 'static,
    B::Error: Into<Box<dyn StdError + Send + Sync>>,
    GearioExecutor: hyper::rt::bounds::Http2ServerConnExec<S::Future, B>,
{
    match detect(&io).await? {
        Protocol::Http1 => {
            http1::Builder::new()
                .timer(GearioTimer::new())
                .serve_connection(GearioTransport::new(io), service)
                .await?;
        }
        Protocol::Http2 => {
            http2::Builder::new(GearioExecutor)
                .timer(GearioTimer::new())
                .serve_connection(GearioTransport::new(io), service)
                .await?;
        }
    }
    Ok(())
}

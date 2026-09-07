//! Response tickets.
//!
//! A responder is a `u64` rather than a pointer, which keeps the host from
//! having to reason about the lifetime of anything geario owns. The top 32
//! bits carry the worker that issued it.
//!
//! That encoding is not decoration. geario runs thread-per-core and its
//! handles are `Rc`-based, so a response has to be delivered on the worker
//! that took the request. C cannot express that in a type, so it is checked
//! at run time and reported as `GEARIO_HTTP_STATUS_WRONG_THREAD` instead of
//! becoming undefined behaviour.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use geario::util::channel::{mpsc, oneshot};
use hyper::body::Bytes;
use hyper::header::{HeaderName, HeaderValue};

/// What the host eventually says in reply.
pub(crate) enum Reply {
    /// Status, headers and the whole body at once.
    Once {
        status: u16,
        headers: Vec<(HeaderName, HeaderValue)>,
        body: Vec<u8>,
    },
    /// Status and headers now, body chunks as they arrive.
    Stream {
        status: u16,
        headers: Vec<(HeaderName, HeaderValue)>,
        chunks: mpsc::Receiver<Bytes>,
    },
}

static NEXT_WORKER: AtomicU32 = AtomicU32::new(1);

thread_local! {
    /// Identifies this worker inside responder ids. Zero is never handed out,
    /// so a responder of 0 is always invalid.
    static WORKER_ID: u32 = NEXT_WORKER.fetch_add(1, Ordering::Relaxed);
    static NEXT_LOCAL: Cell<u32> = const { Cell::new(1) };
    static PENDING: RefCell<HashMap<u64, oneshot::Sender<Reply>>> =
        RefCell::new(HashMap::new());
    /// Responders whose head has been sent and whose body is still open.
    static STREAMING: RefCell<HashMap<u64, mpsc::Sender<Bytes>>> =
        RefCell::new(HashMap::new());
}

pub(crate) fn worker_id() -> u32 {
    WORKER_ID.with(|w| *w)
}

/// Reserve a responder on this worker and return it with the receiving half.
pub(crate) fn register() -> (u64, oneshot::Receiver<Reply>) {
    let local = NEXT_LOCAL.with(|n| {
        let v = n.get();
        n.set(v.wrapping_add(1).max(1));
        v
    });
    let id = (u64::from(worker_id()) << 32) | u64::from(local);
    let (tx, rx) = oneshot::channel();
    PENDING.with(|p| p.borrow_mut().insert(id, tx));
    (id, rx)
}

/// Outcome of trying to answer a responder.
pub(crate) enum Delivery {
    Sent,
    /// Called from a worker other than the one that issued the responder.
    WrongThread,
    /// Unknown id, or one that was already answered.
    Unknown,
}

pub(crate) fn deliver(id: u64, reply: Reply) -> Delivery {
    if id == 0 {
        return Delivery::Unknown;
    }
    if (id >> 32) as u32 != worker_id() {
        return Delivery::WrongThread;
    }
    let tx = PENDING.with(|p| p.borrow_mut().remove(&id));
    match tx {
        Some(tx) => {
            // A closed receiver means the connection went away first, which is
            // not the host's fault and not worth an error.
            let _ = tx.send(reply);
            Delivery::Sent
        }
        None => Delivery::Unknown,
    }
}

/// Drop a responder that will never be answered.
pub(crate) fn forget(id: u64) {
    PENDING.with(|p| p.borrow_mut().remove(&id));
}

/// Where a responder is in its life.
pub(crate) enum Phase {
    /// Nothing sent yet.
    Fresh,
    /// Head sent, body open.
    Streaming,
    /// Answered, or never existed.
    Done,
}

pub(crate) fn phase(id: u64) -> Phase {
    if id == 0 {
        return Phase::Done;
    }
    if STREAMING.with(|s| s.borrow().contains_key(&id)) {
        return Phase::Streaming;
    }
    if PENDING.with(|p| p.borrow().contains_key(&id)) {
        return Phase::Fresh;
    }
    Phase::Done
}

/// Open a streaming body and hand the head to the waiting handler.
pub(crate) fn begin_stream(
    id: u64,
    status: u16,
    headers: Vec<(HeaderName, HeaderValue)>,
) -> Delivery {
    if id == 0 {
        return Delivery::Unknown;
    }
    if (id >> 32) as u32 != worker_id() {
        return Delivery::WrongThread;
    }
    let (tx, rx) = mpsc::channel();
    let sender = PENDING.with(|p| p.borrow_mut().remove(&id));
    match sender {
        Some(sender) => {
            STREAMING.with(|s| s.borrow_mut().insert(id, tx));
            let _ = sender.send(Reply::Stream {
                status,
                headers,
                chunks: rx,
            });
            Delivery::Sent
        }
        None => Delivery::Unknown,
    }
}

/// Push one chunk into an open body.
pub(crate) fn write_chunk(id: u64, chunk: Bytes) -> Delivery {
    if id == 0 {
        return Delivery::Unknown;
    }
    if (id >> 32) as u32 != worker_id() {
        return Delivery::WrongThread;
    }
    STREAMING.with(|s| match s.borrow().get(&id) {
        // A send failure means the peer went away; that is not the host's
        // fault, and the stream ends on its own.
        Some(tx) => {
            let _ = tx.send(chunk);
            Delivery::Sent
        }
        None => Delivery::Unknown,
    })
}

/// Close an open body. Dropping the sender ends the stream.
pub(crate) fn finish_stream(id: u64) -> Delivery {
    if id == 0 {
        return Delivery::Unknown;
    }
    if (id >> 32) as u32 != worker_id() {
        return Delivery::WrongThread;
    }
    match STREAMING.with(|s| s.borrow_mut().remove(&id)) {
        Some(_) => Delivery::Sent,
        None => Delivery::Unknown,
    }
}

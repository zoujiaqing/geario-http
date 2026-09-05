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

use geario::util::channel::oneshot;

/// What the host eventually says in reply.
pub(crate) struct Reply {
    pub status: u16,
    pub headers: Vec<(Vec<u8>, Vec<u8>)>,
    pub body: Vec<u8>,
}

static NEXT_WORKER: AtomicU32 = AtomicU32::new(1);

thread_local! {
    /// Identifies this worker inside responder ids. Zero is never handed out,
    /// so a responder of 0 is always invalid.
    static WORKER_ID: u32 = NEXT_WORKER.fetch_add(1, Ordering::Relaxed);
    static NEXT_LOCAL: Cell<u32> = const { Cell::new(1) };
    static PENDING: RefCell<HashMap<u64, oneshot::Sender<Reply>>> =
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

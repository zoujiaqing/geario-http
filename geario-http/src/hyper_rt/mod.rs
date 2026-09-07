//! Running hyper on geario.
//!
//! hyper has been runtime-agnostic since 1.0: it reaches the outside world
//! through `hyper::rt::{Read, Write, Executor, Timer}` and nothing else. Its
//! only tokio dependency is `features = ["sync"]`, for channel primitives it
//! uses internally. So supplying geario here is supported use, not a
//! workaround.
//!
//! The shapes do not line up for free. hyper wants poll-based reads and
//! writes over a caller-supplied buffer; geario owns its buffers and hands
//! them out through closures. This module is where that mismatch is paid for,
//! deliberately in one place, so the cost can be found and measured rather
//! than spread through the codebase.
//!
//! Vectored writes let hyper retain separate header/body slices until they
//! reach geario's write buffer, avoiding hyper's intermediate flattening
//! copy. A write that the socket can take goes there directly, without
//! passing through the write buffer at all; the buffer is the fallback for
//! when it cannot, and taking it is what applies backpressure.

mod executor;
#[cfg(all(
    feature = "hyper-server",
    feature = "hyper-http1",
    feature = "hyper-http2"
))]
mod serve;
mod timer;
mod transport;

pub use self::executor::GearioExecutor;
#[cfg(all(
    feature = "hyper-server",
    feature = "hyper-http1",
    feature = "hyper-http2"
))]
pub use self::serve::{Protocol, detect, serve_auto};
pub use self::timer::GearioTimer;
pub use self::transport::GearioTransport;

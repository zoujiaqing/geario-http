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
//! copy. This is still buffered IO, not zero-copy transport. Writes honor
//! geario's high watermark, and flush waits for all buffered output.

mod executor;
mod transport;

pub use self::executor::GearioExecutor;
pub use self::transport::GearioTransport;

//! C ABI for geario.
//!
//! Not a normal Rust module: everything here exists to be called from C, and
//! the shapes are chosen for that. Rust callers should use `geario` and
//! `geario-http` directly.
#![allow(unreachable_pub)]
#![allow(missing_debug_implementations)]

mod abi;
mod responder;
mod slice;

#[cfg(feature = "client")]
mod client;

#[cfg(feature = "server")]
mod server;

pub use self::abi::*;
pub use self::slice::*;

#[cfg(feature = "client")]
pub use self::client::*;

#[cfg(feature = "server")]
pub use self::server::*;

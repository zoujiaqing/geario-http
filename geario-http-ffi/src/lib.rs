//! C ABI for geario.
//!
//! Not a normal Rust module: everything here exists to be called from C, and
//! the shapes are chosen for that. Rust callers should use `geario` and
//! `geario-http` directly.
#![allow(unreachable_pub)]
#![allow(missing_debug_implementations)]

mod abi;

pub use self::abi::*;

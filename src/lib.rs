//! HTTP protocol support.
#![allow(unreachable_pub)]

#[allow(unused_extern_crates)]
extern crate self as geario_http;

pub mod types;

// re-exports, matching what the upstream facade exposed
pub use crate::types::uri::{self, Uri};
pub use crate::types::{HeaderMap, Method, StatusCode, Version, body, header};

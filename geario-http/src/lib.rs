//! HTTP protocol support.
#![allow(unreachable_pub)]

#[allow(unused_extern_crates)]
extern crate self as geario_http;

#[cfg(feature = "http1")]
mod config;
#[cfg(feature = "compress")]
pub mod encoding;
#[cfg(feature = "http1")]
pub(crate) mod helpers;
#[cfg(feature = "http1")]
mod httpcodes;
#[cfg(feature = "http1")]
mod httpmessage;
#[cfg(feature = "http1")]
mod message;
#[cfg(feature = "http1")]
mod payload;
#[cfg(feature = "http1")]
mod request;
#[cfg(feature = "http1")]
mod response;
#[cfg(feature = "server")]
mod service;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "http1")]
pub mod error;
#[cfg(feature = "http1")]
pub mod h1;
#[cfg(feature = "hyper-rt")]
pub mod hyper_rt;
#[cfg(feature = "http1")]
pub mod types;
#[cfg(feature = "ws")]
pub mod ws;

#[cfg(feature = "http1")]
pub use self::config::{DateService, HttpServiceConfig, KeepAlive};
#[cfg(feature = "http1")]
pub use self::error::ResponseError;
#[cfg(feature = "http1")]
pub use self::httpmessage::HttpMessage;
#[cfg(feature = "http1")]
pub use self::message::{ConnectionType, RequestHead, ResponseHead};
#[cfg(feature = "http1")]
pub use self::payload::{Payload, PayloadStream};
#[cfg(feature = "http1")]
pub use self::request::Request;
#[cfg(feature = "http1")]
pub use self::response::{Response, ResponseBuilder};
#[cfg(feature = "server")]
pub use self::service::HttpService;
pub use geario::io::types::HttpProtocol;

pub const ALPN_PROTO_H1: &[&str] = &["http/1.1"];
pub const ALPN_PROTO_H2: &[&str] = &["h2"];
pub const ALPN_PROTOS: &[&str] = &["h2", "http/1.1"];

/// Header item
#[cfg(feature = "http1")]
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HeaderItem {
    pub name: header::HeaderName,
    pub origin: geario::bytes::ByteString,
    pub value: header::HeaderValue,
}

// re-exports, matching what the upstream facade exposed
#[cfg(feature = "http1")]
pub use crate::types::uri::{self, Uri};
#[cfg(feature = "http1")]
pub use crate::types::{HeaderMap, Method, StatusCode, Version, body, header};

#[cfg(feature = "proxy")]
pub mod proxy;

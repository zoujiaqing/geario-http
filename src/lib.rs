//! HTTP protocol support.
#![allow(unreachable_pub)]

#[allow(unused_extern_crates)]
extern crate self as geario_http;

mod config;
pub(crate) mod helpers;
mod httpcodes;
mod httpmessage;
mod message;
mod payload;
mod request;
mod response;
mod service;

pub mod error;
pub mod h1;
pub mod types;

pub use self::config::{DateService, HttpServiceConfig, KeepAlive};
pub use self::error::ResponseError;
pub use self::httpmessage::HttpMessage;
pub use self::message::{ConnectionType, RequestHead, ResponseHead};
pub use self::payload::{Payload, PayloadStream};
pub use self::request::Request;
pub use self::response::{Response, ResponseBuilder};
pub use self::service::HttpService;
pub use geario::io::types::HttpProtocol;

pub const ALPN_PROTO_H1: &[&str] = &["http/1.1"];
pub const ALPN_PROTO_H2: &[&str] = &["h2"];
pub const ALPN_PROTOS: &[&str] = &["h2", "http/1.1"];

/// Header item
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HeaderItem {
    pub name: header::HeaderName,
    pub origin: geario::bytes::ByteString,
    pub value: header::HeaderValue,
}

// re-exports, matching what the upstream facade exposed
pub use crate::types::uri::{self, Uri};
pub use crate::types::{HeaderMap, Method, StatusCode, Version, body, header};

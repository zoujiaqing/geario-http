//! Http client errors
use std::{error::Error as StdError, io, ops::Deref, rc::Rc};

use serde_json::error::Error as JsonError;

#[cfg(feature = "openssl")]
use tls_openssl::ssl::{Error as SslError, HandshakeError};

use crate::error::{DecodeError, EncodeError, HttpError, PayloadError};
use geario::error::ErrorDiagnostic;
use geario::util::clone_io_error;
use geario::util::future::Either;

/// A set of errors that can occur during parsing json payloads
#[derive(thiserror::Error, Debug)]
pub enum JsonPayloadError {
    /// Content type error
    #[error("Content type error")]
    ContentType,
    /// Deserialize error
    #[error("Json deserialize error")]
    Deserialize(#[source] Option<JsonError>),
    /// Payload error
    #[error("Error that occur during reading payload")]
    Payload(
        #[from]
        #[source]
        ClientPayloadError,
    ),
}

impl Clone for JsonPayloadError {
    fn clone(&self) -> Self {
        match self {
            JsonPayloadError::ContentType => JsonPayloadError::ContentType,
            JsonPayloadError::Deserialize(_) => JsonPayloadError::Deserialize(None),
            JsonPayloadError::Payload(err) => JsonPayloadError::Payload(err.clone()),
        }
    }
}

impl From<JsonError> for JsonPayloadError {
    fn from(err: JsonError) -> JsonPayloadError {
        JsonPayloadError::Deserialize(Some(err))
    }
}

impl From<PayloadError> for JsonPayloadError {
    fn from(err: PayloadError) -> JsonPayloadError {
        JsonPayloadError::Payload(ClientPayloadError(err))
    }
}

impl ErrorDiagnostic for JsonPayloadError {
    fn signature(&self) -> &'static str {
        match self {
            JsonPayloadError::ContentType => "geario-client-JsonContentType",
            JsonPayloadError::Deserialize(_) => "geario-client-JsonDeserialize",
            JsonPayloadError::Payload(_) => "geario-client-JsonPayload",
        }
    }
}

#[derive(thiserror::Error, Clone, Debug)]
#[error("{0}")]
pub struct ClientPayloadError(
    #[from]
    #[source]
    pub(crate) PayloadError,
);

impl Deref for ClientPayloadError {
    type Target = PayloadError;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ErrorDiagnostic for ClientPayloadError {
    fn signature(&self) -> &'static str {
        "geario-client-Payload"
    }
}

/// A set of errors that can occur while building HTTP client
#[derive(thiserror::Error, Copy, Clone, Debug)]
pub enum ClientBuilderError {
    /// Connector failed
    #[error("Cannot construct connector")]
    ConnectorFailed,
}

/// A set of errors that can occur while connecting to an HTTP host
#[derive(thiserror::Error, Debug)]
pub enum ConnectError {
    /// SSL feature is not enabled
    #[error("SSL is not supported")]
    SslIsNotSupported,

    /// SSL error
    #[cfg(feature = "openssl")]
    #[error("{0}")]
    SslError(#[source] Rc<SslError>),

    /// SSL Handshake error
    #[cfg(feature = "openssl")]
    #[error("{0}")]
    SslHandshakeError(#[source] Rc<dyn StdError>),

    /// Failed to resolve the hostname
    #[error("Failed resolving hostname: {0}")]
    Resolver(
        #[from]
        #[source]
        io::Error,
    ),

    /// No dns records
    #[error("No dns records found for the input")]
    NoRecords,

    /// Connecting took too long
    #[error("Timeout while establishing connection")]
    Timeout,

    /// Connector has been disconnected
    #[error("Connector has been disconnected")]
    Disconnected(#[source] Option<io::Error>),

    /// Unresolved host name
    #[error("Connector received `Connect` method with unresolved host")]
    Unresolved,
}

impl ErrorDiagnostic for ConnectError {
    fn signature(&self) -> &'static str {
        match self {
            ConnectError::SslIsNotSupported => "geario-client-connect-SslIsNotSupported",
            #[cfg(feature = "openssl")]
            ConnectError::SslError(_) => "geario-client-connect-SslError",
            #[cfg(feature = "openssl")]
            ConnectError::SslHandshakeError(_) => "geario-client-connect-SslHandshakeError",
            ConnectError::Resolver(..) => "geario-client-connect-Resolver",
            ConnectError::NoRecords => "geario-client-connect-NoRecords",
            ConnectError::Timeout => "geario-client-connect-Timeout",
            ConnectError::Disconnected(_) => "geario-client-connect-Disconnected",
            ConnectError::Unresolved => "geario-client-connect-Unresolved",
        }
    }
}

impl Clone for ConnectError {
    fn clone(&self) -> Self {
        match self {
            ConnectError::SslIsNotSupported => ConnectError::SslIsNotSupported,
            #[cfg(feature = "openssl")]
            ConnectError::SslError(e) => ConnectError::SslError(e.clone()),
            #[cfg(feature = "openssl")]
            ConnectError::SslHandshakeError(e) => ConnectError::SslHandshakeError(e.clone()),
            ConnectError::Resolver(e) => ConnectError::Resolver(clone_io_error(e)),
            ConnectError::NoRecords => ConnectError::NoRecords,
            ConnectError::Timeout => ConnectError::Timeout,
            ConnectError::Disconnected(e) => {
                if let Some(e) = e {
                    ConnectError::Disconnected(Some(clone_io_error(e)))
                } else {
                    ConnectError::Disconnected(None)
                }
            }
            ConnectError::Unresolved => ConnectError::Unresolved,
        }
    }
}

#[cfg(feature = "openssl")]
impl From<SslError> for ConnectError {
    fn from(err: SslError) -> Self {
        ConnectError::SslError(Rc::new(err))
    }
}

impl From<geario::net::connect::ConnectError> for ConnectError {
    fn from(err: geario::net::connect::ConnectError) -> ConnectError {
        match err {
            geario::net::connect::ConnectError::Resolver(e) => ConnectError::Resolver(e),
            geario::net::connect::ConnectError::NoRecords => ConnectError::NoRecords,
            geario::net::connect::ConnectError::InvalidInput => panic!(),
            geario::net::connect::ConnectError::Unresolved => ConnectError::Unresolved,
            geario::net::connect::ConnectError::Io(e) => ConnectError::Disconnected(Some(e)),
        }
    }
}

#[cfg(feature = "openssl")]
impl<T: StdError + 'static> From<HandshakeError<T>> for ConnectError {
    fn from(err: HandshakeError<T>) -> ConnectError {
        ConnectError::SslHandshakeError(Rc::new(err))
    }
}

#[derive(Copy, Clone, Debug, thiserror::Error)]
pub enum InvalidUrl {
    #[error("Missing url scheme")]
    MissingScheme,
    #[error("Unknown url scheme")]
    UnknownScheme,
    #[error("Missing host name")]
    MissingHost,
    #[error("Url parse error: {0}")]
    Http(
        #[from]
        #[source]
        HttpError,
    ),
}

/// A set of errors that can occur during request sending and response reading
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Invalid URL
    #[error("Invalid URL: {0}")]
    Url(
        #[from]
        #[source]
        InvalidUrl,
    ),
    /// Failed to connect to host
    #[error("Failed to connect to host: {0}")]
    Connect(
        #[from]
        #[source]
        ConnectError,
    ),
    /// Error sending request
    #[error("Error sending request: {0}")]
    Send(
        #[from]
        #[source]
        io::Error,
    ),
    /// Error encoding request
    #[error("Error during request encoding: {0}")]
    Request(
        #[from]
        #[source]
        EncodeError,
    ),
    /// Error parsing response
    #[error("Error during response parsing: {0}")]
    Response(
        #[from]
        #[source]
        DecodeError,
    ),
    /// Http error
    #[error("{0}")]
    Http(
        #[from]
        #[source]
        HttpError,
    ),
    /// Response took too long
    #[error("Timeout while waiting for response")]
    Timeout,
    /// Tunnels are not supported for http2 connection
    #[error("Tunnels are not supported for http2 connection")]
    TunnelNotSupported,
    /// Error sending request body
    #[error("Error sending request body {0}")]
    Error(
        #[from]
        #[source]
        Rc<dyn StdError>,
    ),
}

impl Clone for ClientError {
    fn clone(&self) -> ClientError {
        match self {
            ClientError::Url(err) => ClientError::Url(*err),
            ClientError::Connect(err) => ClientError::Connect(err.clone()),
            ClientError::Request(err) => ClientError::Request(err.clone()),
            ClientError::Response(err) => ClientError::Response(*err),
            ClientError::Http(err) => ClientError::Http(*err),
            ClientError::Timeout => ClientError::Timeout,
            ClientError::TunnelNotSupported => ClientError::TunnelNotSupported,
            ClientError::Error(err) => ClientError::Error(err.clone()),
            ClientError::Send(err) => ClientError::Send(geario::util::clone_io_error(err)),
        }
    }
}

impl From<Either<EncodeError, io::Error>> for ClientError {
    fn from(err: Either<EncodeError, io::Error>) -> Self {
        match err {
            Either::Left(err) => ClientError::Request(err),
            Either::Right(err) => ClientError::Send(err),
        }
    }
}

impl From<Either<DecodeError, io::Error>> for ClientError {
    fn from(err: Either<DecodeError, io::Error>) -> Self {
        match err {
            Either::Left(err) => ClientError::Response(err),
            Either::Right(err) => ClientError::Send(err),
        }
    }
}

impl ErrorDiagnostic for ClientError {
    fn signature(&self) -> &'static str {
        match self {
            ClientError::Url(_) => "geario-client-Url",
            ClientError::Http(_) => "geario-client-Http",
            ClientError::Connect(err) => err.signature(),
            ClientError::Send(err) => err.signature(),
            ClientError::Request(_) => "geario-client-Request",
            ClientError::Response(_) => "geario-client-Response",
            ClientError::Timeout => "geario-client-Timeout",
            ClientError::TunnelNotSupported => "geario-client-TunnelNotSupported",
            ClientError::Error(_) => "geario-client-SendBody",
        }
    }
}

impl From<ClientBuilderError> for io::Error {
    fn from(err: ClientBuilderError) -> io::Error {
        io::Error::other(err)
    }
}

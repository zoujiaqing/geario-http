//! Shared C ABI surface: status codes, capability bits, version queries.
//!
//! Every type here crosses Rust, C and Kotlin/Native, so all of them are fixed
//! width. A C `enum` has implementation-defined width; freezing the values
//! without freezing the representation would not be freezing anything.

use std::ffi::c_char;

/// Synchronous status code returned by the geario C API.
pub type GearioHttpStatus = i32;

pub const GEARIO_HTTP_STATUS_OK: GearioHttpStatus = 0;
/// `abi_version` in a caller-supplied struct is not compatible with this build.
pub const GEARIO_HTTP_STATUS_ABI_MISMATCH: GearioHttpStatus = -1;
/// `struct_size` is below the minimum this ABI accepts.
pub const GEARIO_HTTP_STATUS_STRUCT_SIZE: GearioHttpStatus = -2;
/// `flags` contains a bit this build does not know. Never ignored silently: a
/// dropped flag can be the one that was carrying a security decision.
pub const GEARIO_HTTP_STATUS_UNKNOWN_FLAGS: GearioHttpStatus = -3;
/// NULL where a pointer is required, or an unparsable address, method or URL.
pub const GEARIO_HTTP_STATUS_INVALID_ARG: GearioHttpStatus = -4;
/// A combination this build does not implement. Ask the capability bits first.
pub const GEARIO_HTTP_STATUS_UNSUPPORTED: GearioHttpStatus = -5;
/// The client or server has been closed.
pub const GEARIO_HTTP_STATUS_CLOSED: GearioHttpStatus = -6;
/// A real allocation failure.
pub const GEARIO_HTTP_STATUS_OOM: GearioHttpStatus = -7;
/// Deliberate throttling to stay under a configured ceiling. Distinct from
/// OOM so operators are not sent hunting for a memory leak that is not there.
pub const GEARIO_HTTP_STATUS_THROTTLED: GearioHttpStatus = -8;

// -20 through -22 are left free. hyper4k uses them for NOT_FOUND,
// ALREADY_DONE and NOT_PAUSED; nothing here needs them yet, and taking them
// for something else would put a wrong meaning behind a number a host has
// already learned.

// Codes with no hyper4k counterpart start at -40, out of the way of anything
// hyper4k might add.

/// The handle was created on another worker thread. See `geario_server_start`.
pub const GEARIO_HTTP_STATUS_WRONG_THREAD: GearioHttpStatus = -40;
/// The responder is not in a state that allows this call, such as answering
/// one that is already streaming.
pub const GEARIO_HTTP_STATUS_WRONG_STATE: GearioHttpStatus = -41;

/// ABI revision, `(major << 16) | minor`, the encoding hyper4k uses. A major
/// change means a compiled caller cannot survive it.
pub const GEARIO_HTTP_ABI_VERSION: u32 = (1 << 16) | 0;

// ---------------------------------------------------------------------------
// Capability bits
// ---------------------------------------------------------------------------
//
// Derived from cargo features rather than written by hand, so a bit cannot
// claim something the build does not contain. Server and client have separate
// sets, numbered as hyper4k numbers them: the same Kotlin host reads both.

/// The server speaks HTTP/1.1.
pub const GEARIO_HTTP_SERVER_CAP_HTTP1: u64 = 1 << 0;
/// The server speaks HTTP/2 over cleartext with prior knowledge, on the same
/// port as HTTP/1.1.
pub const GEARIO_HTTP_SERVER_CAP_H2C: u64 = 1 << 1;
/// Streaming response bodies.
pub const GEARIO_HTTP_SERVER_CAP_STREAMING: u64 = 1 << 2;

/// The client speaks HTTP/1.1.
pub const GEARIO_HTTP_CLIENT_CAP_HTTP1: u64 = 1 << 0;
/// The client speaks HTTP/2, negotiated through ALPN over TLS.
pub const GEARIO_HTTP_CLIENT_CAP_HTTP2: u64 = 1 << 1;
/// `https://` targets.
pub const GEARIO_HTTP_CLIENT_CAP_TLS: u64 = 1 << 2;
/// A caller-supplied CA bundle is accepted.
pub const GEARIO_HTTP_CLIENT_CAP_CUSTOM_CA: u64 = 1 << 3;
/// Requests can be cancelled once started.
pub const GEARIO_HTTP_CLIENT_CAP_CANCEL: u64 = 1 << 4;
/// Response bodies are delivered chunk by chunk with backpressure.
pub const GEARIO_HTTP_CLIENT_CAP_STREAMING: u64 = 1 << 5;
/// An HTTP proxy can be configured.
pub const GEARIO_HTTP_CLIENT_CAP_PROXY: u64 = 1 << 6;

// ---------------------------------------------------------------------------
// Client option flags
// ---------------------------------------------------------------------------

/// Fail a TLS connection that does not negotiate HTTP/2, rather than falling
/// back to HTTP/1.1. Never a silent downgrade.
pub const GEARIO_HTTP_CLIENT_HTTP2_REQUIRED: u64 = 1 << 0;
/// Trust only the caller's CA bundle. Without it the bundle is added to the
/// built-in roots.
pub const GEARIO_HTTP_CLIENT_CA_REPLACE_SYSTEM: u64 = 1 << 1;

// ---------------------------------------------------------------------------
// Callback verdicts
// ---------------------------------------------------------------------------
//
// Headers and chunks get separate types on purpose. There is no coherent
// meaning for "pause before the next chunk" at the headers stage, and one
// shared enum would leave that combination undefined.

pub type GearioHttpHeadersAction = i32;
pub const GEARIO_HTTP_HEADERS_CONTINUE: GearioHttpHeadersAction = 0;
pub const GEARIO_HTTP_HEADERS_CANCEL: GearioHttpHeadersAction = 2;

pub type GearioHttpChunkAction = i32;
pub const GEARIO_HTTP_CHUNK_CONTINUE: GearioHttpChunkAction = 0;
pub const GEARIO_HTTP_CHUNK_PAUSE: GearioHttpChunkAction = 1;
pub const GEARIO_HTTP_CHUNK_CANCEL: GearioHttpChunkAction = 2;

// ---------------------------------------------------------------------------
// Error kinds, delivered through on_done
// ---------------------------------------------------------------------------
//
// Numbered as hyper4k numbers them. The distinctions are operational:
// "cannot connect", "the certificate is wrong" and "the response was cut
// off" are different problems for whoever is on call, and a caller deciding
// whether to replay a request needs to know whether the peer saw it.

pub type GearioHttpErrorKind = i32;
pub const GEARIO_HTTP_ERR_NONE: GearioHttpErrorKind = 0;
/// The host name did not resolve.
pub const GEARIO_HTTP_ERR_DNS: GearioHttpErrorKind = 1;
/// TCP connection refused, unreachable, or reset before a byte was sent.
pub const GEARIO_HTTP_ERR_CONNECT: GearioHttpErrorKind = 2;
/// The certificate does not chain to a trusted root.
pub const GEARIO_HTTP_ERR_TLS_CA: GearioHttpErrorKind = 3;
/// The certificate is not for the host that was asked for.
pub const GEARIO_HTTP_ERR_TLS_HOSTNAME: GearioHttpErrorKind = 4;
/// The certificate has expired, or is not yet valid.
pub const GEARIO_HTTP_ERR_TLS_EXPIRED: GearioHttpErrorKind = 5;
/// Any other TLS failure.
pub const GEARIO_HTTP_ERR_TLS_OTHER: GearioHttpErrorKind = 6;
/// HTTP/2 was required and the peer did not negotiate it.
pub const GEARIO_HTTP_ERR_ALPN_NO_H2: GearioHttpErrorKind = 7;
/// The peer violated the protocol.
pub const GEARIO_HTTP_ERR_PROTOCOL: GearioHttpErrorKind = 8;
/// The connect or request timeout elapsed.
pub const GEARIO_HTTP_ERR_TIMEOUT: GearioHttpErrorKind = 9;
/// No bytes arrived for longer than the read-idle timeout.
pub const GEARIO_HTTP_ERR_IDLE_TIMEOUT: GearioHttpErrorKind = 10;
/// Cancelled by the host, including by closing the client.
pub const GEARIO_HTTP_ERR_CANCELLED: GearioHttpErrorKind = 11;
/// The response had started when the connection failed. The request was
/// certainly processed; only the response is incomplete.
pub const GEARIO_HTTP_ERR_TRUNCATED: GearioHttpErrorKind = 12;
/// The connection failed after the request may have been sent and before a
/// response arrived. Whether the peer processed it cannot be known; this is
/// the only kind on which replaying a non-idempotent request is a judgement
/// call rather than a bug.
pub const GEARIO_HTTP_ERR_OUTCOME_UNKNOWN: GearioHttpErrorKind = 13;

// Kinds with no hyper4k counterpart start at 40, out of the way of anything
// hyper4k might add.

/// The URL or method could not be parsed.
pub const GEARIO_HTTP_ERR_INVALID_URL: GearioHttpErrorKind = 40;
/// The request asks for something this build does not do.
pub const GEARIO_HTTP_ERR_UNSUPPORTED: GearioHttpErrorKind = 41;

/// ABI revision of this build.
#[unsafe(no_mangle)]
pub extern "C" fn geario_http_abi_version() -> u32 {
    GEARIO_HTTP_ABI_VERSION
}

/// NUL-terminated crate version. Static storage; the caller must not free it.
#[unsafe(no_mangle)]
pub extern "C" fn geario_http_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

/// What the server side of this build can do.
///
/// Returns 0 when the crate was built without the `server` feature, which is
/// the only way a caller can tell that `geario_server_start` is absent rather
/// than merely failing.
#[unsafe(no_mangle)]
pub extern "C" fn geario_http_server_capabilities() -> u64 {
    #[cfg(feature = "server")]
    {
        GEARIO_HTTP_SERVER_CAP_HTTP1 | GEARIO_HTTP_SERVER_CAP_H2C | GEARIO_HTTP_SERVER_CAP_STREAMING
    }
    #[cfg(not(feature = "server"))]
    {
        0
    }
}

/// What the client side of this build can do.
#[unsafe(no_mangle)]
pub extern "C" fn geario_http_client_capabilities() -> u64 {
    #[cfg(feature = "client")]
    {
        GEARIO_HTTP_CLIENT_CAP_HTTP1
    }
    #[cfg(not(feature = "client"))]
    {
        0
    }
}

#[cfg(test)]
mod status_code_tests {
    use super::*;

    /// The numbers are pinned against hyper4k's, spelled out rather than
    /// computed, because the point is the exact value a compiled host has
    /// already learned.
    ///
    /// These two ABIs are read by the same Kotlin host. Where both have a
    /// code for the same condition it must be the same number: -7 meaning
    /// "closed" here and "out of memory" there would have operators hunting
    /// a memory leak that was a closed connection.
    #[test]
    fn shared_conditions_use_the_same_numbers_as_hyper4k() {
        assert_eq!(GEARIO_HTTP_STATUS_OK, 0);
        assert_eq!(GEARIO_HTTP_STATUS_ABI_MISMATCH, -1);
        assert_eq!(GEARIO_HTTP_STATUS_STRUCT_SIZE, -2);
        assert_eq!(GEARIO_HTTP_STATUS_UNKNOWN_FLAGS, -3);
        assert_eq!(GEARIO_HTTP_STATUS_INVALID_ARG, -4);
        assert_eq!(GEARIO_HTTP_STATUS_UNSUPPORTED, -5);
        assert_eq!(GEARIO_HTTP_STATUS_CLOSED, -6);
        assert_eq!(GEARIO_HTTP_STATUS_OOM, -7);
        assert_eq!(GEARIO_HTTP_STATUS_THROTTLED, -8);
    }

    /// Codes with no hyper4k counterpart stay out of the range hyper4k has
    /// already spent, including -20 through -22.
    #[test]
    fn geario_only_codes_stay_clear_of_hyper4k_s_range() {
        for code in [
            GEARIO_HTTP_STATUS_WRONG_THREAD,
            GEARIO_HTTP_STATUS_WRONG_STATE,
        ] {
            assert!(code <= -40, "{code} is inside hyper4k's range");
        }
    }

    /// Capability bits, flags and error kinds are read by the same host that
    /// reads hyper4k's, so where both name a thing it has to be the same
    /// number. Spelled out rather than computed, for the same reason as the
    /// status codes.
    #[test]
    fn capabilities_flags_and_error_kinds_match_hyper4k() {
        assert_eq!(GEARIO_HTTP_SERVER_CAP_HTTP1, 1);
        assert_eq!(GEARIO_HTTP_SERVER_CAP_H2C, 2);
        assert_eq!(GEARIO_HTTP_SERVER_CAP_STREAMING, 4);

        assert_eq!(GEARIO_HTTP_CLIENT_CAP_HTTP1, 1);
        assert_eq!(GEARIO_HTTP_CLIENT_CAP_HTTP2, 2);
        assert_eq!(GEARIO_HTTP_CLIENT_CAP_TLS, 4);
        assert_eq!(GEARIO_HTTP_CLIENT_CAP_CUSTOM_CA, 8);
        assert_eq!(GEARIO_HTTP_CLIENT_CAP_CANCEL, 16);
        assert_eq!(GEARIO_HTTP_CLIENT_CAP_STREAMING, 32);
        assert_eq!(GEARIO_HTTP_CLIENT_CAP_PROXY, 64);

        assert_eq!(GEARIO_HTTP_CLIENT_HTTP2_REQUIRED, 1);
        assert_eq!(GEARIO_HTTP_CLIENT_CA_REPLACE_SYSTEM, 2);

        for (kind, n) in [
            (GEARIO_HTTP_ERR_NONE, 0),
            (GEARIO_HTTP_ERR_DNS, 1),
            (GEARIO_HTTP_ERR_CONNECT, 2),
            (GEARIO_HTTP_ERR_TLS_CA, 3),
            (GEARIO_HTTP_ERR_TLS_HOSTNAME, 4),
            (GEARIO_HTTP_ERR_TLS_EXPIRED, 5),
            (GEARIO_HTTP_ERR_TLS_OTHER, 6),
            (GEARIO_HTTP_ERR_ALPN_NO_H2, 7),
            (GEARIO_HTTP_ERR_PROTOCOL, 8),
            (GEARIO_HTTP_ERR_TIMEOUT, 9),
            (GEARIO_HTTP_ERR_IDLE_TIMEOUT, 10),
            (GEARIO_HTTP_ERR_CANCELLED, 11),
            (GEARIO_HTTP_ERR_TRUNCATED, 12),
            (GEARIO_HTTP_ERR_OUTCOME_UNKNOWN, 13),
        ] {
            assert_eq!(kind, n);
        }
        for kind in [GEARIO_HTTP_ERR_INVALID_URL, GEARIO_HTTP_ERR_UNSUPPORTED] {
            assert!(kind >= 40, "{kind} is inside hyper4k's range");
        }
    }

    #[test]
    fn abi_version_is_major_shifted_by_sixteen() {
        assert_eq!(GEARIO_HTTP_ABI_VERSION >> 16, 1, "major");
        assert_eq!(GEARIO_HTTP_ABI_VERSION & 0xffff, 0, "minor");
    }
}

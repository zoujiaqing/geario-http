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
// These are derived from cargo features rather than written by hand, so a bit
// cannot claim something the build does not contain.

/// HTTP/1.1 is available.
pub const GEARIO_HTTP_CAP_HTTP1: u64 = 1 << 0;
/// HTTP/2 is available.
pub const GEARIO_HTTP_CAP_HTTP2: u64 = 1 << 1;
/// TLS is available.
pub const GEARIO_HTTP_CAP_TLS: u64 = 1 << 2;
/// Streaming response bodies are available.
pub const GEARIO_HTTP_CAP_STREAMING: u64 = 1 << 3;

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
// Error reporting
// ---------------------------------------------------------------------------

pub type GearioHttpErrorKind = i32;
pub const GEARIO_HTTP_ERR_NONE: GearioHttpErrorKind = 0;
pub const GEARIO_HTTP_ERR_CONNECT: GearioHttpErrorKind = 1;
pub const GEARIO_HTTP_ERR_TIMEOUT: GearioHttpErrorKind = 2;
pub const GEARIO_HTTP_ERR_PROTOCOL: GearioHttpErrorKind = 3;
pub const GEARIO_HTTP_ERR_IO: GearioHttpErrorKind = 4;
pub const GEARIO_HTTP_ERR_CANCELLED: GearioHttpErrorKind = 5;
pub const GEARIO_HTTP_ERR_INVALID_URL: GearioHttpErrorKind = 6;
pub const GEARIO_HTTP_ERR_UNSUPPORTED: GearioHttpErrorKind = 7;

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
        GEARIO_HTTP_CAP_HTTP1 | GEARIO_HTTP_CAP_STREAMING
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
        GEARIO_HTTP_CAP_HTTP1
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

    #[test]
    fn abi_version_is_major_shifted_by_sixteen() {
        assert_eq!(GEARIO_HTTP_ABI_VERSION >> 16, 1, "major");
        assert_eq!(GEARIO_HTTP_ABI_VERSION & 0xffff, 0, "minor");
    }
}

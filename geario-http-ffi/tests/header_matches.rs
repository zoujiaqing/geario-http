//! The C header is written by hand, so nothing stops it drifting from the
//! Rust constants. A C caller that compiles against a stale number gets no
//! warning: it just reads the wrong meaning out of a return value.

use std::collections::HashMap;

use geario_http_ffi::*;

/// Every `#define NAME value` in the header, with the value as written.
fn defines() -> HashMap<String, String> {
    let header = include_str!("../include/geario_http.h");
    header
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("#define ")?;
            let mut parts = rest.split_whitespace();
            let name = parts.next()?.to_owned();
            // Values are either a plain number, "(-7)", or a shift written as
            // "(UINT64_C(1) << 3)"; the shift is evaluated so bits can be
            // compared as numbers too.
            let rest: String = parts.collect::<Vec<_>>().join(" ");
            let value = if let Some(shift) = rest
                .strip_prefix("(UINT64_C(1) << ")
                .and_then(|r| r.split(')').next())
            {
                (1u64 << shift.trim().parse::<u32>().ok()?).to_string()
            } else {
                rest.split("/*")
                    .next()?
                    .trim()
                    .trim_matches(['(', ')'])
                    .to_owned()
            };
            Some((name, value))
        })
        .collect()
}

fn assert_i32(defines: &HashMap<String, String>, name: &str, expected: i32) {
    let found = defines
        .get(name)
        .unwrap_or_else(|| panic!("{name} is missing from geario_http.h"));
    let found: i32 = found
        .parse()
        .unwrap_or_else(|_| panic!("{name} is `{found}` in the header, which is not a number"));
    assert_eq!(found, expected, "{name} disagrees with the Rust constant");
}

#[test]
fn the_header_status_codes_match_the_rust_ones() {
    let defines = defines();
    for (name, expected) in [
        ("GEARIO_HTTP_STATUS_OK", GEARIO_HTTP_STATUS_OK),
        (
            "GEARIO_HTTP_STATUS_ABI_MISMATCH",
            GEARIO_HTTP_STATUS_ABI_MISMATCH,
        ),
        (
            "GEARIO_HTTP_STATUS_STRUCT_SIZE",
            GEARIO_HTTP_STATUS_STRUCT_SIZE,
        ),
        (
            "GEARIO_HTTP_STATUS_UNKNOWN_FLAGS",
            GEARIO_HTTP_STATUS_UNKNOWN_FLAGS,
        ),
        (
            "GEARIO_HTTP_STATUS_INVALID_ARG",
            GEARIO_HTTP_STATUS_INVALID_ARG,
        ),
        (
            "GEARIO_HTTP_STATUS_UNSUPPORTED",
            GEARIO_HTTP_STATUS_UNSUPPORTED,
        ),
        ("GEARIO_HTTP_STATUS_CLOSED", GEARIO_HTTP_STATUS_CLOSED),
        ("GEARIO_HTTP_STATUS_OOM", GEARIO_HTTP_STATUS_OOM),
        ("GEARIO_HTTP_STATUS_THROTTLED", GEARIO_HTTP_STATUS_THROTTLED),
        (
            "GEARIO_HTTP_STATUS_WRONG_THREAD",
            GEARIO_HTTP_STATUS_WRONG_THREAD,
        ),
        (
            "GEARIO_HTTP_STATUS_WRONG_STATE",
            GEARIO_HTTP_STATUS_WRONG_STATE,
        ),
    ] {
        assert_i32(&defines, name, expected);
    }
}

#[test]
fn the_header_capabilities_flags_verdicts_and_error_kinds_match() {
    let defines = defines();
    let u = |v: u64| i32::try_from(v).unwrap();
    for (name, expected) in [
        (
            "GEARIO_HTTP_SERVER_CAP_HTTP1",
            u(GEARIO_HTTP_SERVER_CAP_HTTP1),
        ),
        ("GEARIO_HTTP_SERVER_CAP_H2C", u(GEARIO_HTTP_SERVER_CAP_H2C)),
        (
            "GEARIO_HTTP_SERVER_CAP_STREAMING",
            u(GEARIO_HTTP_SERVER_CAP_STREAMING),
        ),
        (
            "GEARIO_HTTP_CLIENT_CAP_HTTP1",
            u(GEARIO_HTTP_CLIENT_CAP_HTTP1),
        ),
        (
            "GEARIO_HTTP_CLIENT_CAP_HTTP2",
            u(GEARIO_HTTP_CLIENT_CAP_HTTP2),
        ),
        ("GEARIO_HTTP_CLIENT_CAP_TLS", u(GEARIO_HTTP_CLIENT_CAP_TLS)),
        (
            "GEARIO_HTTP_CLIENT_CAP_CUSTOM_CA",
            u(GEARIO_HTTP_CLIENT_CAP_CUSTOM_CA),
        ),
        (
            "GEARIO_HTTP_CLIENT_CAP_CANCEL",
            u(GEARIO_HTTP_CLIENT_CAP_CANCEL),
        ),
        (
            "GEARIO_HTTP_CLIENT_CAP_STREAMING",
            u(GEARIO_HTTP_CLIENT_CAP_STREAMING),
        ),
        (
            "GEARIO_HTTP_CLIENT_CAP_PROXY",
            u(GEARIO_HTTP_CLIENT_CAP_PROXY),
        ),
        (
            "GEARIO_HTTP_CLIENT_HTTP2_REQUIRED",
            u(GEARIO_HTTP_CLIENT_HTTP2_REQUIRED),
        ),
        (
            "GEARIO_HTTP_CLIENT_CA_REPLACE_SYSTEM",
            u(GEARIO_HTTP_CLIENT_CA_REPLACE_SYSTEM),
        ),
        ("GEARIO_HTTP_HEADERS_CONTINUE", GEARIO_HTTP_HEADERS_CONTINUE),
        ("GEARIO_HTTP_HEADERS_CANCEL", GEARIO_HTTP_HEADERS_CANCEL),
        ("GEARIO_HTTP_CHUNK_CONTINUE", GEARIO_HTTP_CHUNK_CONTINUE),
        ("GEARIO_HTTP_CHUNK_PAUSE", GEARIO_HTTP_CHUNK_PAUSE),
        ("GEARIO_HTTP_CHUNK_CANCEL", GEARIO_HTTP_CHUNK_CANCEL),
        ("GEARIO_HTTP_ERR_NONE", GEARIO_HTTP_ERR_NONE),
        ("GEARIO_HTTP_ERR_DNS", GEARIO_HTTP_ERR_DNS),
        ("GEARIO_HTTP_ERR_CONNECT", GEARIO_HTTP_ERR_CONNECT),
        ("GEARIO_HTTP_ERR_TLS_CA", GEARIO_HTTP_ERR_TLS_CA),
        ("GEARIO_HTTP_ERR_TLS_HOSTNAME", GEARIO_HTTP_ERR_TLS_HOSTNAME),
        ("GEARIO_HTTP_ERR_TLS_EXPIRED", GEARIO_HTTP_ERR_TLS_EXPIRED),
        ("GEARIO_HTTP_ERR_TLS_OTHER", GEARIO_HTTP_ERR_TLS_OTHER),
        ("GEARIO_HTTP_ERR_ALPN_NO_H2", GEARIO_HTTP_ERR_ALPN_NO_H2),
        ("GEARIO_HTTP_ERR_PROTOCOL", GEARIO_HTTP_ERR_PROTOCOL),
        ("GEARIO_HTTP_ERR_TIMEOUT", GEARIO_HTTP_ERR_TIMEOUT),
        ("GEARIO_HTTP_ERR_IDLE_TIMEOUT", GEARIO_HTTP_ERR_IDLE_TIMEOUT),
        ("GEARIO_HTTP_ERR_CANCELLED", GEARIO_HTTP_ERR_CANCELLED),
        ("GEARIO_HTTP_ERR_TRUNCATED", GEARIO_HTTP_ERR_TRUNCATED),
        (
            "GEARIO_HTTP_ERR_OUTCOME_UNKNOWN",
            GEARIO_HTTP_ERR_OUTCOME_UNKNOWN,
        ),
        ("GEARIO_HTTP_ERR_INVALID_URL", GEARIO_HTTP_ERR_INVALID_URL),
        ("GEARIO_HTTP_ERR_UNSUPPORTED", GEARIO_HTTP_ERR_UNSUPPORTED),
    ] {
        assert_i32(&defines, name, expected);
    }
}

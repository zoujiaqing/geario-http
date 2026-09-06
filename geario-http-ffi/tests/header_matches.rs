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
            let value = parts.next()?.trim_matches(['(', ')']).to_owned();
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
        ("GEARIO_HTTP_STATUS_ABI_MISMATCH", GEARIO_HTTP_STATUS_ABI_MISMATCH),
        ("GEARIO_HTTP_STATUS_STRUCT_SIZE", GEARIO_HTTP_STATUS_STRUCT_SIZE),
        ("GEARIO_HTTP_STATUS_UNKNOWN_FLAGS", GEARIO_HTTP_STATUS_UNKNOWN_FLAGS),
        ("GEARIO_HTTP_STATUS_INVALID_ARG", GEARIO_HTTP_STATUS_INVALID_ARG),
        ("GEARIO_HTTP_STATUS_UNSUPPORTED", GEARIO_HTTP_STATUS_UNSUPPORTED),
        ("GEARIO_HTTP_STATUS_CLOSED", GEARIO_HTTP_STATUS_CLOSED),
        ("GEARIO_HTTP_STATUS_OOM", GEARIO_HTTP_STATUS_OOM),
        ("GEARIO_HTTP_STATUS_THROTTLED", GEARIO_HTTP_STATUS_THROTTLED),
        ("GEARIO_HTTP_STATUS_WRONG_THREAD", GEARIO_HTTP_STATUS_WRONG_THREAD),
        ("GEARIO_HTTP_STATUS_WRONG_STATE", GEARIO_HTTP_STATUS_WRONG_STATE),
    ] {
        assert_i32(&defines, name, expected);
    }
}

#[test]
fn the_header_callback_verdicts_and_error_kinds_match() {
    let defines = defines();
    for (name, expected) in [
        ("GEARIO_HTTP_CHUNK_CONTINUE", i32::from(GEARIO_HTTP_CHUNK_CONTINUE)),
        ("GEARIO_HTTP_CHUNK_PAUSE", i32::from(GEARIO_HTTP_CHUNK_PAUSE)),
        ("GEARIO_HTTP_CHUNK_CANCEL", i32::from(GEARIO_HTTP_CHUNK_CANCEL)),
        ("GEARIO_HTTP_ERR_NONE", i32::from(GEARIO_HTTP_ERR_NONE)),
        ("GEARIO_HTTP_ERR_CONNECT", i32::from(GEARIO_HTTP_ERR_CONNECT)),
        ("GEARIO_HTTP_ERR_TIMEOUT", i32::from(GEARIO_HTTP_ERR_TIMEOUT)),
        ("GEARIO_HTTP_ERR_PROTOCOL", i32::from(GEARIO_HTTP_ERR_PROTOCOL)),
        ("GEARIO_HTTP_ERR_IO", i32::from(GEARIO_HTTP_ERR_IO)),
        ("GEARIO_HTTP_ERR_CANCELLED", i32::from(GEARIO_HTTP_ERR_CANCELLED)),
        ("GEARIO_HTTP_ERR_INVALID_URL", i32::from(GEARIO_HTTP_ERR_INVALID_URL)),
        ("GEARIO_HTTP_ERR_UNSUPPORTED", i32::from(GEARIO_HTTP_ERR_UNSUPPORTED)),
    ] {
        assert_i32(&defines, name, expected);
    }
}

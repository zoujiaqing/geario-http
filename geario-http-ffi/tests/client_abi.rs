//! The parts of the client ABI a host can get wrong.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use geario_http_ffi::*;

fn default_options() -> GearioHttpClientOptions {
    let mut opts = std::mem::MaybeUninit::<GearioHttpClientOptions>::zeroed();
    let st = unsafe {
        geario_http_client_options_init(
            opts.as_mut_ptr(),
            std::mem::size_of::<GearioHttpClientOptions>() as u32,
        )
    };
    assert_eq!(st, GEARIO_HTTP_STATUS_OK);
    unsafe { opts.assume_init() }
}

#[test]
fn options_init_fills_defaults() {
    let opts = default_options();
    assert_eq!(opts.abi_version, geario_http_abi_version());
    assert_eq!(opts.flags, 0);
    assert_ne!(
        opts.max_inflight_requests, 0,
        "a zero ceiling would block everything"
    );
}

#[test]
fn a_short_struct_is_refused_rather_than_read_past() {
    let mut opts = default_options();
    opts.struct_size = 4; // shorter than the frozen prefix
    let mut out = std::ptr::null_mut();
    let st = unsafe { geario_http_client_new(&opts, &mut out) };
    assert_eq!(st, GEARIO_HTTP_STATUS_STRUCT_SIZE);
    assert!(out.is_null());
}

#[test]
fn a_foreign_abi_version_is_refused() {
    let mut opts = default_options();
    opts.abi_version = geario_http_abi_version() + 1;
    let mut out = std::ptr::null_mut();
    let st = unsafe { geario_http_client_new(&opts, &mut out) };
    assert_eq!(st, GEARIO_HTTP_STATUS_ABI_MISMATCH);
}

#[test]
fn an_unknown_flag_is_refused_not_ignored() {
    // A flag this build does not know may be the one carrying a security
    // decision, so dropping it quietly would be the wrong default.
    let mut opts = default_options();
    opts.flags = 1 << 40;
    let mut out = std::ptr::null_mut();
    let st = unsafe { geario_http_client_new(&opts, &mut out) };
    assert_eq!(st, GEARIO_HTTP_STATUS_UNKNOWN_FLAGS);
}

#[test]
fn null_arguments_are_rejected() {
    let mut out = std::ptr::null_mut();
    assert_eq!(
        unsafe { geario_http_client_new(std::ptr::null(), &mut out) },
        GEARIO_HTTP_STATUS_INVALID_ARG
    );
    let opts = default_options();
    assert_eq!(
        unsafe { geario_http_client_new(&opts, std::ptr::null_mut()) },
        GEARIO_HTTP_STATUS_INVALID_ARG
    );
    // Counters answer for a NULL client instead of faulting.
    assert_eq!(
        unsafe { geario_http_client_inflight_count(std::ptr::null_mut()) },
        0
    );
    assert_eq!(
        unsafe { geario_http_client_paused_stream_count(std::ptr::null_mut()) },
        0
    );
    // Freeing NULL is a no-op.
    unsafe { geario_http_client_free(std::ptr::null_mut()) };
}

#[test]
fn sending_over_the_ceiling_is_throttled_not_queued() {
    static DONE: AtomicU32 = AtomicU32::new(0);

    extern "C" fn on_done(_ud: *mut c_void, _id: u64, _e: *const GearioHttpError) {
        DONE.fetch_add(1, Ordering::SeqCst);
    }

    let mut opts = default_options();
    opts.max_inflight_requests = 1;
    let mut client = std::ptr::null_mut();
    assert_eq!(
        unsafe { geario_http_client_new(&opts, &mut client) },
        GEARIO_HTTP_STATUS_OK
    );

    // Nothing is listening on this port, so the first request stays in flight
    // long enough for the second to hit the ceiling.
    let url = b"http://127.0.0.1:1/";
    let mut statuses = Vec::new();
    for _ in 0..8 {
        let mut req = std::mem::MaybeUninit::<GearioHttpClientRequest>::zeroed();
        unsafe {
            geario_http_client_request_init(
                req.as_mut_ptr(),
                std::mem::size_of::<GearioHttpClientRequest>() as u32,
            );
        }
        let mut req = unsafe { req.assume_init() };
        req.url.ptr = url.as_ptr();
        req.url.len = url.len();
        let mut id = 0u64;
        statuses.push(unsafe {
            geario_http_client_send(
                client,
                &req,
                None,
                None,
                Some(on_done),
                std::ptr::null_mut(),
                &mut id,
            )
        });
    }

    unsafe { geario_http_client_free(client) };

    assert!(
        statuses.iter().any(|s| *s == GEARIO_HTTP_STATUS_THROTTLED),
        "a ceiling of one never throttled across eight sends: {statuses:?}"
    );
}

#[test]
fn a_closed_client_refuses_new_work() {
    let opts = default_options();
    let mut client = std::ptr::null_mut();
    assert_eq!(
        unsafe { geario_http_client_new(&opts, &mut client) },
        GEARIO_HTTP_STATUS_OK
    );
    unsafe { geario_http_client_close(client) };

    let url = b"http://127.0.0.1:1/";
    let mut req = std::mem::MaybeUninit::<GearioHttpClientRequest>::zeroed();
    unsafe {
        geario_http_client_request_init(
            req.as_mut_ptr(),
            std::mem::size_of::<GearioHttpClientRequest>() as u32,
        );
    }
    let mut req = unsafe { req.assume_init() };
    req.url.ptr = url.as_ptr();
    req.url.len = url.len();
    let mut id = 0u64;
    let st = unsafe {
        geario_http_client_send(
            client,
            &req,
            None,
            None,
            None,
            std::ptr::null_mut(),
            &mut id,
        )
    };
    assert_eq!(st, GEARIO_HTTP_STATUS_CLOSED);

    unsafe { geario_http_client_free(client) };
}

#[test]
fn options_default_to_bounded_retries() {
    let opts = default_options();
    // Zero would be a valid choice, but silently is not: the default has to be
    // something a reader can see.
    assert_eq!(opts.max_retries, 2);
}

/// A non-idempotent method must not be repeated.
///
/// The port is closed, so every attempt fails at connect. A GET burns its
/// retries and a POST does not, which is visible in how long each takes only
/// indirectly — so this checks the classification directly instead.
#[test]
fn only_idempotent_methods_are_retried() {
    for (method, retried) in [
        (&b"GET"[..], true),
        (&b"HEAD"[..], true),
        (&b"PUT"[..], true),
        (&b"DELETE"[..], true),
        (&b"OPTIONS"[..], true),
        (&b"TRACE"[..], true),
        (&b"POST"[..], false),
        (&b"PATCH"[..], false),
    ] {
        assert_eq!(
            geario_http_ffi::is_idempotent_for_tests(method),
            retried,
            "{} classified wrongly",
            String::from_utf8_lossy(method)
        );
    }
}

#[test]
fn a_proxy_url_that_cannot_be_honoured_is_refused() {
    // Ignoring it would connect direct, which is the one outcome the caller
    // did not ask for.
    for bad in [
        &b"https://proxy.example"[..], // TLS to the proxy is another shape
        &b"http://user:pass@proxy.example"[..], // credentials would be dropped
        &b"socks5://proxy.example"[..],
        &b"not a url"[..],
    ] {
        let mut opts = default_options();
        opts.proxy_url = bad.as_ptr();
        opts.proxy_url_len = bad.len();
        let mut out = std::ptr::null_mut();
        assert_eq!(
            unsafe { geario_http_client_new(&opts, &mut out) },
            GEARIO_HTTP_STATUS_INVALID_ARG,
            "{} was accepted",
            String::from_utf8_lossy(bad)
        );
        assert!(out.is_null());
    }
}

#[test]
fn a_usable_proxy_url_is_accepted() {
    let url = b"http://127.0.0.1:3128";
    let mut opts = default_options();
    opts.proxy_url = url.as_ptr();
    opts.proxy_url_len = url.len();
    let mut client = std::ptr::null_mut();
    assert_eq!(
        unsafe { geario_http_client_new(&opts, &mut client) },
        GEARIO_HTTP_STATUS_OK
    );
    unsafe { geario_http_client_free(client) };
}

#[test]
fn options_default_to_no_proxy() {
    let opts = default_options();
    assert!(opts.proxy_url.is_null());
    assert_eq!(opts.proxy_url_len, 0);
}

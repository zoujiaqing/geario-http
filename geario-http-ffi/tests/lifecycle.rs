use std::ffi::{c_void, CString};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use geario_http_ffi::*;

static HITS: AtomicU64 = AtomicU64::new(0);

extern "C" fn on_request(_user: *mut c_void, req: *const GearioHttpRequest) {
    HITS.fetch_add(1, Ordering::Relaxed);
    let responder = unsafe { (*req).responder };
    let body = b"ok";
    unsafe {
        geario_http_respond(
            responder,
            200,
            std::ptr::null(),
            0,
            body.as_ptr(),
            body.len(),
        );
    }
}

#[test]
fn start_then_stop_returns() {
    let host = CString::new("127.0.0.1").unwrap();
    let srv = unsafe {
        geario_http_server_start(host.as_ptr(), 8123, on_request, std::ptr::null_mut())
    };
    assert!(!srv.is_null(), "server did not start");

    // Prove it is actually serving before testing shutdown.
    let resp = std::process::Command::new("curl")
        .args(["-s", "http://127.0.0.1:8123/probe"])
        .output()
        .expect("curl");
    assert_eq!(String::from_utf8_lossy(&resp.stdout), "ok");

    let t = Instant::now();
    unsafe { geario_http_server_stop(srv) };
    let took = t.elapsed();
    assert!(
        took < Duration::from_secs(5),
        "server_stop took {took:?}, it is hanging"
    );
}

/// The library must not take the host's signal handlers.
///
/// geario's server installs handlers for SIGINT, SIGTERM and SIGQUIT by
/// default. Inside a linked library that silently replaces whatever the host
/// installed, which left a C program unable to shut itself down.
///
/// Rather than raising a signal, this reads back the installed disposition:
/// `signal()` returns the handler it replaced, so installing SIG_DFL once
/// reports who owned SIGINT at that moment.
#[test]
fn does_not_steal_host_signal_handlers() {
    const SIGINT: i32 = 2;

    extern "C" fn host_handler(_sig: i32) {}
    let host_addr = host_handler as *const () as usize;

    let previous = unsafe { c_signal(SIGINT, host_addr) };
    assert_ne!(previous, usize::MAX, "could not install the test handler");

    let host = CString::new("127.0.0.1").unwrap();
    let srv = unsafe {
        geario_http_server_start(host.as_ptr(), 8124, on_request, std::ptr::null_mut())
    };
    assert!(!srv.is_null());

    // Read back who owns SIGINT now, then put things back as they were.
    let owner = unsafe { c_signal(SIGINT, host_addr) };
    unsafe { geario_http_server_stop(srv) };
    unsafe { c_signal(SIGINT, previous) };

    assert_eq!(
        owner, host_addr,
        "starting the server replaced the host's SIGINT handler"
    );
}

unsafe extern "C" {
    #[link_name = "signal"]
    fn c_signal(sig: i32, handler: usize) -> usize;
}

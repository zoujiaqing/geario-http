#![cfg(feature = "native-h1")]

use std::ffi::{CString, c_void};

use geario_http_ffi::{
    GearioHttpNativeRequest, GearioHttpNativeRequestCallback, geario_http_native_h1_respond,
    geario_http_native_h1_start, geario_http_native_h1_stop,
};

extern "C" fn handler(_user: *mut c_void, request: *const GearioHttpNativeRequest) {
    let request = unsafe { &*request };
    let responder = request.responder;
    let path = unsafe { std::slice::from_raw_parts(request.path.ptr, request.path.len) }.to_vec();
    std::thread::spawn(move || {
        assert_eq!(path, b"/native-h1");
        let body = b"native-h1";
        unsafe {
            geario_http_native_h1_respond(
                responder,
                200,
                std::ptr::null(),
                0,
                body.as_ptr(),
                body.len(),
            );
        }
    });
}

#[test]
fn native_h1_serves_and_accepts_cross_thread_response() {
    let host = CString::new("127.0.0.1").unwrap();
    let server = unsafe {
        geario_http_native_h1_start(
            host.as_ptr(),
            8130,
            handler as GearioHttpNativeRequestCallback,
            std::ptr::null_mut(),
        )
    };
    assert!(!server.is_null(), "native H1 server did not start");

    let response = std::process::Command::new("curl")
        .args(["-sS", "--max-time", "5", "http://127.0.0.1:8130/native-h1"])
        .output()
        .expect("curl");
    unsafe { geario_http_native_h1_stop(server) };

    assert!(response.status.success(), "curl failed: {response:?}");
    assert_eq!(response.stdout, b"native-h1");
}

//! Direct FFI adapter for geario-http's native HTTP/1.1 service.
//!
//! This module intentionally has a separate entry point from `server.rs`.
//! The latter is the Hyper compatibility path; this one drives geario-http's
//! own H1 parser and response writer so the two paths can be benchmarked with
//! the same host callback.

use std::ffi::{CStr, c_char, c_void};
use std::io;
use std::net::SocketAddr;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use geario::bytes::BytesMut;
use geario::service::cfg::SharedCfg;
use geario::util::future::Stream;
use geario_http::{HttpService, Request, Response, StatusCode};

use crate::slice::GearioHttpSlice;

#[repr(C)]
pub struct GearioHttpNativeRequest {
    pub method: GearioHttpSlice,
    pub path: GearioHttpSlice,
    pub query: GearioHttpSlice,
    pub headers: GearioHttpSlice,
    pub body: GearioHttpSlice,
    pub responder: u64,
}

pub type GearioHttpNativeRequestCallback =
    extern "C" fn(*mut c_void, *const GearioHttpNativeRequest);

struct NativeReply {
    status: u16,
    headers: Vec<u8>,
    body: Vec<u8>,
}

/// One response rendezvous per request. Keeping the sender with the request
/// removes the global lock and ID table from the hot path while Arc keeps the
/// slot alive across the host callback.
struct NativeResponder {
    sender: Mutex<Option<async_channel::Sender<NativeReply>>>,
}

#[derive(Copy, Clone)]
struct NativeHandler {
    callback: GearioHttpNativeRequestCallback,
    user_data: usize,
}

unsafe impl Send for NativeHandler {}
unsafe impl Sync for NativeHandler {}

pub struct GearioHttpNativeServer {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Start geario-http's native H1 service without the Hyper compatibility layer.
///
/// The callback receives borrowed request fields. Its responder may be
/// completed from another thread; only owned bytes cross that boundary.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geario_http_native_h1_start(
    host: *const c_char,
    port: u16,
    on_request: GearioHttpNativeRequestCallback,
    user_data: *mut c_void,
) -> *mut GearioHttpNativeServer {
    let host = if host.is_null() {
        "0.0.0.0".to_owned()
    } else {
        match unsafe { CStr::from_ptr(host) }.to_str() {
            Ok(value) => value.to_owned(),
            Err(_) => return std::ptr::null_mut(),
        }
    };
    let addr: SocketAddr = match format!("{host}:{port}").parse() {
        Ok(value) => value,
        Err(_) => return std::ptr::null_mut(),
    };
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<bool>();
    let handler = NativeHandler {
        callback: on_request,
        user_data: user_data as usize,
    };
    let thread = std::thread::Builder::new()
        .name("geario-http-native-h1".into())
        .spawn(move || {
            geario::rt::System::build()
                .name("geario-http-native-h1")
                .build(geario::rt::DefaultRuntime)
                .block_on(async move {
                    let server = match geario::server::net::build()
                        .disable_signals()
                        .workers(std::thread::available_parallelism().map_or(1, |n| n.get()))
                        .bind(
                            "native-h1",
                            addr,
                            SharedCfg::new("GEARIO_NATIVE_H1"),
                            async move |_| {
                                HttpService::new(move |request: Request| {
                                    native_dispatch(
                                        request,
                                        handler.callback,
                                        handler.user_data as *mut c_void,
                                    )
                                })
                                .build()
                            },
                        ) {
                        Ok(value) => value.run(),
                        Err(_) => {
                            let _ = ready_tx.send(false);
                            return;
                        }
                    };
                    let _ = ready_tx.send(true);
                    let _ = geario::rt::spawn_blocking(move || stop_rx.recv()).await;
                    server.stop(false).await;
                });
        });
    let thread = match thread {
        Ok(value) => value,
        Err(_) => return std::ptr::null_mut(),
    };
    if ready_rx.recv() != Ok(true) {
        return std::ptr::null_mut();
    }
    Box::into_raw(Box::new(GearioHttpNativeServer {
        stop: Some(stop_tx),
        thread: Some(thread),
    }))
}

async fn native_dispatch(
    mut request: Request,
    callback: GearioHttpNativeRequestCallback,
    user_data: *mut c_void,
) -> Result<Response, io::Error> {
    let mut body = BytesMut::new();
    let mut payload = request.take_payload();
    while let Some(chunk) =
        std::future::poll_fn(|cx| std::pin::Pin::new(&mut payload).poll_next(cx)).await
    {
        match chunk {
            Ok(bytes) => body.extend_from_slice(&bytes),
            Err(_) => return Ok(Response::BadRequest().finish()),
        }
    }
    let headers = render_headers(&request);
    let query = request.uri().query().unwrap_or("");
    let (tx, rx) = async_channel::bounded::<NativeReply>(1);
    let slot = Arc::new(NativeResponder {
        sender: Mutex::new(Some(tx)),
    });
    let responder = Arc::into_raw(slot.clone()) as usize as u64;
    let c_request = GearioHttpNativeRequest {
        method: GearioHttpSlice::borrow(request.method().as_str().as_bytes()),
        path: GearioHttpSlice::borrow(request.path().as_bytes()),
        query: GearioHttpSlice::borrow(query.as_bytes()),
        headers: GearioHttpSlice::borrow(&headers),
        body: GearioHttpSlice::borrow(&body),
        responder,
    };
    callback(user_data, &c_request);
    let reply = match rx.recv().await {
        Ok(value) => value,
        Err(_) => return Ok(Response::InternalServerError().finish()),
    };
    // Release the FFI-owned reference after the request reaches a terminal
    // response. The callback-side temporary reference is restored in respond.
    unsafe { drop(Arc::from_raw(responder as usize as *const NativeResponder)) };
    Ok(build_native_response(reply))
}

fn render_headers(request: &Request) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, value) in request.headers() {
        if !out.is_empty() {
            out.push(b'\n');
        }
        out.extend_from_slice(name.as_str().as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(value.as_bytes());
    }
    out
}

fn build_native_response(reply: NativeReply) -> Response {
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut builder = Response::build(status);
    for line in reply.headers.split(|byte| *byte == b'\n') {
        if let Some(index) = line.iter().position(|byte| *byte == b':') {
            let name = String::from_utf8_lossy(&line[..index]);
            let value = String::from_utf8_lossy(line[index + 1..].trim_ascii_start());
            builder.header(name.as_ref(), value.as_ref());
        }
    }
    builder.body(reply.body)
}

/// Complete a native H1 request. This call is safe from any host thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geario_http_native_h1_respond(
    responder: u64,
    status: u16,
    headers: *const u8,
    headers_len: usize,
    body: *const u8,
    body_len: usize,
) -> i32 {
    let header_bytes = if headers.is_null() || headers_len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(headers, headers_len) }.to_vec()
    };
    let body_bytes = if body.is_null() || body_len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(body, body_len) }.to_vec()
    };
    if responder == 0 {
        return crate::abi::GEARIO_HTTP_STATUS_INVALID_ARG;
    }
    let slot = unsafe { Arc::from_raw(responder as usize as *const NativeResponder) };
    let sender = slot.sender.lock().unwrap().take();
    let result = sender.and_then(|sender| {
        sender
            .send_blocking(NativeReply {
                status,
                headers: header_bytes,
                body: body_bytes,
            })
            .ok()
    });
    // Keep the FFI-owned reference alive until native_dispatch releases it.
    let _ = Arc::into_raw(slot);
    match result {
        Some(()) => crate::abi::GEARIO_HTTP_STATUS_OK,
        None => crate::abi::GEARIO_HTTP_STATUS_INVALID_ARG,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn geario_http_native_h1_stop(server: *mut GearioHttpNativeServer) {
    if server.is_null() {
        return;
    }
    let mut server = unsafe { Box::from_raw(server) };
    if let Some(stop) = server.stop.take() {
        let _ = stop.send(());
    }
    if let Some(thread) = server.thread.take() {
        let _ = thread.join();
    }
}

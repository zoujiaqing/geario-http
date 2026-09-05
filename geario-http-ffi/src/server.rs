//! Server side of the C ABI.

use std::ffi::{CStr, c_char, c_void};
use std::net::SocketAddr;
use std::sync::mpsc;

use geario::service::cfg::SharedCfg;
use geario_http::{HttpService, Request, Response, StatusCode};
use geario_http::header::{HeaderName, HeaderValue};

use crate::abi::*;
use crate::responder::{self, Reply};
use crate::slice::GearioHttpSlice;

/// A request handed to the host.
///
/// Every slice borrows memory geario owns and is only valid until the
/// callback returns. `responder` outlives the callback: answer it later from
/// the same worker if the handler needs to.
#[repr(C)]
pub struct GearioHttpRequest {
    pub method: GearioHttpSlice,
    pub path: GearioHttpSlice,
    pub query: GearioHttpSlice,
    /// Headers as `name: value` lines separated by `\n`.
    pub headers: GearioHttpSlice,
    pub body: GearioHttpSlice,
    pub responder: u64,
}

/// Called once per request, on the worker that owns the connection.
pub type GearioHttpRequestCallback =
    extern "C" fn(user_data: *mut c_void, req: *const GearioHttpRequest);

/// Opaque server handle.
pub struct GearioHttpServer {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// The host's callback plus its context, carried into the worker threads.
#[derive(Copy, Clone)]
struct Handler {
    cb: GearioHttpRequestCallback,
    user_data: *mut c_void,
}

// The host promises `user_data` stays alive and is safe to touch from the
// worker threads; Kotlin/Native does this with a StableRef. Nothing here can
// check that, so it is a documented part of the contract.
unsafe impl Send for Handler {}
unsafe impl Sync for Handler {}

fn parse_headers(ptr: *const u8, len: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
    if ptr.is_null() || len == 0 {
        return Vec::new();
    }
    let raw = unsafe { std::slice::from_raw_parts(ptr, len) };
    raw.split(|b| *b == b'\n')
        .filter_map(|line| {
            let i = line.iter().position(|b| *b == b':')?;
            let name = &line[..i];
            let value = line[i + 1..]
                .iter()
                .position(|b| *b != b' ')
                .map_or(&line[..0], |s| &line[i + 1 + s..]);
            if name.is_empty() {
                None
            } else {
                Some((name.to_vec(), value.to_vec()))
            }
        })
        .collect()
}

fn render_headers(req: &Request) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, value) in req.headers().iter() {
        if !out.is_empty() {
            out.push(b'\n');
        }
        out.extend_from_slice(name.as_str().as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(value.as_bytes());
    }
    out
}

/// Start an HTTP/1.1 server.
///
/// Returns NULL on a bad address or if the worker thread cannot start. The
/// server runs until `geario_http_server_stop`.
///
/// `on_request` is called on a worker thread, and different connections land
/// on different workers, so it must be safe to call concurrently. The
/// `responder` in the request may only be answered from the worker that
/// delivered it.
///
/// # Safety
///
/// `host` must be NUL-terminated or NULL. `user_data` must stay valid until
/// the server is stopped.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geario_http_server_start(
    host: *const c_char,
    port: u16,
    on_request: GearioHttpRequestCallback,
    user_data: *mut c_void,
) -> *mut GearioHttpServer {
    let host = if host.is_null() {
        "0.0.0.0".to_owned()
    } else {
        match unsafe { CStr::from_ptr(host) }.to_str() {
            Ok(h) => h.to_owned(),
            Err(_) => return std::ptr::null_mut(),
        }
    };

    let addr: SocketAddr = match format!("{host}:{port}").parse() {
        Ok(a) => a,
        Err(_) => return std::ptr::null_mut(),
    };

    let handler = Handler {
        cb: on_request,
        user_data,
    };
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (ready_tx, ready_rx) = mpsc::channel::<bool>();

    let thread = std::thread::Builder::new()
        .name("geario-http-ffi".into())
        .spawn(move || {
            geario::rt::System::build()
                .name("geario-http-ffi")
                .build(geario::rt::DefaultRuntime)
                .block_on(async move {
                    // An embedded library has no business taking the host's
                    // signals. Without this, geario installs its own SIGINT
                    // handler and the host's never runs.
                    let srv = match geario::server::net::build()
                        .disable_signals()
                        .bind(
                        "ffi",
                        addr,
                        SharedCfg::new("FFI"),
                        async move |_| {
                            HttpService::new(async move |req: Request| {
                                Ok::<_, std::io::Error>(dispatch(handler, req).await)
                            })
                            .build()
                        },
                    ) {
                        Ok(b) => b,
                        Err(_) => {
                            let _ = ready_tx.send(false);
                            return;
                        }
                    }
                    .run();

                    let _ = ready_tx.send(true);
                    // Park until the host asks to stop. The recv is blocking,
                    // so it goes on the blocking pool rather than the reactor.
                    let _ = geario::rt::spawn_blocking(move || stop_rx.recv()).await;
                    srv.stop(false).await;
                });
        });

    let thread = match thread {
        Ok(t) => t,
        Err(_) => return std::ptr::null_mut(),
    };

    match ready_rx.recv() {
        Ok(true) => {}
        _ => return std::ptr::null_mut(),
    }

    Box::into_raw(Box::new(GearioHttpServer {
        stop: Some(stop_tx),
        thread: Some(thread),
    }))
}

async fn dispatch(handler: Handler, req: Request) -> Response {
    let (responder, rx) = responder::register();

    let path = req.path().to_owned();
    let query = req.uri().query().unwrap_or("").to_owned();
    let headers = render_headers(&req);

    let c_req = GearioHttpRequest {
        method: GearioHttpSlice::borrow(req.method().as_str().as_bytes()),
        path: GearioHttpSlice::borrow(path.as_bytes()),
        query: GearioHttpSlice::borrow(query.as_bytes()),
        headers: GearioHttpSlice::borrow(&headers),
        body: GearioHttpSlice::empty(),
        responder,
    };

    (handler.cb)(handler.user_data, &c_req);

    match rx.await {
        Ok(reply) => build_response(reply),
        Err(_) => {
            responder::forget(responder);
            Response::InternalServerError().body("handler dropped the responder")
        }
    }
}

fn build_response(reply: Reply) -> Response {
    let status =
        StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut builder = Response::build(status);
    for (name, value) in &reply.headers {
        if let (Ok(n), Ok(v)) = (
            HeaderName::from_bytes(name),
            HeaderValue::from_bytes(value),
        ) {
            builder.header(n, v);
        }
    }
    builder.body(reply.body)
}

/// Answer a request.
///
/// Must be called on the worker that delivered the responder; anything else
/// returns `GEARIO_HTTP_STATUS_WRONG_THREAD` rather than corrupting state.
///
/// `headers` is `name: value` lines separated by `\n`, or NULL.
///
/// # Safety
///
/// The pointers must either be NULL or point at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geario_http_respond(
    responder: u64,
    status: u16,
    headers_ptr: *const u8,
    headers_len: usize,
    body_ptr: *const u8,
    body_len: usize,
) -> GearioHttpStatus {
    let headers = parse_headers(headers_ptr, headers_len);
    let body = if body_ptr.is_null() || body_len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(body_ptr, body_len) }.to_vec()
    };

    match responder::deliver(
        responder,
        Reply {
            status,
            headers,
            body,
        },
    ) {
        responder::Delivery::Sent => GEARIO_HTTP_STATUS_OK,
        responder::Delivery::WrongThread => GEARIO_HTTP_STATUS_WRONG_THREAD,
        responder::Delivery::Unknown => GEARIO_HTTP_STATUS_INVALID_ARG,
    }
}

/// Stop a server and free its handle. Passing NULL is a no-op.
///
/// # Safety
///
/// `server` must come from `geario_http_server_start` and must not be used
/// afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geario_http_server_stop(server: *mut GearioHttpServer) {
    if server.is_null() {
        return;
    }
    let mut server = unsafe { Box::from_raw(server) };
    if let Some(stop) = server.stop.take() {
        let _ = stop.send(());
    }
    if let Some(t) = server.thread.take() {
        let _ = t.join();
    }
}

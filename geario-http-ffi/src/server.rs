//! Server side of the C ABI.

use std::ffi::{CStr, c_char, c_void};
use std::net::SocketAddr;
use std::sync::mpsc;

use geario::bytes::Bytes;
use geario::service::cfg::SharedCfg;
use geario_http::{HttpService, Request, Response, StatusCode};
use geario_http::body::BodyStream;
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

/// Parse `name: value` lines straight into header types.
///
/// Going through owned byte pairs first would allocate twice per header and
/// then throw both away on conversion.
fn parse_headers(ptr: *const u8, len: usize) -> Vec<(HeaderName, HeaderValue)> {
    if ptr.is_null() || len == 0 {
        return Vec::new();
    }
    let raw = unsafe { std::slice::from_raw_parts(ptr, len) };
    raw.split(|b| *b == b'\n')
        .filter_map(|line| {
            let i = line.iter().position(|b| *b == b':')?;
            let name = &line[..i];
            if name.is_empty() {
                return None;
            }
            let value = line[i + 1..]
                .iter()
                .position(|b| *b != b' ')
                .map_or(&line[..0], |s| &line[i + 1 + s..]);
            // A header the host spelled wrong is dropped rather than failing
            // the whole response: the rest of it is still correct.
            Some((
                HeaderName::from_bytes(name).ok()?,
                HeaderValue::from_bytes(value).ok()?,
            ))
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

/// The largest request body handed to the host.
///
/// The body is delivered as one slice, so it has to be held in memory, and
/// the host cannot ask for a limit: `server_start`'s signature is fixed by
/// the ABI. A request over the limit is answered with 413 and the handler is
/// never called, rather than being delivered truncated.
const MAX_BODY: usize = 16 * 1024 * 1024;

/// Read the whole request body.
///
/// `Err` means the body was too large or the connection failed; either way
/// the handler must not see a partial body and believe it complete.
async fn read_body(req: &mut Request) -> Result<Vec<u8>, Response> {
    let mut payload = req.take_payload();
    let mut body = Vec::new();
    while let Some(chunk) = payload.recv().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) => return Err(Response::BadRequest().body("could not read the request body")),
        };
        if body.len() + chunk.len() > MAX_BODY {
            return Err(Response::PayloadTooLarge().finish());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn dispatch(handler: Handler, mut req: Request) -> Response {
    let body = match read_body(&mut req).await {
        Ok(body) => body,
        Err(response) => return response,
    };

    let (responder, rx) = responder::register();

    // req outlives the callback, so its str slices can be borrowed straight
    // across rather than copied first.
    let headers = render_headers(&req);
    let query = req.uri().query().unwrap_or("");

    let c_req = GearioHttpRequest {
        method: GearioHttpSlice::borrow(req.method().as_str().as_bytes()),
        path: GearioHttpSlice::borrow(req.path().as_bytes()),
        query: GearioHttpSlice::borrow(query.as_bytes()),
        headers: GearioHttpSlice::borrow(&headers),
        body: GearioHttpSlice::borrow(&body),
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

/// Wraps a chunk stream so the body sees `Result<Bytes, _>`.
///
/// A dedicated type rather than `StreamExt::map`, which geario does not carry
/// and which would mean pulling futures-util into the FFI crate for one call.
struct OkStream(geario::util::channel::mpsc::Receiver<Bytes>);

impl futures_core::Stream for OkStream {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::pin::Pin::new(&mut self.0).poll_next(cx).map(|o| o.map(Ok))
    }
}

fn apply_head(
    status: u16,
    headers: Vec<(HeaderName, HeaderValue)>,
) -> geario_http::ResponseBuilder {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut builder = Response::build(status);
    for (name, value) in headers {
        builder.header(name, value);
    }
    builder
}

fn build_response(reply: Reply) -> Response {
    match reply {
        Reply::Once {
            status,
            headers,
            body,
        } => apply_head(status, headers).body(body),
        Reply::Stream {
            status,
            headers,
            chunks,
        } => {
            apply_head(status, headers).body(BodyStream::new(OkStream(chunks)))
        }
    }
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
    // One-shot and streaming are mutually exclusive on a responder: the head
    // is already on the wire, so there is nothing left to decide.
    if matches!(responder::phase(responder), responder::Phase::Streaming) {
        return GEARIO_HTTP_STATUS_WRONG_STATE;
    }

    let headers = parse_headers(headers_ptr, headers_len);
    let body = if body_ptr.is_null() || body_len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(body_ptr, body_len) }.to_vec()
    };

    match responder::deliver(
        responder,
        Reply::Once {
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

/// Send status and headers now and stream the body afterwards.
///
/// After this returns OK, call `geario_http_response_write` for each chunk
/// and `geario_http_response_finish` when done. Mixing this with
/// `geario_http_respond` on the same responder is rejected.
///
/// # Safety
///
/// `headers` must be NULL or point at `headers_len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geario_http_response_begin(
    responder: u64,
    status: u16,
    headers_ptr: *const u8,
    headers_len: usize,
) -> GearioHttpStatus {
    if matches!(responder::phase(responder), responder::Phase::Streaming) {
        return GEARIO_HTTP_STATUS_WRONG_STATE;
    }

    let headers = parse_headers(headers_ptr, headers_len);
    match responder::begin_stream(responder, status, headers) {
        responder::Delivery::Sent => GEARIO_HTTP_STATUS_OK,
        responder::Delivery::WrongThread => GEARIO_HTTP_STATUS_WRONG_THREAD,
        responder::Delivery::Unknown => GEARIO_HTTP_STATUS_INVALID_ARG,
    }
}

/// Append one chunk to a streaming body.
///
/// # Safety
///
/// `chunk` must be NULL or point at `chunk_len` readable bytes. The bytes are
/// copied before this returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geario_http_response_write(
    responder: u64,
    chunk_ptr: *const u8,
    chunk_len: usize,
) -> GearioHttpStatus {
    let chunk = if chunk_ptr.is_null() || chunk_len == 0 {
        Bytes::new()
    } else {
        Bytes::copy_from_slice(unsafe { std::slice::from_raw_parts(chunk_ptr, chunk_len) })
    };
    match responder::write_chunk(responder, chunk) {
        responder::Delivery::Sent => GEARIO_HTTP_STATUS_OK,
        responder::Delivery::WrongThread => GEARIO_HTTP_STATUS_WRONG_THREAD,
        responder::Delivery::Unknown => GEARIO_HTTP_STATUS_INVALID_ARG,
    }
}

/// Close a streaming body.
#[unsafe(no_mangle)]
pub extern "C" fn geario_http_response_finish(responder: u64) -> GearioHttpStatus {
    match responder::finish_stream(responder) {
        responder::Delivery::Sent => GEARIO_HTTP_STATUS_OK,
        responder::Delivery::WrongThread => GEARIO_HTTP_STATUS_WRONG_THREAD,
        responder::Delivery::Unknown => GEARIO_HTTP_STATUS_INVALID_ARG,
    }
}

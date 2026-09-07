//! Client side of the C ABI.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;

use geario_http::client::proxy::ProxyTarget;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;

mod connect;
mod pool;

use self::connect::{Fail, Origin, Sender, Tls};
use self::pool::Pool;
use crate::abi::*;
use crate::slice::{GearioHttpError, GearioHttpHeader, GearioHttpSlice};

/// Requests in flight at once when the caller does not say.
const DEFAULT_MAX_INFLIGHT: u32 = 256;

/// Fields through `flags` must be present. A caller who supplies less has told
/// us nothing at all, and "everything defaults" would turn their mistake into
/// a silent configuration.
pub const GEARIO_HTTP_CLIENT_OPTIONS_MIN_SIZE: u32 = 16;
/// Fields through `url`.
pub const GEARIO_HTTP_CLIENT_REQUEST_MIN_SIZE: u32 =
    8 + 2 * std::mem::size_of::<GearioHttpSlice>() as u32;

#[repr(C)]
pub struct GearioHttpClientOptions {
    pub abi_version: u32,
    pub struct_size: u32,
    pub flags: u64,
    /// 0 disables the connect timeout. Not "use the default", not "expire
    /// immediately" — both readings exist in the wild, so this one is pinned.
    pub connect_timeout_ms: u64,
    /// 0 disables the overall timeout, which streaming responses need.
    pub request_timeout_ms: u64,
    /// Ceiling on requests in flight at once. 0 uses the built-in default.
    pub max_inflight_requests: u32,
    /// *Additional* attempts: 0 means try once, 2 means at most three tries.
    ///
    /// Only idempotent methods are retried, and only when the failure happened
    /// before a response started. Retrying a POST that may already have been
    /// applied is a correctness bug, not a resilience feature.
    pub max_retries: u32,
    /// NULL or zero length means direct connections. Otherwise
    /// `http://host[:port]`.
    ///
    /// Plaintext targets go through it in absolute-form. TLS targets are
    /// refused: tunnelling them needs CONNECT, which this build does not have,
    /// and going direct instead would quietly defeat the proxy.
    pub proxy_url: *const u8,
    pub proxy_url_len: usize,
}

#[repr(C)]
pub struct GearioHttpClientRequest {
    pub abi_version: u32,
    pub struct_size: u32,
    pub method: GearioHttpSlice,
    pub url: GearioHttpSlice,
    pub headers: *const GearioHttpHeader,
    pub header_count: usize,
    pub body_ptr: *const u8,
    pub body_len: usize,
}

/// Called once with the response head. Return CONTINUE or CANCEL.
pub type GearioHttpOnHeaders = extern "C" fn(
    user_data: *mut c_void,
    request_id: u64,
    status: u16,
    http_version: u8,
    headers: *const GearioHttpHeader,
    header_count: usize,
) -> GearioHttpHeadersAction;

/// Called per body chunk. Return CONTINUE, PAUSE or CANCEL.
pub type GearioHttpOnChunk = extern "C" fn(
    user_data: *mut c_void,
    request_id: u64,
    chunk: *const u8,
    chunk_len: usize,
) -> GearioHttpChunkAction;

/// Called once at the end. `error` is NULL when the request succeeded.
pub type GearioHttpOnDone =
    extern "C" fn(user_data: *mut c_void, request_id: u64, error: *const GearioHttpError);

/// Opaque client handle.
pub struct GearioHttpClient {
    tx: async_channel::Sender<Command>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Read from the host thread while the worker writes them, so these are
    /// atomics rather than the Cell the rest of the crate can get away with.
    inflight: Arc<AtomicU32>,
    paused: Arc<AtomicU32>,
    max_inflight: u32,
    max_retries: u32,
    next_id: Cell<u64>,
    closed: Cell<bool>,
}

struct Job {
    id: u64,
    max_retries: u32,
    method: Vec<u8>,
    url: Vec<u8>,
    headers: Vec<(Vec<u8>, Vec<u8>)>,
    body: Vec<u8>,
    cbs: Callbacks,
}

enum Command {
    Send(Box<Job>),
    Cancel(u64),
    Resume(u64),
    Shutdown,
}

#[derive(Copy, Clone)]
struct Callbacks {
    on_headers: Option<GearioHttpOnHeaders>,
    on_chunk: Option<GearioHttpOnChunk>,
    on_done: Option<GearioHttpOnDone>,
    user_data: *mut c_void,
}

// The host promises `user_data` stays alive until this request's on_done
// returns. Nothing here can check that; it is part of the contract.
unsafe impl Send for Callbacks {}

/// Write at most `min(struct_size, size_of::<T>())` bytes.
///
/// A caller built against an older header allocated a shorter struct. Writing
/// the whole of ours would run past the end of their allocation.
unsafe fn write_prefix<T>(dst: *mut T, value: T, struct_size: u32) {
    let n = std::cmp::min(struct_size as usize, std::mem::size_of::<T>());
    unsafe {
        std::ptr::copy_nonoverlapping(std::ptr::addr_of!(value).cast::<u8>(), dst.cast::<u8>(), n);
    }
    std::mem::forget(value);
}

fn validate_header(abi: u32, size: u32, min_size: u32) -> GearioHttpStatus {
    if abi != GEARIO_HTTP_ABI_VERSION {
        return GEARIO_HTTP_STATUS_ABI_MISMATCH;
    }
    if size < min_size {
        return GEARIO_HTTP_STATUS_STRUCT_SIZE;
    }
    GEARIO_HTTP_STATUS_OK
}

/// Fill an options struct with defaults.
///
/// # Safety
///
/// `opts` must point at `struct_size` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geario_http_client_options_init(
    opts: *mut GearioHttpClientOptions,
    struct_size: u32,
) -> GearioHttpStatus {
    if opts.is_null() {
        return GEARIO_HTTP_STATUS_INVALID_ARG;
    }
    if struct_size < GEARIO_HTTP_CLIENT_OPTIONS_MIN_SIZE {
        return GEARIO_HTTP_STATUS_STRUCT_SIZE;
    }
    let defaults = GearioHttpClientOptions {
        abi_version: GEARIO_HTTP_ABI_VERSION,
        struct_size,
        flags: 0,
        connect_timeout_ms: 10_000,
        request_timeout_ms: 60_000,
        max_inflight_requests: DEFAULT_MAX_INFLIGHT,
        max_retries: 2,
        proxy_url: std::ptr::null(),
        proxy_url_len: 0,
    };
    unsafe { write_prefix(opts, defaults, struct_size) };
    GEARIO_HTTP_STATUS_OK
}

/// Fill a request struct with defaults.
///
/// # Safety
///
/// `request` must point at `struct_size` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geario_http_client_request_init(
    request: *mut GearioHttpClientRequest,
    struct_size: u32,
) -> GearioHttpStatus {
    if request.is_null() {
        return GEARIO_HTTP_STATUS_INVALID_ARG;
    }
    if struct_size < GEARIO_HTTP_CLIENT_REQUEST_MIN_SIZE {
        return GEARIO_HTTP_STATUS_STRUCT_SIZE;
    }
    let defaults = GearioHttpClientRequest {
        abi_version: GEARIO_HTTP_ABI_VERSION,
        struct_size,
        method: GearioHttpSlice::empty(),
        url: GearioHttpSlice::empty(),
        headers: std::ptr::null(),
        header_count: 0,
        body_ptr: std::ptr::null(),
        body_len: 0,
    };
    unsafe { write_prefix(request, defaults, struct_size) };
    GEARIO_HTTP_STATUS_OK
}

/// Shared counters the host can read while requests run.
#[derive(Clone)]
struct Counters {
    inflight: Arc<AtomicU32>,
    paused: Arc<AtomicU32>,
}

thread_local! {
    /// Requests the host asked to pause, and the waker that resumes them.
    static PAUSED: RefCell<HashMap<u64, geario::util::channel::oneshot::Sender<()>>> =
        RefCell::new(HashMap::new());
    static CANCELLED: RefCell<HashMap<u64, ()>> = RefCell::new(HashMap::new());
}

/// Create a client.
///
/// # Safety
///
/// `opts` must point at a struct whose `struct_size` field describes it, and
/// `out_client` at a writable pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geario_http_client_new(
    opts: *const GearioHttpClientOptions,
    out_client: *mut *mut GearioHttpClient,
) -> GearioHttpStatus {
    if opts.is_null() || out_client.is_null() {
        return GEARIO_HTTP_STATUS_INVALID_ARG;
    }
    // Read only the prefix the caller allocated. Dereferencing their shorter
    // buffer as a whole struct would read past the end of their allocation,
    // which is exactly what the init functions are careful to avoid writing.
    let raw_abi = unsafe { std::ptr::read_unaligned(opts.cast::<u32>()) };
    let raw_size = unsafe {
        std::ptr::read_unaligned(
            opts.cast::<u8>()
                .add(std::mem::size_of::<u32>())
                .cast::<u32>(),
        )
    };
    let st = validate_header(raw_abi, raw_size, GEARIO_HTTP_CLIENT_OPTIONS_MIN_SIZE);
    if st != GEARIO_HTTP_STATUS_OK {
        return st;
    }

    let full = raw_size as usize >= std::mem::size_of::<GearioHttpClientOptions>();
    let (connect_ms, request_ms, max_inflight, max_retries, flags, proxy) = if full {
        let o = unsafe { &*opts };
        let proxy = if o.proxy_url.is_null() || o.proxy_url_len == 0 {
            None
        } else {
            let raw = unsafe { std::slice::from_raw_parts(o.proxy_url, o.proxy_url_len) };
            match std::str::from_utf8(raw).ok().map(ProxyTarget::parse) {
                Some(Ok(p)) => Some(p),
                // A proxy that cannot be honoured is refused rather than
                // ignored: connecting direct instead would be the one outcome
                // the caller did not ask for.
                _ => return GEARIO_HTTP_STATUS_INVALID_ARG,
            }
        };
        (
            o.connect_timeout_ms,
            o.request_timeout_ms,
            o.max_inflight_requests,
            o.max_retries,
            o.flags,
            proxy,
        )
    } else {
        let flags = unsafe { std::ptr::read_unaligned(opts.cast::<u8>().add(8).cast::<u64>()) };
        (10_000, 60_000, DEFAULT_MAX_INFLIGHT, 2, flags, None)
    };

    // A flag this build does not know may be the one carrying a security
    // decision, so it is never dropped quietly.
    let known = GEARIO_HTTP_CLIENT_HTTP2_REQUIRED | GEARIO_HTTP_CLIENT_CA_REPLACE_SYSTEM;
    if flags & !known != 0 {
        return GEARIO_HTTP_STATUS_UNKNOWN_FLAGS;
    }
    let require_h2 = flags & GEARIO_HTTP_CLIENT_HTTP2_REQUIRED != 0;

    let max_inflight = if max_inflight == 0 {
        DEFAULT_MAX_INFLIGHT
    } else {
        max_inflight
    };

    let (tx, rx) = async_channel::unbounded::<Command>();
    let (ready_tx, ready_rx) = mpsc::channel::<bool>();
    let inflight = Arc::new(AtomicU32::new(0));
    let paused = Arc::new(AtomicU32::new(0));
    let counters = Counters {
        inflight: inflight.clone(),
        paused: paused.clone(),
    };

    let thread = std::thread::Builder::new()
        .name("geario-http-ffi-client".into())
        .spawn(move || {
            geario::rt::System::build()
                .name("geario-http-ffi-client")
                .build(geario::rt::DefaultRuntime)
                .block_on(async move {
                    // rustls needs a process-wide crypto provider; installing
                    // it here is idempotent and harmless if the host already
                    // did. Without it every https request would fail.
                    let _ = tls_rustls::crypto::aws_lc_rs::default_provider().install_default();
                    let tls = match Tls::new(None, false, require_h2) {
                        Ok(t) => Some(t),
                        // No usable TLS: https will report UNSUPPORTED, http
                        // still works. Not a reason to refuse to start.
                        Err(_) => None,
                    };
                    let connect_timeout =
                        geario::util::time::Millis(u32::try_from(connect_ms).unwrap_or(u32::MAX));
                    let request_timeout =
                        geario::util::time::Millis(u32::try_from(request_ms).unwrap_or(u32::MAX));
                    let pool = Pool::new(tls, proxy, connect_timeout);
                    let _ = ready_tx.send(true);

                    while let Ok(cmd) = rx.recv().await {
                        match cmd {
                            Command::Shutdown => break,
                            Command::Cancel(id) => {
                                CANCELLED.with(|c| c.borrow_mut().insert(id, ()));
                                if let Some(tx) = PAUSED.with(|p| p.borrow_mut().remove(&id)) {
                                    let _ = tx.send(());
                                }
                            }
                            Command::Resume(id) => {
                                if let Some(tx) = PAUSED.with(|p| p.borrow_mut().remove(&id)) {
                                    let _ = tx.send(());
                                }
                            }
                            Command::Send(job) => {
                                let pool = pool.clone();
                                let counters = counters.clone();
                                geario::rt::spawn(run_job(pool, *job, counters, request_timeout));
                            }
                        }
                    }
                });
        });

    let thread = match thread {
        Ok(t) => t,
        Err(_) => return GEARIO_HTTP_STATUS_OOM,
    };
    if ready_rx.recv() != Ok(true) {
        return GEARIO_HTTP_STATUS_OOM;
    }

    let client = Box::new(GearioHttpClient {
        tx,
        thread: Some(thread),
        inflight,
        paused,
        max_inflight,
        max_retries,
        next_id: Cell::new(1),
        closed: Cell::new(false),
    });
    unsafe { *out_client = Box::into_raw(client) };
    GEARIO_HTTP_STATUS_OK
}

fn report(cbs: &Callbacks, id: u64, kind: GearioHttpErrorKind, msg: &str) {
    if let Some(done) = cbs.on_done {
        if kind == GEARIO_HTTP_ERR_NONE {
            done(cbs.user_data, id, std::ptr::null());
        } else {
            let err = GearioHttpError {
                kind,
                protocol_code: 0,
                message: GearioHttpSlice::borrow(msg.as_bytes()),
            };
            done(cbs.user_data, id, &err);
        }
    }
}

async fn run_job(
    pool: Pool,
    job: Job,
    counters: Counters,
    request_timeout: geario::util::time::Millis,
) {
    let Job {
        id,
        max_retries,
        method,
        url,
        headers,
        body,
        cbs,
    } = job;

    // The slot was claimed in geario_http_client_send; this only releases it.
    let _guard = InflightGuard(counters.inflight.clone());

    let url = match std::str::from_utf8(&url) {
        Ok(u) => u,
        Err(_) => return report(&cbs, id, GEARIO_HTTP_ERR_INVALID_URL, "url is not utf-8"),
    };
    let (origin, uri) = match Origin::parse(url) {
        Ok(v) => v,
        Err(f) => return report(&cbs, id, f.kind, &f.message),
    };
    let method = match hyper::Method::from_bytes(&method) {
        Ok(m) => m,
        Err(_) => return report(&cbs, id, GEARIO_HTTP_ERR_INVALID_URL, "bad method"),
    };

    // Only idempotent methods are retried, and only when the failure happened
    // before the request could have reached the server. A POST that failed
    // after being sent may already have been applied.
    let idempotent = is_idempotent(method.as_str().as_bytes());
    let attempts = if idempotent {
        max_retries.saturating_add(1)
    } else {
        1
    };

    let mut last: Option<Fail> = None;
    'attempts: for _ in 0..attempts {
        let mut lease = match pool.checkout(&origin).await {
            Ok(l) => l,
            Err(f) => {
                // Nothing was sent, so a fresh attempt is safe.
                last = Some(f);
                continue;
            }
        };

        // Request-line form: HTTP/2 needs the whole URI (it fills :authority
        // and :path from it); a plaintext request through a proxy carries the
        // whole URI so the proxy learns where it is going; a direct HTTP/1
        // request sends only the path, which is what an origin server wants.
        let target = if matches!(lease.sender, Sender::H2(_)) || (!origin.tls && pool.is_proxied())
        {
            uri.clone()
        } else {
            uri.path_and_query()
                .map(|pq| pq.as_str())
                .unwrap_or("/")
                .parse()
                .unwrap_or_else(|_| uri.clone())
        };
        let mut builder = hyper::Request::builder().method(method.clone()).uri(target);
        for (name, value) in &headers {
            builder = builder.header(&name[..], &value[..]);
        }
        // hyper needs a Host header for an origin-form HTTP/1 request.
        if !builder
            .headers_ref()
            .is_some_and(|h| h.contains_key(hyper::header::HOST))
            && !matches!(lease.sender, Sender::H2(_))
        {
            if let Some(auth) = uri.authority() {
                builder = builder.header(hyper::header::HOST, auth.as_str());
            }
        }
        let req = match builder.body(Full::new(Bytes::from(body.clone()))) {
            Ok(r) => r,
            Err(e) => return report(&cbs, id, GEARIO_HTTP_ERR_INVALID_URL, &format!("{e}")),
        };

        let sent = match &mut lease.sender {
            Sender::H1(s) => with_timeout(request_timeout, s.send_request(req)).await,
            Sender::H2(s) => with_timeout(request_timeout, s.send_request(req)).await,
            Sender::Gone => unreachable!(),
        };
        let resp = match sent {
            Ok(Ok(r)) => r,
            Ok(Err(_e)) => {
                // The connection failed before a response. Whether the peer
                // saw the request cannot be known; only an idempotent method
                // is safe to replay, and the retry opens a fresh connection.
                lease.checkin();
                if idempotent {
                    last = Some(Fail::new(
                        GEARIO_HTTP_ERR_CONNECT,
                        "connection failed before response",
                    ));
                    continue 'attempts;
                }
                return report(
                    &cbs,
                    id,
                    GEARIO_HTTP_ERR_OUTCOME_UNKNOWN,
                    "connection failed before response",
                );
            }
            Err(()) => {
                lease.checkin();
                return report(&cbs, id, GEARIO_HTTP_ERR_TIMEOUT, "request timed out");
            }
        };

        deliver(resp, id, &cbs, &counters, lease).await;
        return;
    }

    let f = last.expect("a failed attempt always records its error");
    report(&cbs, id, f.kind, &f.message);
}

/// Run `fut` under a deadline. A zero timeout disables it, which streaming
/// responses need.
async fn with_timeout<T>(
    dur: geario::util::time::Millis,
    fut: impl Future<Output = T>,
) -> Result<T, ()> {
    if dur.0 == 0 {
        Ok(fut.await)
    } else {
        geario::util::time::timeout_checked(dur, fut).await
    }
}

/// Hand the response head and body to the host, then return the connection.
async fn deliver(
    resp: hyper::Response<hyper::body::Incoming>,
    id: u64,
    cbs: &Callbacks,
    counters: &Counters,
    lease: crate::client::pool::Lease,
) {
    let version = match resp.version() {
        hyper::Version::HTTP_2 => 2u8,
        _ => 1u8,
    };

    if let Some(on_headers) = cbs.on_headers {
        // resp owns the header storage and outlives the callback, so the
        // slices point straight at it.
        let view: Vec<GearioHttpHeader> = resp
            .headers()
            .iter()
            .map(|(n, v)| GearioHttpHeader {
                name: GearioHttpSlice::borrow(n.as_str().as_bytes()),
                value: GearioHttpSlice::borrow(v.as_bytes()),
            })
            .collect();
        let action = on_headers(
            cbs.user_data,
            id,
            resp.status().as_u16(),
            version,
            view.as_ptr(),
            view.len(),
        );
        if action == GEARIO_HTTP_HEADERS_CANCEL {
            return report(cbs, id, GEARIO_HTTP_ERR_CANCELLED, "cancelled at headers");
        }
    }

    let mut body = resp.into_body();
    loop {
        let frame = match body.frame().await {
            None => break,
            Some(Ok(f)) => f,
            Some(Err(e)) => {
                // The head was already delivered, so the request was
                // certainly processed; only the response is incomplete.
                return report(cbs, id, GEARIO_HTTP_ERR_TRUNCATED, &format!("{e}"));
            }
        };
        let Ok(chunk) = frame.into_data() else {
            // A trailers frame; nothing to hand to a body callback.
            continue;
        };
        if chunk.is_empty() {
            continue;
        }
        let Some(on_chunk) = cbs.on_chunk else {
            continue;
        };
        match on_chunk(cbs.user_data, id, chunk.as_ptr(), chunk.len()) {
            GEARIO_HTTP_CHUNK_CANCEL => {
                return report(cbs, id, GEARIO_HTTP_ERR_CANCELLED, "cancelled at chunk");
            }
            GEARIO_HTTP_CHUNK_PAUSE => {
                // Not polling the body is real backpressure: over h2 the flow
                // control window stops advancing, over h1 the socket is not
                // read.
                counters.paused.fetch_add(1, Ordering::Relaxed);
                let (tx, rx) = geario::util::channel::oneshot::channel();
                PAUSED.with(|p| p.borrow_mut().insert(id, tx));
                let _ = rx.await;
                counters.paused.fetch_sub(1, Ordering::Relaxed);
                if CANCELLED.with(|c| c.borrow_mut().remove(&id)).is_some() {
                    return report(cbs, id, GEARIO_HTTP_ERR_CANCELLED, "cancelled");
                }
            }
            _ => {}
        }
    }

    // The body is fully read, so an h1 connection can serve another request.
    lease.checkin();
    report(cbs, id, GEARIO_HTTP_ERR_NONE, "");
}

struct InflightGuard(Arc<AtomicU32>);

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Exposed so the retry classification can be tested without a network.
#[doc(hidden)]
pub fn is_idempotent_for_tests(method: &[u8]) -> bool {
    is_idempotent(method)
}

/// Methods that may be repeated without changing the outcome.
fn is_idempotent(method: &[u8]) -> bool {
    matches!(
        method,
        b"GET" | b"HEAD" | b"PUT" | b"DELETE" | b"OPTIONS" | b"TRACE"
    )
}

/// Send a request. Returns immediately; the callbacks report progress.
///
/// # Safety
///
/// `client` must come from `geario_http_client_new`, `request` must describe
/// itself through `struct_size`, and `user_data` must outlive `on_done`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geario_http_client_send(
    client: *mut GearioHttpClient,
    request: *const GearioHttpClientRequest,
    on_headers: Option<GearioHttpOnHeaders>,
    on_chunk: Option<GearioHttpOnChunk>,
    on_done: Option<GearioHttpOnDone>,
    user_data: *mut c_void,
    out_request_id: *mut u64,
) -> GearioHttpStatus {
    if client.is_null() || request.is_null() || out_request_id.is_null() {
        return GEARIO_HTTP_STATUS_INVALID_ARG;
    }
    let c = unsafe { &*client };
    if c.closed.get() {
        return GEARIO_HTTP_STATUS_CLOSED;
    }
    // Claim a slot here, not on the worker. Checking a counter the worker
    // raises later would let every queued send through before the first one
    // starts, which makes the ceiling decorative.
    //
    // Refusing rather than queueing also keeps it meaningful: an unbounded
    // queue just moves the pressure somewhere the host cannot see.
    if c.inflight
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < c.max_inflight).then_some(n + 1)
        })
        .is_err()
    {
        return GEARIO_HTTP_STATUS_THROTTLED;
    }

    let raw_abi = unsafe { std::ptr::read_unaligned(request.cast::<u32>()) };
    let raw_size = unsafe {
        std::ptr::read_unaligned(
            request
                .cast::<u8>()
                .add(std::mem::size_of::<u32>())
                .cast::<u32>(),
        )
    };
    let st = validate_header(raw_abi, raw_size, GEARIO_HTTP_CLIENT_REQUEST_MIN_SIZE);
    if st != GEARIO_HTTP_STATUS_OK {
        return st;
    }

    let r = unsafe { &*request };
    let take = |s: &GearioHttpSlice| -> Vec<u8> {
        if s.ptr.is_null() || s.len == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(s.ptr, s.len) }.to_vec()
        }
    };

    let url = take(&r.url);
    if url.is_empty() {
        return GEARIO_HTTP_STATUS_INVALID_ARG;
    }
    let method = if r.method.ptr.is_null() || r.method.len == 0 {
        b"GET".to_vec()
    } else {
        take(&r.method)
    };

    let mut headers = Vec::new();
    if !r.headers.is_null() {
        for i in 0..r.header_count {
            let h = unsafe { &*r.headers.add(i) };
            headers.push((take(&h.name), take(&h.value)));
        }
    }
    let body = if r.body_ptr.is_null() || r.body_len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(r.body_ptr, r.body_len) }.to_vec()
    };

    let id = c.next_id.get();
    c.next_id.set(id.wrapping_add(1).max(1));

    let job = Job {
        id,
        max_retries: c.max_retries,
        method,
        url,
        headers,
        body,
        cbs: Callbacks {
            on_headers,
            on_chunk,
            on_done,
            user_data,
        },
    };

    match c.tx.try_send(Command::Send(Box::new(job))) {
        Ok(()) => {
            unsafe { *out_request_id = id };
            GEARIO_HTTP_STATUS_OK
        }
        Err(_) => {
            // The job never reached a worker, so nothing will release the slot.
            c.inflight.fetch_sub(1, Ordering::AcqRel);
            GEARIO_HTTP_STATUS_CLOSED
        }
    }
}

/// Resume a request the host paused from `on_chunk`.
///
/// # Safety
///
/// `client` must come from `geario_http_client_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geario_http_client_resume(
    client: *mut GearioHttpClient,
    request_id: u64,
) -> GearioHttpStatus {
    if client.is_null() {
        return GEARIO_HTTP_STATUS_INVALID_ARG;
    }
    let c = unsafe { &*client };
    match c.tx.try_send(Command::Resume(request_id)) {
        Ok(()) => GEARIO_HTTP_STATUS_OK,
        Err(_) => GEARIO_HTTP_STATUS_CLOSED,
    }
}

/// Cancel a request.
///
/// # Safety
///
/// `client` must come from `geario_http_client_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geario_http_client_cancel(
    client: *mut GearioHttpClient,
    request_id: u64,
) -> GearioHttpStatus {
    if client.is_null() {
        return GEARIO_HTTP_STATUS_INVALID_ARG;
    }
    let c = unsafe { &*client };
    match c.tx.try_send(Command::Cancel(request_id)) {
        Ok(()) => GEARIO_HTTP_STATUS_OK,
        Err(_) => GEARIO_HTTP_STATUS_CLOSED,
    }
}

/// How many requests are in flight. Zero for a NULL client.
///
/// # Safety
///
/// `client` must be NULL or come from `geario_http_client_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geario_http_client_inflight_count(client: *mut GearioHttpClient) -> u32 {
    if client.is_null() {
        return 0;
    }
    unsafe { &*client }.inflight.load(Ordering::Relaxed)
}

/// How many requests are paused waiting for `geario_http_client_resume`.
///
/// # Safety
///
/// `client` must be NULL or come from `geario_http_client_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geario_http_client_paused_stream_count(
    client: *mut GearioHttpClient,
) -> u32 {
    if client.is_null() {
        return 0;
    }
    unsafe { &*client }.paused.load(Ordering::Relaxed)
}

/// Stop accepting new requests and wind the worker down.
///
/// # Safety
///
/// `client` must be NULL or come from `geario_http_client_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geario_http_client_close(client: *mut GearioHttpClient) {
    if client.is_null() {
        return;
    }
    let c = unsafe { &*client };
    if c.closed.replace(true) {
        return;
    }
    let _ = c.tx.try_send(Command::Shutdown);
    c.tx.close();
}

/// Close if needed, then free. Passing NULL is a no-op.
///
/// # Safety
///
/// `client` must come from `geario_http_client_new` and must not be used
/// afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn geario_http_client_free(client: *mut GearioHttpClient) {
    if client.is_null() {
        return;
    }
    unsafe { geario_http_client_close(client) };
    let mut c = unsafe { Box::from_raw(client) };
    if let Some(t) = c.thread.take() {
        let _ = t.join();
    }
}

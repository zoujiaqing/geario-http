//! The FFI client against a real server, driven the way a C host would.
//!
//! client_abi.rs covers the option/struct validation; this covers a request
//! actually going out and the callbacks firing in order.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU8, AtomicU16, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use geario_http_ffi::*;

/// What the callbacks recorded for one request.
#[derive(Default)]
struct Record {
    status: AtomicU16,
    version: AtomicU8,
    header_count: AtomicU32,
    error: AtomicU8,
    done: AtomicU8,
}

static REC: OnceLock<Record> = OnceLock::new();
static BODY: Mutex<Vec<u8>> = Mutex::new(Vec::new());

fn rec() -> &'static Record {
    REC.get_or_init(Record::default)
}

extern "C" fn on_headers(
    _u: *mut c_void,
    _id: u64,
    status: u16,
    version: u8,
    _h: *const GearioHttpHeader,
    n: usize,
) -> GearioHttpHeadersAction {
    rec().status.store(status, Ordering::SeqCst);
    rec().version.store(version, Ordering::SeqCst);
    rec().header_count.store(n as u32, Ordering::SeqCst);
    GEARIO_HTTP_HEADERS_CONTINUE
}

extern "C" fn on_chunk(
    _u: *mut c_void,
    _id: u64,
    chunk: *const u8,
    len: usize,
) -> GearioHttpChunkAction {
    let bytes = unsafe { std::slice::from_raw_parts(chunk, len) };
    BODY.lock().unwrap().extend_from_slice(bytes);
    GEARIO_HTTP_CHUNK_CONTINUE
}

extern "C" fn on_done(_u: *mut c_void, _id: u64, error: *const GearioHttpError) {
    if !error.is_null() {
        rec()
            .error
            .store(unsafe { (*error).kind } as u8, Ordering::SeqCst);
    }
    rec().done.store(1, Ordering::SeqCst);
}

fn wait_done() {
    let t = Instant::now();
    while rec().done.load(Ordering::SeqCst) == 0 {
        assert!(t.elapsed() < Duration::from_secs(10), "on_done never fired");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Send one GET and return once on_done has fired.
fn get(url: &[u8]) {
    let mut opts: GearioHttpClientOptions = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe {
            geario_http_client_options_init(
                &mut opts,
                std::mem::size_of::<GearioHttpClientOptions>() as u32,
            )
        },
        GEARIO_HTTP_STATUS_OK
    );
    let mut client = std::ptr::null_mut();
    assert_eq!(
        unsafe { geario_http_client_new(&opts, &mut client) },
        GEARIO_HTTP_STATUS_OK
    );

    let mut req: GearioHttpClientRequest = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe {
            geario_http_client_request_init(
                &mut req,
                std::mem::size_of::<GearioHttpClientRequest>() as u32,
            )
        },
        GEARIO_HTTP_STATUS_OK
    );
    req.url = GearioHttpSlice {
        ptr: url.as_ptr(),
        len: url.len(),
    };

    let mut request_id = 0u64;
    assert_eq!(
        unsafe {
            geario_http_client_send(
                client,
                &req,
                Some(on_headers),
                Some(on_chunk),
                Some(on_done),
                std::ptr::null_mut(),
                &mut request_id,
            )
        },
        GEARIO_HTTP_STATUS_OK
    );
    wait_done();
    unsafe { geario_http_client_free(client) };
}

/// A tiny HTTP/1.1 server on a background thread with its own geario runtime.
fn spawn_server(body: &'static [u8]) -> u16 {
    use geario::service::cfg::SharedCfg;
    use geario::service::fn_service;
    use geario_http::hyper_rt::{GearioTransport as _T, serve_auto};
    use http_body_util::Full;
    use hyper::service::service_fn;
    use hyper::{Response, body::Bytes};
    let _ = _T::<geario::io::Base>::new;

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        geario::rt::System::build()
            .name("test-server")
            .build(geario::rt::DefaultRuntime)
            .block_on(async move {
                let lst = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                let port = lst.local_addr().unwrap().port();
                tx.send(port).unwrap();
                let _srv = geario::server::net::build()
                    .disable_signals()
                    .listen("t", lst, SharedCfg::new("T"), async move |_| {
                        fn_service(async move |io: geario::io::Io| {
                            let _ = serve_auto(
                                io,
                                service_fn(move |_req| async move {
                                    Ok::<_, std::convert::Infallible>(
                                        Response::builder()
                                            .header("content-type", "text/plain")
                                            .body(Full::new(Bytes::from_static(body)))
                                            .unwrap(),
                                    )
                                }),
                            )
                            .await;
                            Ok::<_, std::io::Error>(())
                        })
                    })
                    .unwrap()
                    .run();
                std::future::pending::<()>().await;
            });
    });
    rx.recv().unwrap()
}

#[test]
fn a_get_over_http1_delivers_head_and_body() {
    let port = spawn_server(b"hello from the server");
    BODY.lock().unwrap().clear();
    get(format!("http://127.0.0.1:{port}/x").as_bytes());

    assert_eq!(rec().status.load(Ordering::SeqCst), 200);
    assert_eq!(
        rec().version.load(Ordering::SeqCst),
        1,
        "should be HTTP/1.1"
    );
    assert_eq!(rec().error.load(Ordering::SeqCst), 0, "reported an error");
    assert!(rec().header_count.load(Ordering::SeqCst) >= 1);
    assert_eq!(&BODY.lock().unwrap()[..], b"hello from the server");
}

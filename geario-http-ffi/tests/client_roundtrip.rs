//! The FFI client against a real server, driven the way a C host would.
//!
//! client_abi.rs covers the option/struct validation; this covers a request
//! actually going out and the callbacks firing in order. Each request carries
//! its own state through `user_data`, so tests are independent and run in
//! parallel, which is also how a real host keeps per-request state.

use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use geario_http_ffi::*;

/// What one request's callbacks recorded. `done` gates the waiter; the rest
/// are only read after it is set, so a plain Mutex is enough.
#[derive(Default)]
struct Ctx {
    status: Mutex<u16>,
    version: Mutex<u8>,
    header_count: Mutex<u32>,
    error: Mutex<u8>,
    body: Mutex<Vec<u8>>,
    done: std::sync::atomic::AtomicBool,
}

extern "C" fn on_headers(
    u: *mut c_void,
    _id: u64,
    status: u16,
    version: u8,
    _h: *const GearioHttpHeader,
    n: usize,
) -> GearioHttpHeadersAction {
    let ctx = unsafe { &*(u as *const Ctx) };
    *ctx.status.lock().unwrap() = status;
    *ctx.version.lock().unwrap() = version;
    *ctx.header_count.lock().unwrap() = n as u32;
    GEARIO_HTTP_HEADERS_CONTINUE
}

extern "C" fn on_chunk(
    u: *mut c_void,
    _id: u64,
    chunk: *const u8,
    len: usize,
) -> GearioHttpChunkAction {
    let ctx = unsafe { &*(u as *const Ctx) };
    ctx.body
        .lock()
        .unwrap()
        .extend_from_slice(unsafe { std::slice::from_raw_parts(chunk, len) });
    GEARIO_HTTP_CHUNK_CONTINUE
}

extern "C" fn on_done(u: *mut c_void, _id: u64, error: *const GearioHttpError) {
    let ctx = unsafe { &*(u as *const Ctx) };
    if !error.is_null() {
        *ctx.error.lock().unwrap() = unsafe { (*error).kind } as u8;
    }
    ctx.done.store(true, Ordering::SeqCst);
}

fn wait_done(ctx: &Ctx) {
    let t = Instant::now();
    while !ctx.done.load(Ordering::SeqCst) {
        assert!(t.elapsed() < Duration::from_secs(10), "on_done never fired");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Send one GET, blocking until on_done fires, and return the recorded state.
fn get(url: &[u8], ca: Option<&[u8]>, replace_system: bool) -> Ctx {
    let ctx = Box::new(Ctx::default());

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
    if let Some(ca) = ca {
        opts.custom_ca_pem = ca.as_ptr();
        opts.custom_ca_pem_len = ca.len();
    }
    if replace_system {
        opts.flags |= GEARIO_HTTP_CLIENT_CA_REPLACE_SYSTEM;
    }
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
                std::ptr::addr_of!(*ctx) as *mut c_void,
                &mut request_id,
            )
        },
        GEARIO_HTTP_STATUS_OK
    );
    wait_done(&ctx);
    unsafe { geario_http_client_free(client) };
    *ctx
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
    let ctx = get(format!("http://127.0.0.1:{port}/x").as_bytes(), None, false);

    assert_eq!(*ctx.status.lock().unwrap(), 200);
    assert_eq!(*ctx.version.lock().unwrap(), 1, "should be HTTP/1.1");
    assert_eq!(*ctx.error.lock().unwrap(), 0, "reported an error");
    assert!(*ctx.header_count.lock().unwrap() >= 1);
    assert_eq!(&ctx.body.lock().unwrap()[..], b"hello from the server");
}

/// A self-signed server is reached when its CA is trusted, and not reached
/// when only the built-in roots are.
mod tls {
    use super::*;
    use std::sync::Arc;

    use geario::service::cfg::SharedCfg;
    use geario::tls::rustls::TlsServerFilter;
    use geario_http::hyper_rt::{GearioExecutor, GearioTransport};
    use http_body_util::Full;
    use hyper::service::service_fn;
    use hyper::{Response, body::Bytes};
    use tls_rustls::ServerConfig;
    use tls_rustls::pki_types::{CertificateDer, PrivateKeyDer};

    fn spawn_tls_server() -> (u16, String) {
        let mut ca = rcgen::CertificateParams::new(Vec::new()).unwrap();
        ca.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_key = rcgen::KeyPair::generate().unwrap();
        let ca_cert = ca.self_signed(&ca_key).unwrap();
        let leaf = rcgen::CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
        let leaf_key = rcgen::KeyPair::generate().unwrap();
        let leaf_cert = leaf.signed_by(&leaf_key, &ca_cert, &ca_key).unwrap();
        let ca_pem = ca_cert.pem();
        let cert: CertificateDer<'static> = leaf_cert.der().clone();
        let key: PrivateKeyDer<'static> =
            PrivateKeyDer::try_from(leaf_key.serialize_der()).unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            geario::rt::System::build()
                .name("tls-server")
                .build(geario::rt::DefaultRuntime)
                .block_on(async move {
                    let _ = tls_rustls::crypto::aws_lc_rs::default_provider().install_default();
                    let mut cfg = ServerConfig::builder()
                        .with_no_client_auth()
                        .with_single_cert(vec![cert], key)
                        .unwrap();
                    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
                    let cfg = Arc::new(cfg);
                    let lst = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                    tx.send(lst.local_addr().unwrap().port()).unwrap();
                    loop {
                        let Ok(Ok((s, _))) = geario::rt::spawn_blocking({
                            let lst = lst.try_clone().unwrap();
                            move || lst.accept()
                        })
                        .await
                        else {
                            return;
                        };
                        s.set_nonblocking(true).ok();
                        let Ok(io) = geario::net::from_tcp_stream(s, SharedCfg::new("S").into())
                        else {
                            continue;
                        };
                        let cfg = cfg.clone();
                        geario::rt::spawn(async move {
                            let Ok(io) =
                                TlsServerFilter::create(io, cfg, geario::util::time::Millis(5000))
                                    .await
                            else {
                                return;
                            };
                            let svc = service_fn(|_r| async {
                                Ok::<_, std::convert::Infallible>(
                                    Response::builder()
                                        .body(Full::new(Bytes::from_static(b"secure")))
                                        .unwrap(),
                                )
                            });
                            use geario::io::types::HttpProtocol;
                            if matches!(io.query::<HttpProtocol>().get(), Some(HttpProtocol::Http2))
                            {
                                let _ = hyper::server::conn::http2::Builder::new(GearioExecutor)
                                    .serve_connection(GearioTransport::new(io), svc)
                                    .await;
                            } else {
                                let _ = hyper::server::conn::http1::Builder::new()
                                    .serve_connection(GearioTransport::new(io), svc)
                                    .await;
                            }
                        });
                    }
                });
        });
        (rx.recv().unwrap(), ca_pem)
    }

    #[test]
    fn a_trusted_custom_ca_reaches_an_https_server() {
        let (port, ca) = spawn_tls_server();
        let ctx = get(
            format!("https://localhost:{port}/x").as_bytes(),
            Some(ca.as_bytes()),
            true,
        );
        assert_eq!(
            *ctx.error.lock().unwrap(),
            0,
            "trusted CA should have connected"
        );
        assert_eq!(&ctx.body.lock().unwrap()[..], b"secure");
    }

    #[test]
    fn the_same_server_is_refused_with_only_the_system_roots() {
        let (port, _ca) = spawn_tls_server();
        let ctx = get(
            format!("https://localhost:{port}/x").as_bytes(),
            None,
            false,
        );
        assert_eq!(
            *ctx.error.lock().unwrap(),
            GEARIO_HTTP_ERR_TLS_CA as u8,
            "a self-signed cert must not verify against system roots"
        );
    }
}

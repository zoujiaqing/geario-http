//! hyper's HTTP/1.1 running on geario's IO and workers.
//!
//! The point of this build: hyper owns the protocol, geario owns accept,
//! workers, sockets and buffers. No tokio anywhere.
use std::convert::Infallible;
use std::io;

use bytes::Bytes;
use geario::io::Io;
use geario::service::cfg::SharedCfg;
use geario::service::fn_service;
use geario_http::hyper_rt::{GearioExecutor, GearioTransport};
use http_body_util::Full;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};


/// Response body of the configured size, built once.
///
/// A repeating pattern rather than zeros: a compressible or all-zero body can
/// be handled differently by the stack than realistic bytes.
fn body_bytes() -> &'static [u8] {
    use std::sync::OnceLock;
    static BODY: OnceLock<Vec<u8>> = OnceLock::new();
    BODY.get_or_init(|| {
        let n: usize = std::env::var("BENCH_BODY_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(24);
        let seed = b"hello from the benchmark ";
        seed.iter().copied().cycle().take(n).collect()
    })
    .as_slice()
}

async fn handle(req: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    // Drain the request body, for the same reason the geario server does.
    if req.method() == hyper::Method::POST {
        use http_body_util::BodyExt;
        let _ = req.into_body().collect().await;
    }
    Ok(Response::builder()
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from_static(body_bytes())))
        .unwrap())
}

#[geario::main]
async fn main() -> io::Result<()> {
    let addr = std::env::var("BENCH_ADDR").unwrap_or_else(|_| "127.0.0.1:18093".into());

    if let Ok(secs) = std::env::var("BENCH_SECONDS") {
        if let Ok(secs) = secs.parse::<u64>() {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(secs));
                std::process::exit(0);
            });
        }
    }

    let workers: usize = std::env::var("BENCH_WORKERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()));

    geario::server::net::build()
        .disable_signals()
        .workers(workers)
        .bind("bench", addr, SharedCfg::new("BENCH"), async |_| {
            fn_service(async |io: Io| {
                // hyper drives the connection; geario supplied the socket.
                let _ = http1::Builder::new()
                    .serve_connection(GearioTransport::new(io), service_fn(handle))
                    .await;
                Ok::<_, io::Error>(())
            })
        })?
        .run()
        .await
}

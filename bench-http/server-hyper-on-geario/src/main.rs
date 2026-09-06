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

const BODY: &[u8] = b"hello from the benchmark";

async fn handle(_req: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    Ok(Response::builder()
        .header("content-type", "text/plain")
        .body(Full::new(Bytes::from_static(BODY)))
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

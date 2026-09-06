//! hyper, native Rust, HTTP/1.1.
//!
//! Answers with the same bytes and headers as the geario server, so the
//! client can check both are doing the same work.
use std::convert::Infallible;
use std::net::SocketAddr;

use bytes::Bytes;
use http_body_util::Full;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;

const BODY: &[u8] = b"hello from the benchmark";

async fn handle(_req: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    Ok(Response::builder()
        .header("content-type", "text/plain")
        .body(Full::new(Bytes::from_static(BODY)))
        .unwrap())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = std::env::var("BENCH_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:18091".into())
        .parse()?;

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

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()?;

    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        loop {
            let (stream, _) = listener.accept().await?;
            stream.set_nodelay(true).ok();
            tokio::task::spawn(async move {
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service_fn(handle))
                    .await;
            });
        }
    })
}

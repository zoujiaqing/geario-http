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

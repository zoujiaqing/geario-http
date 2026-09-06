//! hyper's HTTP/2 on tokio, single threaded.
//!
//! Single threaded to match geario's one worker: a work-stealing runtime
//! against a thread-per-core one on the same core count is not the same
//! measurement.
use std::convert::Infallible;

use bytes::Bytes;
use http_body_util::Full;
use hyper::server::conn::http2;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};

fn body_bytes() -> &'static [u8] {
    use std::sync::OnceLock;
    static BODY: OnceLock<Vec<u8>> = OnceLock::new();
    BODY.get_or_init(|| {
        let n: usize = std::env::var("BENCH_BODY_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(24);
        b"hello from the benchmark ".iter().copied().cycle().take(n).collect()
    })
    .as_slice()
}

async fn handle(_: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    Ok(Response::builder()
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from_static(body_bytes())))
        .unwrap())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let addr = std::env::var("BENCH_ADDR").unwrap_or_else(|_| "127.0.0.1:18096".into());
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        stream.set_nodelay(true).ok();
        tokio::task::spawn(async move {
            let _ = http2::Builder::new(TokioExecutor::new())
                .timer(TokioTimer::new())
                .serve_connection(TokioIo::new(stream), service_fn(handle))
                .await;
        });
    }
}

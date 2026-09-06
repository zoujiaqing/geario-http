//! hyper's HTTP/2 running on geario's IO, executor and timer.
use std::convert::Infallible;
use std::io;

use bytes::Bytes;
use geario::io::Io;
use geario::service::cfg::SharedCfg;
use geario::service::fn_service;
use geario_http::hyper_rt::{GearioExecutor, GearioTimer, GearioTransport};
use http_body_util::Full;
use hyper::server::conn::http2;
use hyper::service::service_fn;
use hyper::{Request, Response};

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

#[geario::main]
async fn main() -> io::Result<()> {
    let addr = std::env::var("BENCH_ADDR").unwrap_or_else(|_| "127.0.0.1:18095".into());
    let workers: usize = std::env::var("BENCH_WORKERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);

    geario::server::net::build()
        .disable_signals()
        .workers(workers)
        .bind("bench", addr, SharedCfg::new("BENCH"), async |_| {
            fn_service(async |io: Io| {
                let _ = http2::Builder::new(GearioExecutor)
                    .timer(GearioTimer::new())
                    .serve_connection(GearioTransport::new(io), service_fn(handle))
                    .await;
                Ok::<_, io::Error>(())
            })
        })?
        .run()
        .await
}

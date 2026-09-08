//! geario-http, native Rust, HTTP/1.1.
//!
//! Both servers in this comparison answer with the same bytes and the same
//! headers, so the client can check it is talking to the same thing.
use std::io;

use geario::service::cfg::SharedCfg;
use geario_http::{HttpService, Request, Response};


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

#[geario::main]
async fn main() -> io::Result<()> {
    let addr = std::env::var("BENCH_ADDR").unwrap_or_else(|_| "127.0.0.1:18090".into());

    // Exiting on its own lets a profiler write its output; killing the
    // profiler instead loses the recording.
    if let Ok(secs) = std::env::var("BENCH_SECONDS") {
        if let Ok(secs) = secs.parse::<u64>() {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(secs));
                std::process::exit(0);
            });
        }
    }

    // Worker count is configurable so the comparison can be run with one
    // worker each, which takes connection distribution out of the picture.
    let workers: usize = std::env::var("BENCH_WORKERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()));

    let driver = if cfg!(feature = "uring") {
        "neon-uring"
    } else if cfg!(feature = "polling") {
        "neon-polling"
    } else {
        "neon-default"
    };
    eprintln!(
        "server-geario driver={} workers={} body={}",
        driver,
        workers,
        std::env::var("BENCH_BODY_SIZE").unwrap_or_else(|_| "24".into()),
    );

    geario::server::net::build()
        .disable_signals()
        .workers(workers)
        .bind("bench", addr, SharedCfg::new("BENCH"), async |_| {
            HttpService::new(async move |mut req: Request| {
                // Drain the request body. A server that skips it is not doing
                // the work the POST case is meant to measure.
                if req.method() == geario_http::Method::POST {
                    use geario::util::future::Stream;
                    let mut pl = req.take_payload();
                    loop {
                        let next = std::future::poll_fn(|cx| {
                            std::pin::Pin::new(&mut pl).poll_next(cx)
                        })
                        .await;
                        match next {
                            Some(Ok(_)) => {}
                            _ => break,
                        }
                    }
                }
                Ok::<_, io::Error>(
                    Response::Ok()
                        .header("content-type", "application/json")
                        .body(body_bytes()),
                )
            })
            .build()
        })?
        .run()
        .await
}

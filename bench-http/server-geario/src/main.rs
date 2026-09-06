//! geario-http, native Rust, HTTP/1.1.
//!
//! Both servers in this comparison answer with the same bytes and the same
//! headers, so the client can check it is talking to the same thing.
use std::io;

use geario::service::cfg::SharedCfg;
use geario_http::{HttpService, Request, Response};

const BODY: &[u8] = b"hello from the benchmark";

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

    geario::server::net::build()
        .disable_signals()
        .workers(workers)
        .bind("bench", addr, SharedCfg::new("BENCH"), async |_| {
            HttpService::new(async move |_req: Request| {
                Ok::<_, io::Error>(
                    Response::Ok()
                        .header("content-type", "text/plain")
                        .body(BODY),
                )
            })
            .build()
        })?
        .run()
        .await
}

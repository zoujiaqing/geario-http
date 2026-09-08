//! ntex 4.0 (main) HTTP/1.1 server on ntex's native neon runtime.
//!
//! Raw HttpService, no web/routing layer, to match server-geario (which uses
//! geario-http's HttpService directly). Same fixed-size application/json body.
use ntex::http::{HttpService, Response};
use ntex::SharedCfg;

const NTEX_REV: &str = "8af6d0271c1f4a3226c79a92740f7e9bf31347cd";

fn driver() -> &'static str {
    if cfg!(feature = "uring") {
        "neon-uring"
    } else if cfg!(feature = "polling") {
        "neon-polling"
    } else {
        "neon-default"
    }
}

fn body_bytes() -> &'static [u8] {
    use std::sync::OnceLock;
    static BODY: OnceLock<Vec<u8>> = OnceLock::new();
    BODY.get_or_init(|| {
        let n: usize = std::env::var("BENCH_BODY_SIZE").ok().and_then(|v| v.parse().ok()).unwrap_or(24);
        b"hello from the benchmark ".iter().copied().cycle().take(n).collect()
    })
    .as_slice()
}

#[ntex::main]
async fn main() -> std::io::Result<()> {
    let addr = std::env::var("BENCH_ADDR").unwrap_or_else(|_| "127.0.0.1:18086".into());
    let workers: usize = std::env::var("BENCH_WORKERS").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    eprintln!(
        "server-ntex4 ntex@{} driver={} workers={} body={}",
        NTEX_REV,
        driver(),
        if workers == 0 { "default(all)".into() } else { workers.to_string() },
        std::env::var("BENCH_BODY_SIZE").unwrap_or_else(|_| "24".into()),
    );
    let mut b = ntex::server::build();
    if workers > 0 {
        b = b.workers(workers);
    }
    b.bind("bench", addr, SharedCfg::new("BENCH"), async |_| {
        HttpService::h1(async |mut req: ntex::http::Request| {
            if req.method() == ntex::http::Method::POST {
                let mut pl = req.take_payload();
                use ntex::util::stream_recv;
                while (stream_recv(&mut pl).await).is_some() {}
            }
            Ok::<_, std::io::Error>(
                Response::Ok().content_type("application/json").body(body_bytes()),
            )
        })
    })?
    .run()
    .await
}

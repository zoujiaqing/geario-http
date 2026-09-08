//! ntex HTTP/1.1 server, the counterpart to server-geario (native h1).
//!
//! Same fixed-size body and headers as the geario server, so the load client
//! can check the response rather than only count it.
use ntex::http::{HttpService, Response};
use ntex::service::fn_service;

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
    let addr = std::env::var("BENCH_ADDR").unwrap_or_else(|_| "127.0.0.1:18085".into());
    ntex::server::build()
        .bind("bench", addr, |_| {
            HttpService::build().h1(fn_service(|mut req: ntex::http::Request| async move {
                if req.method() == ntex::http::Method::POST {
                    use ntex::util::stream_recv;
                    let mut pl = req.take_payload();
                    while (stream_recv(&mut pl).await).is_some() {}
                }
                Ok::<_, std::io::Error>(
                    Response::Ok()
                        .content_type("application/json")
                        .body(body_bytes()),
                )
            }))
        })?
        .run()
        .await
}

//! Minimal HTTP/1.1 server.
//!
//! Run with `cargo run -p geario-http --example hello`, then
//! `curl -v http://127.0.0.1:8080/`.
use std::io;

use geario::service::cfg::SharedCfg;
use geario_http::{HttpService, Request, Response};

#[geario::main]
async fn main() -> io::Result<()> {
    env_logger::init();

    let cfg = SharedCfg::new("HELLO");

    geario::server::net::build()
        .bind("hello", "127.0.0.1:8080", cfg, async |_| {
            HttpService::new(async move |_req: Request| {
                Ok::<_, io::Error>(Response::Ok().body("Hello from geario-http\n"))
            })
            .build()
        })?
        .run()
        .await
}

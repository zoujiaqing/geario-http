//! Drives the geario-http client against the geario-http server.
//!
//! Run with `cargo run -p geario-http --example roundtrip`.
use std::io;

use geario::service::cfg::SharedCfg;
use geario_http::client::Client;
use geario_http::{HttpService, Request, Response};

#[geario::main]
async fn main() -> io::Result<()> {
    env_logger::init();

    let srv = geario::server::net::build()
        .bind("rt", "127.0.0.1:8081", SharedCfg::new("SRV"), async |_| {
            HttpService::new(async move |req: Request| {
                Ok::<_, io::Error>(Response::Ok().body(format!("you asked for {}\n", req.path())))
            })
            .build()
        })?
        .run();

    geario::util::time::sleep(geario::util::time::Millis(200)).await;

    let client = Client::new();
    for path in ["/one", "/two", "/three"] {
        let res = client
            .get(format!("http://127.0.0.1:8081{path}"))
            .send()
            .await
            .expect("request failed");
        let body = res.body().await.expect("body failed");
        print!(
            "{} {} -> {}",
            path,
            res.status(),
            String::from_utf8_lossy(&body)
        );
    }

    srv.stop(false).await;
    Ok(())
}

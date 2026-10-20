//! Arena baseline only: real query parsing, body collection and integer addition.
use std::io;

use geario::bytes::BytesMut;
use geario::service::cfg::SharedCfg;
use geario::util::future::Stream;
use geario_http::{HttpService, Method, Request, Response};
#[path = "../../../arena-common.rs"]
mod common;
use common::sum;

async fn handle(mut req: Request) -> Result<Response, io::Error> {
    if req.path() != "/baseline11" {
        return Ok(Response::NotFound().finish());
    }
    if req.method() != Method::GET && req.method() != Method::POST {
        return Ok(Response::MethodNotAllowed().finish());
    }
    let mut body = BytesMut::new();
    if req.method() == Method::POST {
        let mut payload = req.take_payload();
        while let Some(chunk) =
            std::future::poll_fn(|cx| std::pin::Pin::new(&mut payload).poll_next(cx)).await
        {
            match chunk {
                Ok(bytes) if body.len() + bytes.len() <= 1024 * 1024 => {
                    body.extend_from_slice(&bytes)
                }
                _ => return Ok(Response::BadRequest().finish()),
            }
        }
    }
    Ok(match sum(req.uri().query().unwrap_or(""), &body) {
        Ok(value) => Response::Ok()
            .header("server", "geario-http")
            .header("content-type", "text/plain; charset=utf-8")
            .body(value.to_string()),
        Err(()) => Response::BadRequest().finish(),
    })
}

#[geario::main]
async fn main() -> io::Result<()> {
    let addr = std::env::var("BENCH_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let workers = std::thread::available_parallelism()?.get();
    eprintln!("geario-http Arena baseline: workers={workers}, addr={addr}");
    geario::server::net::build()
        .workers(workers)
        .bind("baseline", addr, SharedCfg::new("ARENA"), async |_| {
            HttpService::new(handle).build()
        })?
        .run()
        .await
}

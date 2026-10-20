//! Arena baseline only: real query parsing, body collection and integer addition.
use std::io;

use geario::bytes::BytesMut;
use geario::service::cfg::SharedCfg;
use geario::util::future::Stream;
use geario_http::{HttpService, Method, Request, Response};
use serde::Deserialize;

#[derive(Deserialize)]
struct Query {
    a: Option<i64>,
    b: Option<i64>,
}

fn sum(query: &str, body: &[u8]) -> Result<i64, ()> {
    let query: Query = serde_urlencoded::from_str(query).map_err(|_| ())?;
    let extra = std::str::from_utf8(body).ok()
        .and_then(|s| s.trim().parse::<i64>().ok()).unwrap_or(0);
    query.a.unwrap_or(0).checked_add(query.b.unwrap_or(0))
        .and_then(|n| n.checked_add(extra)).ok_or(())
}

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
        while let Some(chunk) = std::future::poll_fn(|cx| {
            std::pin::Pin::new(&mut payload).poll_next(cx)
        }).await {
            match chunk {
                Ok(bytes) if body.len() + bytes.len() <= 1024 * 1024 =>
                    body.extend_from_slice(&bytes),
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
        .run().await
}

#[cfg(test)]
mod tests {
    use super::sum;

    #[test]
    fn computes_inputs_not_a_fixed_response() {
        assert_eq!(sum("a=3&b=4", b""), Ok(7));
        assert_eq!(sum("a=10&b=-2", b" 5\n"), Ok(13));
        assert_eq!(sum("%61=11&b=2", b"9"), Ok(22));
        assert_eq!(sum("", b"-5"), Ok(-5));
    }

    #[test]
    fn rejects_invalid_query_and_overflow() {
        assert!(sum("a=no", b"").is_err());
        assert!(sum("a=9223372036854775807&b=1", b"").is_err());
    }
}

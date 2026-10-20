//! Arena baseline with Hyper HTTP/1 on geario, without a Tokio runtime.
use std::{convert::Infallible, io};

use bytes::Bytes;
use geario::{
    bytes::BytesMut,
    io::Io,
    service::{cfg::SharedCfg, fn_service},
};
use geario_http::hyper_rt::{GearioTimer, GearioTransport};
use http_body_util::{BodyExt, Full};
use hyper::{
    Method, Request, Response, StatusCode, body::Incoming, server::conn::http1, service::service_fn,
};

#[path = "../../../arena-common.rs"]
mod common;

fn response(status: StatusCode, body: String) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = status;
    if status == StatusCode::OK {
        response
            .headers_mut()
            .insert("server", "geario-http".parse().unwrap());
        response
            .headers_mut()
            .insert("content-type", "text/plain; charset=utf-8".parse().unwrap());
    }
    response
}

async fn handle(mut req: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    if req.uri().path() != "/baseline11" {
        return Ok(response(StatusCode::NOT_FOUND, String::new()));
    }
    if req.method() != Method::GET && req.method() != Method::POST {
        return Ok(response(StatusCode::METHOD_NOT_ALLOWED, String::new()));
    }
    let mut body = BytesMut::new();
    if req.method() == Method::POST {
        while let Some(frame) = req.body_mut().frame().await {
            match frame {
                Ok(frame) => {
                    if let Ok(bytes) = frame.into_data() {
                        if body.len() + bytes.len() > 1024 * 1024 {
                            return Ok(response(StatusCode::BAD_REQUEST, String::new()));
                        }
                        body.extend_from_slice(&bytes);
                    }
                }
                Err(_) => return Ok(response(StatusCode::BAD_REQUEST, String::new())),
            }
        }
    }
    Ok(match common::sum(req.uri().query().unwrap_or(""), &body) {
        Ok(value) => response(StatusCode::OK, value.to_string()),
        Err(()) => response(StatusCode::BAD_REQUEST, String::new()),
    })
}

#[geario::main]
async fn main() -> io::Result<()> {
    let addr = std::env::var("BENCH_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let workers = std::thread::available_parallelism()?.get();
    eprintln!("Hyper-on-geario Arena baseline: workers={workers}, addr={addr}");
    geario::server::net::build()
        .workers(workers)
        .bind("baseline", addr, SharedCfg::new("ARENA"), async |_| {
            fn_service(async |io: Io| {
                if let Err(error) = http1::Builder::new()
                    .timer(GearioTimer::new())
                    .serve_connection(GearioTransport::new(io), service_fn(handle))
                    .await
                {
                    eprintln!("connection error: {error}");
                }
                Ok::<_, io::Error>(())
            })
        })?
        .run()
        .await
}

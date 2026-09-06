//! The same HTTP/1.1 streaming echo, over tokio's transport.
//!
//! A control for `hyper_transport.rs`. If this fails too, the test is wrong
//! rather than the geario adapter.
#![cfg(feature = "hyper-rt")]

use std::convert::Infallible;

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;

async fn echo(req: Request<Incoming>) -> Result<Response<Incoming>, Infallible> {
    Ok(Response::new(req.into_body()))
}

fn body(seed: u8) -> Bytes {
    (0..131_071u32)
        .map(|i| (i as u8).wrapping_add(seed))
        .collect::<Vec<_>>()
        .into()
}

#[test]
fn http1_streaming_post_and_keep_alive_on_tokio() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let lst = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = lst.local_addr().unwrap();

        tokio::task::spawn_local(async move {
            let (s, _) = lst.accept().await.unwrap();
            s.set_nodelay(true).ok();
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(s), service_fn(echo))
                .await;
        });

        let c = tokio::net::TcpStream::connect(addr).await.unwrap();
        c.set_nodelay(true).ok();
        let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(c))
            .await
            .unwrap();
        tokio::task::spawn_local(async move {
            let _ = conn.await;
        });

        for seed in 0..3u8 {
            let expected = body(seed);
            let req = Request::post("/echo")
                .header("host", "test")
                .body(Full::new(expected.clone()))
                .unwrap();
            let resp = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                sender.send_request(req),
            )
            .await
            .expect("send_request timed out on tokio")
            .expect("send_request failed on tokio");
            assert_eq!(resp.status(), 200);
            let got = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                resp.into_body().collect(),
            )
            .await
            .expect("body collect timed out on tokio")
            .unwrap()
            .to_bytes();
            assert_eq!(got, expected, "seed {seed}");
        }
    });
}

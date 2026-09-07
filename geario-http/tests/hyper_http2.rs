//! HTTP/2 over geario, both ends.
//!
//! hyper owns the protocol; geario supplies the socket, the executor and the
//! timer. h2 exercises parts of the adapter that h1 never reaches: the
//! executor, because every stream is a spawned task, and concurrent writes
//! from several streams interleaving on one connection.
#![cfg(all(feature = "hyper-rt", feature = "server"))]

use std::convert::Infallible;

use geario::bytes::Bytes;
use geario::service::cfg::SharedCfg;
use geario_http::hyper_rt::{GearioExecutor, GearioTimer, GearioTransport};
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper::{Request, Response};

/// Serve one h2c connection, echoing the request body back.
fn serve() -> std::net::SocketAddr {
    let lst = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = lst.local_addr().unwrap();

    geario::rt::spawn(async move {
        let accepted = geario::rt::spawn_blocking(move || lst.accept()).await;
        let Ok(Ok((stream, _))) = accepted else {
            return;
        };
        stream.set_nonblocking(true).ok();
        let Ok(io) = geario::net::from_tcp_stream(stream, SharedCfg::new("H2-SRV").into()) else {
            return;
        };
        let _ = hyper::server::conn::http2::Builder::new(GearioExecutor)
            .timer(GearioTimer::new())
            .serve_connection(
                GearioTransport::new(io),
                service_fn(|req: Request<hyper::body::Incoming>| async move {
                    let path = req.uri().path().to_owned();
                    let body = req.into_body().collect().await.unwrap().to_bytes();
                    Ok::<_, Infallible>(
                        Response::builder()
                            .header("x-path", path)
                            .body(Full::new(body))
                            .unwrap(),
                    )
                }),
            )
            .await;
    });

    addr
}

#[geario::test]
async fn request_and_response_over_h2() {
    let addr = serve();
    let io = geario::net::tcp_connect(addr, SharedCfg::new("H2-CLI").into())
        .await
        .expect("connect");

    let (mut sender, conn) = hyper::client::conn::http2::Builder::new(GearioExecutor)
        .timer(GearioTimer::new())
        .handshake::<_, Full<Bytes>>(GearioTransport::new(io))
        .await
        .expect("handshake");
    geario::rt::spawn(async move {
        let _ = conn.await;
    });

    let res = sender
        .send_request(
            Request::builder()
                .uri("http://localhost/one")
                .body(Full::new(Bytes::from_static(b"body of the request")))
                .unwrap(),
        )
        .await
        .expect("send");

    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()["x-path"], "/one");
    let body = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"body of the request");
}

/// Several streams in flight at once on one connection. This is where the
/// executor and the interleaved writes are actually under test; a single
/// request/response would not tell h2 apart from h1.
#[geario::test]
async fn concurrent_streams_on_one_connection() {
    let addr = serve();
    let io = geario::net::tcp_connect(addr, SharedCfg::new("H2-CLI").into())
        .await
        .expect("connect");

    let (sender, conn) = hyper::client::conn::http2::Builder::new(GearioExecutor)
        .timer(GearioTimer::new())
        .handshake::<_, Full<Bytes>>(GearioTransport::new(io))
        .await
        .expect("handshake");
    geario::rt::spawn(async move {
        let _ = conn.await;
    });

    // Bodies large enough to span several frames, so the streams genuinely
    // interleave rather than each completing within one write.
    let mut pending = Vec::new();
    for i in 0..16u8 {
        let mut sender = sender.clone();
        let payload = Bytes::from(vec![i; 40_000]);
        pending.push(geario::rt::spawn(async move {
            let res = sender
                .send_request(
                    Request::builder()
                        .uri(format!("http://localhost/s{i}"))
                        .body(Full::new(payload))
                        .unwrap(),
                )
                .await
                .expect("send");
            let path = res.headers()["x-path"].to_str().unwrap().to_owned();
            let body = res.into_body().collect().await.unwrap().to_bytes();
            (path, body)
        }));
    }

    for (i, task) in pending.into_iter().enumerate() {
        let (path, body) = task.await.expect("stream");
        let i = u8::try_from(i).unwrap();
        assert_eq!(
            path,
            format!("/s{i}"),
            "a response landed on the wrong stream"
        );
        assert_eq!(body.len(), 40_000);
        assert!(
            body.iter().all(|b| *b == i),
            "stream {i} got another stream's bytes"
        );
    }
}

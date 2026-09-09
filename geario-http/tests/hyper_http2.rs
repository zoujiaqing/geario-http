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

/// Dropping one stream's response cancels that stream (hyper sends RST_STREAM).
/// The other streams on the same connection must be untouched: a cancel that
/// corrupted the shared connection would surface as a wrong or missing body on
/// a survivor. Bounded by a timeout so a stall fails instead of hanging.
#[geario::test]
async fn a_cancelled_stream_leaves_the_others_intact() {
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

    // Open a stream and drop its response without reading the body, cancelling
    // it while the survivors below share the same connection.
    {
        let mut s = sender.clone();
        let res = s
            .send_request(
                Request::builder()
                    .uri("http://localhost/cancel")
                    .body(Full::new(Bytes::from(vec![0xAAu8; 40_000])))
                    .unwrap(),
            )
            .await
            .expect("send");
        drop(res);
    }

    let mut pending = Vec::new();
    for i in 0..8u8 {
        let mut s = sender.clone();
        let payload = Bytes::from(vec![i; 40_000]);
        pending.push(geario::rt::spawn(async move {
            let res = s
                .send_request(
                    Request::builder()
                        .uri(format!("http://localhost/s{i}"))
                        .body(Full::new(payload))
                        .unwrap(),
                )
                .await
                .expect("send");
            let body = res.into_body().collect().await.unwrap().to_bytes();
            (i, body)
        }));
    }

    geario::util::time::timeout(geario::util::time::Millis(10_000), async {
        for task in pending {
            let (i, body) = task.await.expect("stream");
            assert_eq!(body.len(), 40_000, "survivor {i} truncated");
            assert!(
                body.iter().all(|b| *b == i),
                "survivor {i} got another stream's bytes"
            );
        }
    })
    .await
    .expect("survivors stalled after the cancel");
}

/// A stream whose reader is slow must not stall a concurrent stream. Per-stream
/// flow control should throttle the slow reader (its window fills and the
/// server's send backs off) without blocking the connection, so a fast stream
/// still completes promptly. If the connection were blocked, the fast stream
/// would hit its timeout.
#[geario::test]
async fn a_slow_reader_does_not_stall_a_concurrent_stream() {
    use http_body_util::BodyExt;

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

    // A large body (well past the default per-stream window) read frame by
    // frame with pauses, so its flow-control window genuinely fills.
    let slow = {
        let mut s = sender.clone();
        geario::rt::spawn(async move {
            let res = s
                .send_request(
                    Request::builder()
                        .uri("http://localhost/slow")
                        .body(Full::new(Bytes::from(vec![7u8; 400_000])))
                        .unwrap(),
                )
                .await
                .expect("send");
            let mut body = res.into_body();
            let mut got = 0usize;
            while let Some(frame) = body.frame().await {
                if let Some(data) = frame.expect("frame").data_ref() {
                    got += data.len();
                }
                geario::util::time::sleep(geario::util::time::Millis(20)).await;
            }
            got
        })
    };

    // The fast stream shares the connection and must finish quickly even while
    // the slow reader is throttled.
    let mut f = sender.clone();
    let fast = geario::util::time::timeout(geario::util::time::Millis(5_000), async move {
        let res = f
            .send_request(
                Request::builder()
                    .uri("http://localhost/fast")
                    .body(Full::new(Bytes::from(vec![9u8; 40_000])))
                    .unwrap(),
            )
            .await
            .expect("send");
        res.into_body().collect().await.unwrap().to_bytes()
    })
    .await
    .expect("fast stream stalled behind the slow reader");
    assert_eq!(fast.len(), 40_000);
    assert!(fast.iter().all(|b| *b == 9), "fast stream corrupted");

    // The slow stream still delivers its whole body once fully read.
    let got = geario::util::time::timeout(geario::util::time::Millis(30_000), async {
        slow.await.expect("slow stream task")
    })
    .await
    .expect("slow stream timed out");
    assert_eq!(got, 400_000, "slow stream truncated");
}

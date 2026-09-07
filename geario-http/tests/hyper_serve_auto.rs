//! One listener, both protocols: the first bytes decide.
#![cfg(feature = "hyper-full")]

use std::convert::Infallible;
use std::io::Write;

use geario::bytes::Bytes;
use geario::service::cfg::SharedCfg;
use geario::util::channel::oneshot;
use geario_http::hyper_rt::{GearioExecutor, GearioTransport, Protocol, detect, serve_auto};
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper::{Request, Response};

fn listen() -> std::net::TcpListener {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap()
}

/// Accept one connection and hand it to `serve_auto` with a service that
/// reports the path and the version it was reached with.
fn serve_one(lst: std::net::TcpListener) -> std::net::SocketAddr {
    let addr = lst.local_addr().unwrap();
    geario::rt::spawn(async move {
        let accepted = geario::rt::spawn_blocking(move || lst.accept()).await;
        let Ok(Ok((stream, _))) = accepted else {
            return;
        };
        stream.set_nonblocking(true).ok();
        let Ok(io) = geario::net::from_tcp_stream(stream, SharedCfg::new("SRV").into()) else {
            return;
        };
        let _ = serve_auto(
            io,
            service_fn(|req: Request<hyper::body::Incoming>| async move {
                Ok::<_, Infallible>(
                    Response::builder()
                        .header("x-path", req.uri().path())
                        .header("x-version", format!("{:?}", req.version()))
                        .body(Full::new(Bytes::from_static(b"ok")))
                        .unwrap(),
                )
            }),
        )
        .await;
    });
    addr
}

#[geario::test]
async fn http1_is_served_as_http1() {
    let addr = serve_one(listen());
    let io = geario::net::tcp_connect(addr, SharedCfg::new("CLI").into())
        .await
        .unwrap();
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(GearioTransport::new(io))
            .await
            .unwrap();
    geario::rt::spawn(async move {
        let _ = conn.await;
    });
    let res = sender
        .send_request(
            Request::builder()
                .uri("/one")
                .header("host", "t")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()["x-version"], "HTTP/1.1");
    assert_eq!(res.headers()["x-path"], "/one");
    assert_eq!(
        &res.into_body().collect().await.unwrap().to_bytes()[..],
        b"ok"
    );
}

#[geario::test]
async fn http2_prior_knowledge_is_served_as_http2() {
    let addr = serve_one(listen());
    let io = geario::net::tcp_connect(addr, SharedCfg::new("CLI").into())
        .await
        .unwrap();
    let (mut sender, conn) = hyper::client::conn::http2::Builder::new(GearioExecutor)
        .handshake::<_, Full<Bytes>>(GearioTransport::new(io))
        .await
        .unwrap();
    geario::rt::spawn(async move {
        let _ = conn.await;
    });
    let res = sender
        .send_request(
            Request::builder()
                .uri("http://t/two")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(res.version(), hyper::Version::HTTP_2);
    assert_eq!(res.headers()["x-version"], "HTTP/2.0");
    assert_eq!(res.headers()["x-path"], "/two");
}

/// Run `detect` on whatever one raw client sends, and report the verdict.
fn detect_one(lst: std::net::TcpListener) -> (std::net::SocketAddr, oneshot::Receiver<Protocol>) {
    let addr = lst.local_addr().unwrap();
    let (tx, rx) = oneshot::channel();
    geario::rt::spawn(async move {
        let accepted = geario::rt::spawn_blocking(move || lst.accept()).await;
        let Ok(Ok((stream, _))) = accepted else {
            return;
        };
        stream.set_nonblocking(true).ok();
        let Ok(io) = geario::net::from_tcp_stream(stream, SharedCfg::new("SRV").into()) else {
            return;
        };
        if let Ok(p) = detect(&io).await {
            let _ = tx.send(p);
        }
    });
    (addr, rx)
}

/// The preface arriving in two pieces is still the preface. A detector that
/// judged the first piece alone would call this HTTP/1 and hand h2 frames to
/// an h1 parser.
#[geario::test]
async fn a_preface_split_across_writes_is_still_http2() {
    let (addr, rx) = detect_one(listen());
    let mut raw = std::net::TcpStream::connect(addr).unwrap();
    let preface = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
    raw.write_all(&preface[..10]).unwrap();
    geario::util::time::sleep(geario::util::time::Millis(50)).await;
    raw.write_all(&preface[10..]).unwrap();
    assert_eq!(rx.await.unwrap(), Protocol::Http2);
}

#[geario::test]
async fn a_request_line_is_http1_without_waiting_for_24_bytes() {
    let (addr, rx) = detect_one(listen());
    let mut raw = std::net::TcpStream::connect(addr).unwrap();
    // Shorter than the preface, and already not a prefix of it.
    raw.write_all(b"GET / HT").unwrap();
    assert_eq!(rx.await.unwrap(), Protocol::Http1);
}

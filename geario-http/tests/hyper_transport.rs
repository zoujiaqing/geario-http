#![cfg(feature = "hyper-rt")]

use std::{convert::Infallible, future::poll_fn};

use geario::service::cfg::SharedCfg;
use geario::util::time::{Seconds, timeout};
use geario_http::hyper_rt::{GearioExecutor, GearioTransport};
use http_body_util::{BodyExt, Full};
use hyper::{
    Request, Response,
    body::{Bytes, Incoming},
    service::service_fn,
};

fn pair() -> (
    GearioTransport<geario::io::Base>,
    GearioTransport<geario::io::Base>,
) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let a = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (b, _) = listener.accept().unwrap();
    a.set_nodelay(true).unwrap();
    b.set_nodelay(true).unwrap();
    (
        GearioTransport::new(
            geario::net::from_tcp_stream(a, SharedCfg::new("CLIENT").into()).unwrap(),
        ),
        GearioTransport::new(
            geario::net::from_tcp_stream(b, SharedCfg::new("SERVER").into()).unwrap(),
        ),
    )
}

async fn echo(req: Request<Incoming>) -> Result<Response<Incoming>, Infallible> {
    Ok(Response::new(req.into_body()))
}

fn body(seed: u8) -> Bytes {
    (0..131_071)
        .map(|i| (i as u8).wrapping_add(seed))
        .collect::<Vec<_>>()
        .into()
}

#[geario::test]
async fn http1_streaming_post_and_keep_alive() {
    let _ = env_logger::try_init();
    timeout(Seconds(5), async {
        let (client, server) = pair();
        geario::rt::spawn(async move {
            hyper::server::conn::http1::Builder::new()
                .serve_connection(server, service_fn(echo))
                .await
                .unwrap();
        });
        let (mut sender, conn) = hyper::client::conn::http1::handshake(client).await.unwrap();
        geario::rt::spawn(async move {
            conn.await.unwrap();
        });
        for seed in 0..3 {
            eprintln!("seed {seed}");
            let expected = body(seed);
            let req = Request::post("/echo")
                .header("host", "test")
                .body(Full::new(expected.clone()))
                .unwrap();
            let resp = sender.send_request(req).await.unwrap();
            eprintln!("response {:?}", resp.headers());
            assert_eq!(resp.status(), 200);
            assert_eq!(
                resp.into_body().collect().await.unwrap().to_bytes(),
                expected
            );
            poll_fn(|cx| sender.poll_ready(cx)).await.unwrap();
        }
    })
    .await
    .expect("HTTP/1 transfer stalled");
}

#[geario::test]
async fn http2_concurrent_streams_cross_flow_control_window() {
    timeout(Seconds(5), async {
        let (client, server) = pair();
        geario::rt::spawn(async move {
            hyper::server::conn::http2::Builder::new(GearioExecutor)
                .serve_connection(server, service_fn(echo))
                .await
                .unwrap();
        });
        let (sender, conn) = hyper::client::conn::http2::handshake(GearioExecutor, client)
            .await
            .unwrap();
        geario::rt::spawn(async move {
            conn.await.unwrap();
        });
        let mut jobs = Vec::new();
        for seed in 0..4 {
            let mut sender = sender.clone();
            jobs.push(async move {
                let expected = body(seed);
                let req = Request::post("http://test/echo")
                    .body(Full::new(expected.clone()))
                    .unwrap();
                let resp = sender.send_request(req).await.unwrap();
                assert_eq!(resp.version(), hyper::Version::HTTP_2);
                assert_eq!(
                    resp.into_body().collect().await.unwrap().to_bytes(),
                    expected
                );
            });
        }
        futures_util::future::join_all(jobs).await;
    })
    .await
    .expect("HTTP/2 flow control stalled");
}

/// A disconnect that was not asked for must still be an error.
///
/// The flush path treats `NotConnected` as success once shutdown has been
/// requested, so a completed transfer does not end by reporting failure. This
/// checks the exemption does not extend to a peer that simply vanished.
#[geario::test]
async fn unrequested_disconnect_is_still_an_error() {
    use hyper::rt::Write as _;
    use std::pin::Pin;

    let (peer, stream) = geario::io::testing::IoTest::create();
    peer.remote_buffer_cap(0);
    let cfg: geario::service::cfg::SharedCfg = SharedCfg::new("DROP").into();
    let mut io = GearioTransport::new(geario::io::Io::new(stream, cfg));

    // Queue bytes the peer will never take, then have it vanish. No shutdown
    // was requested, so the failure has to surface.
    let _ = poll_fn(|cx| Pin::new(&mut io).poll_write(cx, b"unsent")).await;
    peer.close().await;

    let mut last = Ok(());
    for _ in 0..50 {
        match poll_fn(|cx| Pin::new(&mut io).poll_flush(cx)).await {
            Ok(()) => {
                geario::util::time::sleep(geario::util::time::Millis(10)).await;
                last = Ok(());
            }
            Err(e) => {
                last = Err(e);
                break;
            }
        }
    }
    assert!(
        last.is_err(),
        "a peer that vanished without a shutdown request was reported as success"
    );
}

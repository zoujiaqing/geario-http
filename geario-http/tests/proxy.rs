//! Plaintext requests through an HTTP proxy.
#![cfg(feature = "client")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;

use geario_http::client::Client;
use geario_http::client::proxy::ProxyTarget;

/// A proxy that records the request line and answers with a fixed body.
///
/// Deliberately not a real proxy: what is being tested is that the client
/// connects here at all and that the request line names the destination.
fn fake_proxy() -> (String, mpsc::Receiver<String>) {
    let lst = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = lst.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        for stream in lst.incoming() {
            let Ok(mut s) = stream else { break };
            let mut buf = [0u8; 2048];
            let Ok(n) = s.read(&mut buf) else { continue };
            let text = String::from_utf8_lossy(&buf[..n]).to_string();
            let first = text.lines().next().unwrap_or("").to_string();
            let _ = tx.send(first);
            let _ = s.write_all(
                b"HTTP/1.1 200 OK\r\ncontent-length: 7\r\nconnection: close\r\n\r\nviaprox",
            );
            let _ = s.flush();
        }
    });

    (format!("http://{addr}"), rx)
}

#[geario::test]
async fn plaintext_goes_through_the_proxy_in_absolute_form() {
    let (proxy_url, seen) = fake_proxy();
    let target = ProxyTarget::parse(&proxy_url).expect("proxy url");

    let client = Client::builder()
        .proxy(target)
        .build(geario::service::cfg::SharedCfg::new("PROXY"));

    // Nothing listens on this host, so a direct connection could not succeed.
    // Getting a body back proves the proxy was used.
    let res = client
        .get("http://example.invalid/some/path?q=1")
        .send()
        .await
        .expect("request through the proxy");

    assert_eq!(res.status().as_u16(), 200);

    let line = seen
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert!(
        line.starts_with("GET http://example.invalid/some/path?q=1 "),
        "request line was not absolute-form: {line:?}"
    );
}

#[geario::test]
async fn tls_through_a_proxy_is_refused_rather_than_sent_direct() {
    let (proxy_url, _seen) = fake_proxy();
    let target = ProxyTarget::parse(&proxy_url).expect("proxy url");

    let client = Client::builder()
        .proxy(target)
        .build(geario::service::cfg::SharedCfg::new("PROXY"));

    let res = client.get("https://example.invalid/").send().await;

    // Silently going direct would defeat whatever the proxy was there for.
    assert!(res.is_err(), "an https request was not refused");
}

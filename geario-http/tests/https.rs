//! HTTPS through the rustls connector.
#![cfg(feature = "rustls")]

use geario_http::client::Client;

/// Needs the network. Run with `cargo test --features full,rustls -- --ignored`.
#[ignore]
#[geario::test]
async fn fetches_over_https() {
    let _ = tls_rustls::crypto::aws_lc_rs::default_provider().install_default();

    let client = Client::new();
    let res = client
        .get("https://www.rust-lang.org/")
        .send()
        .await
        .expect("request failed");

    assert!(
        res.status().is_success() || res.status().is_redirection(),
        "unexpected status {}",
        res.status()
    );
}

/// The connector must actually validate. Without this the test above would
/// pass just as well against a proxy that terminates TLS with its own
/// certificate.
///
/// Needs the network. Run with `cargo test --features full,rustls -- --ignored`.
#[ignore]
#[geario::test]
async fn refuses_an_expired_certificate() {
    let _ = tls_rustls::crypto::aws_lc_rs::default_provider().install_default();

    let client = Client::new();
    let res = client.get("https://expired.badssl.com/").send().await;

    assert!(
        res.is_err(),
        "an expired certificate was accepted: {:?}",
        res.map(|r| r.status())
    );
}

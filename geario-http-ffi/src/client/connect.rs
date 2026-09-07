//! Opening a connection for the client: TCP, then TLS if the URL asks for it,
//! then whichever HTTP the handshake settled on.
//!
//! ALPN decides the protocol; the URL never does. A plaintext URL is HTTP/1.1
//! because there is no h2c client, which matches hyper4k. An `https` URL
//! offers h2 and http/1.1 and takes what the server picks, unless the host
//! required h2, in which case only h2 is offered and a server that cannot do
//! it fails the handshake rather than being quietly downgraded.

use std::sync::Arc;

use geario::io::types::HttpProtocol;
use geario::net::connect::{Connect, ConnectError, connect as tcp};
use geario::tls::rustls::TlsClientFilter;
use geario::util::time::{Millis, timeout_checked};
use geario_http::hyper_rt::{GearioExecutor, GearioTimer, GearioTransport};
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::client::conn::{http1, http2};
use tls_rustls::pki_types::pem::PemObject;
use tls_rustls::pki_types::{CertificateDer, ServerName};
use tls_rustls::{CertificateError, ClientConfig, RootCertStore};

use crate::abi::*;

/// Where a request is going: enough to key a connection on.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Origin {
    pub tls: bool,
    pub host: String,
    pub port: u16,
}

/// A failure with the kind the host is told and the text it is shown.
#[derive(Debug)]
pub(crate) struct Fail {
    pub kind: GearioHttpErrorKind,
    pub message: String,
}

impl Fail {
    pub fn new(kind: GearioHttpErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl Origin {
    /// Split a URL into where to connect and what to ask for.
    pub fn parse(url: &str) -> Result<(Origin, hyper::Uri), Fail> {
        let uri: hyper::Uri = url
            .parse()
            .map_err(|e| Fail::new(GEARIO_HTTP_ERR_INVALID_URL, format!("bad url: {e}")))?;
        let tls = match uri.scheme_str() {
            Some("http") => false,
            Some("https") => true,
            Some(other) => {
                return Err(Fail::new(
                    GEARIO_HTTP_ERR_INVALID_URL,
                    format!("unsupported scheme {other}"),
                ));
            }
            None => return Err(Fail::new(GEARIO_HTTP_ERR_INVALID_URL, "url has no scheme")),
        };
        let host = uri
            .host()
            .ok_or_else(|| Fail::new(GEARIO_HTTP_ERR_INVALID_URL, "url has no host"))?
            .trim_matches(['[', ']'])
            .to_owned();
        let port = uri.port_u16().unwrap_or(if tls { 443 } else { 80 });
        Ok((Origin { tls, host, port }, uri))
    }
}

/// One open connection, in whichever protocol it negotiated.
pub(crate) enum Sender {
    H1(http1::SendRequest<Full<Bytes>>),
    H2(http2::SendRequest<Full<Bytes>>),
    /// A lease whose h1 sender has been taken back out. Never sent on.
    Gone,
}

/// TLS settings shared by every connection a client opens.
pub(crate) struct Tls {
    pub config: Arc<ClientConfig>,
    pub require_h2: bool,
}

impl Tls {
    /// Build the client's TLS configuration.
    ///
    /// `custom_ca` is a PEM bundle added to the built-in roots, or used
    /// instead of them when `replace_system` is set. A bundle that yields no
    /// certificate is an error here rather than on the first request, where
    /// it would read as a network problem instead of a configuration one.
    pub fn new(
        custom_ca: Option<&[u8]>,
        replace_system: bool,
        require_h2: bool,
    ) -> Result<Tls, Fail> {
        let mut roots = RootCertStore::empty();
        if !replace_system {
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }
        if let Some(pem) = custom_ca {
            let mut added = 0;
            for cert in CertificateDer::pem_slice_iter(pem) {
                let cert = cert.map_err(|e| {
                    Fail::new(GEARIO_HTTP_ERR_TLS_CA, format!("custom CA bundle: {e}"))
                })?;
                roots.add(cert).map_err(|e| {
                    Fail::new(GEARIO_HTTP_ERR_TLS_CA, format!("custom CA bundle: {e}"))
                })?;
                added += 1;
            }
            if added == 0 {
                return Err(Fail::new(
                    GEARIO_HTTP_ERR_TLS_CA,
                    "custom CA bundle holds no certificate",
                ));
            }
        }
        let mut config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        // Offering only h2 makes a peer that cannot do h2 fail the handshake,
        // which is what HTTP2_REQUIRED means: fail, never silently downgrade.
        config.alpn_protocols = if require_h2 {
            vec![b"h2".to_vec()]
        } else {
            vec![b"h2".to_vec(), b"http/1.1".to_vec()]
        };
        Ok(Tls {
            config: Arc::new(config),
            require_h2,
        })
    }
}

/// Open a connection to `origin` and finish the HTTP handshake on it.
///
/// `connect_timeout` covers everything up to a usable connection: DNS, TCP,
/// TLS and the HTTP handshake. Zero disables it.
pub(crate) async fn connect(
    origin: &Origin,
    tls: Option<&Tls>,
    connect_timeout: Millis,
) -> Result<Sender, Fail> {
    match timeout_checked(connect_timeout, open(origin, tls)).await {
        Ok(result) => result,
        Err(()) => Err(Fail::new(
            GEARIO_HTTP_ERR_TIMEOUT,
            format!("connecting to {}:{} timed out", origin.host, origin.port),
        )),
    }
}

async fn open(origin: &Origin, tls: Option<&Tls>) -> Result<Sender, Fail> {
    let io = tcp(Connect::new(origin.host.clone()).set_port(origin.port))
        .await
        .map_err(|e| classify_connect(&e))?;

    if !origin.tls {
        return handshake_h1(io).await;
    }
    let Some(tls) = tls else {
        return Err(Fail::new(
            GEARIO_HTTP_ERR_UNSUPPORTED,
            "this client was built without TLS",
        ));
    };

    let name = ServerName::try_from(origin.host.clone()).map_err(|_| {
        Fail::new(
            GEARIO_HTTP_ERR_TLS_HOSTNAME,
            format!("{} is not a valid server name", origin.host),
        )
    })?;
    let io = TlsClientFilter::create(io, tls.config.clone(), name)
        .await
        .map_err(|e| classify_tls(&e))?;

    // ALPN decides the protocol; the URL never does.
    let h2 = matches!(io.query::<HttpProtocol>().get(), Some(HttpProtocol::Http2));
    if tls.require_h2 && !h2 {
        return Err(Fail::new(
            GEARIO_HTTP_ERR_ALPN_NO_H2,
            "HTTP/2 was required and the server did not negotiate it",
        ));
    }
    if h2 {
        let (sender, conn) = http2::Builder::new(GearioExecutor)
            .timer(GearioTimer::new())
            .handshake::<_, Full<Bytes>>(GearioTransport::new(io))
            .await
            .map_err(|e| Fail::new(GEARIO_HTTP_ERR_PROTOCOL, format!("h2 handshake: {e}")))?;
        geario::rt::spawn(async move {
            let _ = conn.await;
        });
        Ok(Sender::H2(sender))
    } else {
        handshake_h1(io).await
    }
}

async fn handshake_h1<F: geario::io::Filter + Unpin>(
    io: geario::io::Io<F>,
) -> Result<Sender, Fail> {
    let (sender, conn) = http1::Builder::new()
        .handshake::<_, Full<Bytes>>(GearioTransport::new(io))
        .await
        .map_err(|e| Fail::new(GEARIO_HTTP_ERR_PROTOCOL, format!("h1 handshake: {e}")))?;
    geario::rt::spawn(async move {
        let _ = conn.await;
    });
    Ok(Sender::H1(sender))
}

fn classify_connect(e: &geario::error::Error<ConnectError>) -> Fail {
    let kind = match &**e {
        ConnectError::Resolver(_)
        | ConnectError::NoRecords
        | ConnectError::InvalidInput
        | ConnectError::Unresolved => GEARIO_HTTP_ERR_DNS,
        ConnectError::Io(_) => GEARIO_HTTP_ERR_CONNECT,
    };
    Fail::new(kind, format!("{}", &**e))
}

/// "Cannot connect" and "the certificate is wrong" are different problems
/// for whoever is on call; one opaque kind would make both unactionable.
fn classify_tls(e: &std::io::Error) -> Fail {
    let kind = match e
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<tls_rustls::Error>())
    {
        Some(tls_rustls::Error::InvalidCertificate(cert)) => match cert {
            CertificateError::UnknownIssuer | CertificateError::BadSignature => {
                GEARIO_HTTP_ERR_TLS_CA
            }
            CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => {
                GEARIO_HTTP_ERR_TLS_HOSTNAME
            }
            CertificateError::Expired
            | CertificateError::ExpiredContext { .. }
            | CertificateError::NotValidYet
            | CertificateError::NotValidYetContext { .. } => GEARIO_HTTP_ERR_TLS_EXPIRED,
            _ => GEARIO_HTTP_ERR_TLS_OTHER,
        },
        // Either side can be the one to notice there is no protocol in
        // common: this one, or the server, which says so in an alert.
        Some(tls_rustls::Error::NoApplicationProtocol)
        | Some(tls_rustls::Error::AlertReceived(
            tls_rustls::AlertDescription::NoApplicationProtocol,
        )) => GEARIO_HTTP_ERR_ALPN_NO_H2,
        Some(_) => GEARIO_HTTP_ERR_TLS_OTHER,
        // The socket failed underneath the handshake.
        None => GEARIO_HTTP_ERR_CONNECT,
    };
    Fail::new(kind, format!("tls: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use geario::service::cfg::SharedCfg;
    use std::convert::Infallible;

    use geario::tls::rustls::TlsServerFilter;
    use geario_http::hyper_rt::serve_auto;
    use http_body_util::BodyExt;
    use hyper::service::service_fn;
    use hyper::{Request, Response};
    use tls_rustls::ServerConfig;
    use tls_rustls::pki_types::PrivateKeyDer;

    struct Pki {
        ca_pem: String,
        cert: CertificateDer<'static>,
        key: PrivateKeyDer<'static>,
    }

    fn issue(host: &str) -> Pki {
        let mut ca_params = rcgen::CertificateParams::new(Vec::new()).unwrap();
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_key = rcgen::KeyPair::generate().unwrap();
        let ca = ca_params.self_signed(&ca_key).unwrap();
        let leaf_params = rcgen::CertificateParams::new(vec![host.to_owned()]).unwrap();
        let leaf_key = rcgen::KeyPair::generate().unwrap();
        let leaf = leaf_params.signed_by(&leaf_key, &ca, &ca_key).unwrap();
        Pki {
            ca_pem: ca.pem(),
            cert: leaf.der().clone(),
            key: PrivateKeyDer::try_from(leaf_key.serialize_der()).unwrap(),
        }
    }

    async fn answer(
        req: Request<hyper::body::Incoming>,
    ) -> Result<Response<Full<Bytes>>, Infallible> {
        Ok(Response::builder()
            .header("x-version", format!("{:?}", req.version()))
            .body(Full::new(Bytes::from_static(b"ok")))
            .unwrap())
    }

    fn plaintext_server() -> u16 {
        let lst = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = lst.local_addr().unwrap().port();
        geario::rt::spawn(async move {
            let Ok(Ok((s, _))) = geario::rt::spawn_blocking(move || lst.accept()).await else {
                return;
            };
            s.set_nonblocking(true).ok();
            let Ok(io) = geario::net::from_tcp_stream(s, SharedCfg::new("SRV").into()) else {
                return;
            };
            let _ = serve_auto(io, service_fn(answer)).await;
        });
        port
    }

    /// A TLS server offering the given ALPN protocols, serving h2 or h1
    /// according to what it negotiated.
    fn tls_server(pki: &Pki, alpn: &[&[u8]]) -> u16 {
        let mut cfg = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![pki.cert.clone()], pki.key.clone_key())
            .unwrap();
        cfg.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
        let cfg = Arc::new(cfg);
        let lst = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = lst.local_addr().unwrap().port();
        geario::rt::spawn(async move {
            let Ok(Ok((s, _))) = geario::rt::spawn_blocking(move || lst.accept()).await else {
                return;
            };
            s.set_nonblocking(true).ok();
            let Ok(io) = geario::net::from_tcp_stream(s, SharedCfg::new("SRV").into()) else {
                return;
            };
            let Ok(io) = TlsServerFilter::create(io, cfg, Millis(5_000)).await else {
                return;
            };
            if matches!(io.query::<HttpProtocol>().get(), Some(HttpProtocol::Http2)) {
                let _ = hyper::server::conn::http2::Builder::new(GearioExecutor)
                    .serve_connection(GearioTransport::new(io), service_fn(answer))
                    .await;
            } else {
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(GearioTransport::new(io), service_fn(answer))
                    .await;
            }
        });
        port
    }

    fn origin(tls: bool, port: u16) -> Origin {
        Origin {
            tls,
            host: "localhost".into(),
            port,
        }
    }

    async fn roundtrip(sender: &mut Sender) -> Response<hyper::body::Incoming> {
        let req = Request::builder()
            .uri("https://localhost/x")
            .body(Full::new(Bytes::new()))
            .unwrap();
        match sender {
            Sender::H1(s) => s.send_request(req).await.unwrap(),
            Sender::H2(s) => s.send_request(req).await.unwrap(),
            Sender::Gone => unreachable!(),
        }
    }

    #[geario::test]
    async fn plaintext_is_http1() {
        let port = plaintext_server();
        let mut s = connect(&origin(false, port), None, Millis(5_000))
            .await
            .unwrap();
        assert!(matches!(s, Sender::H1(_)));
        let res = roundtrip(&mut s).await;
        assert_eq!(res.headers()["x-version"], "HTTP/1.1");
    }

    #[geario::test]
    async fn tls_with_h2_negotiated_is_http2() {
        let pki = issue("localhost");
        let port = tls_server(&pki, &[b"h2", b"http/1.1"]);
        let tls = Tls::new(Some(pki.ca_pem.as_bytes()), true, false).unwrap();
        let mut s = connect(&origin(true, port), Some(&tls), Millis(5_000))
            .await
            .unwrap();
        assert!(matches!(s, Sender::H2(_)));
        let res = roundtrip(&mut s).await;
        assert_eq!(res.version(), hyper::Version::HTTP_2);
        assert_eq!(
            &res.into_body().collect().await.unwrap().to_bytes()[..],
            b"ok"
        );
    }

    #[geario::test]
    async fn tls_server_without_h2_is_http1_unless_h2_is_required() {
        let pki = issue("localhost");
        let port = tls_server(&pki, &[b"http/1.1"]);
        let tls = Tls::new(Some(pki.ca_pem.as_bytes()), true, false).unwrap();
        let s = connect(&origin(true, port), Some(&tls), Millis(5_000))
            .await
            .unwrap();
        assert!(matches!(s, Sender::H1(_)));

        let port = tls_server(&pki, &[b"http/1.1"]);
        let tls = Tls::new(Some(pki.ca_pem.as_bytes()), true, true).unwrap();
        let err = connect(&origin(true, port), Some(&tls), Millis(5_000))
            .await
            .err()
            .unwrap();
        assert_eq!(err.kind, GEARIO_HTTP_ERR_ALPN_NO_H2, "{}", err.message);
    }

    #[geario::test]
    async fn an_untrusted_issuer_is_tls_ca() {
        let pki = issue("localhost");
        let unrelated = issue("localhost");
        let port = tls_server(&pki, &[b"h2"]);
        let tls = Tls::new(Some(unrelated.ca_pem.as_bytes()), true, false).unwrap();
        let err = connect(&origin(true, port), Some(&tls), Millis(5_000))
            .await
            .err()
            .unwrap();
        assert_eq!(err.kind, GEARIO_HTTP_ERR_TLS_CA, "{}", err.message);
    }

    #[geario::test]
    async fn a_certificate_for_another_name_is_tls_hostname() {
        let pki = issue("localhost");
        let port = tls_server(&pki, &[b"h2"]);
        let tls = Tls::new(Some(pki.ca_pem.as_bytes()), true, false).unwrap();
        let mut o = origin(true, port);
        o.host = "127.0.0.1".into();
        let err = connect(&o, Some(&tls), Millis(5_000)).await.err().unwrap();
        assert_eq!(err.kind, GEARIO_HTTP_ERR_TLS_HOSTNAME, "{}", err.message);
    }

    /// A label longer than 63 bytes is rejected by the resolver itself, so
    /// this does not depend on the network: a resolver that answers every
    /// name, as some captive ones do, still cannot answer this one.
    #[geario::test]
    async fn a_name_that_does_not_resolve_is_dns() {
        let mut o = origin(false, 80);
        o.host = format!("{}.invalid", "a".repeat(70));
        let err = connect(&o, None, Millis(5_000)).await.err().unwrap();
        assert_eq!(err.kind, GEARIO_HTTP_ERR_DNS, "{}", err.message);
    }

    #[geario::test]
    async fn a_closed_port_is_connect() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let err = connect(&origin(false, port), None, Millis(5_000))
            .await
            .err()
            .unwrap();
        assert_eq!(err.kind, GEARIO_HTTP_ERR_CONNECT, "{}", err.message);
    }

    /// A peer that accepts and then says nothing has to be a timeout, and the
    /// timeout has to cover the TLS handshake, not only the TCP connect.
    #[geario::test]
    async fn a_silent_peer_is_timeout() {
        let lst = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = lst.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let held = lst.accept();
            std::thread::sleep(std::time::Duration::from_secs(3));
            drop(held);
        });
        let pki = issue("localhost");
        let tls = Tls::new(Some(pki.ca_pem.as_bytes()), true, false).unwrap();
        let err = connect(&origin(true, port), Some(&tls), Millis(200))
            .await
            .err()
            .unwrap();
        assert_eq!(err.kind, GEARIO_HTTP_ERR_TIMEOUT, "{}", err.message);
    }

    #[test]
    fn an_empty_ca_bundle_is_refused_at_construction() {
        let err = Tls::new(Some(b"not pem at all"), true, false)
            .err()
            .unwrap();
        assert_eq!(err.kind, GEARIO_HTTP_ERR_TLS_CA);
    }

    #[test]
    fn urls_split_into_origin_and_defaults() {
        let (o, _) = Origin::parse("http://example.com/a?b").unwrap();
        assert_eq!((o.tls, o.host.as_str(), o.port), (false, "example.com", 80));
        let (o, _) = Origin::parse("https://example.com:8443/").unwrap();
        assert_eq!(
            (o.tls, o.host.as_str(), o.port),
            (true, "example.com", 8443)
        );
        assert_eq!(
            Origin::parse("ftp://x/").err().unwrap().kind,
            GEARIO_HTTP_ERR_INVALID_URL
        );
        assert_eq!(
            Origin::parse("/relative").err().unwrap().kind,
            GEARIO_HTTP_ERR_INVALID_URL
        );
    }
}

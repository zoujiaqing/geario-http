//! Per-origin connection reuse.
//!
//! Thread-local, like everything else on a worker: a connection is only ever
//! touched by the worker that opened it, so the pool is `Rc<RefCell<..>>` with
//! no locking. HTTP/2 multiplexes, so one connection per origin is shared and
//! handed out by cloning its sender. HTTP/1 does not, so idle connections are
//! kept in a list and a busy one is simply a new connection.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use geario::util::time::Millis;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::client::conn::{http1, http2};

use super::connect::{Fail, Origin, Sender, Tls, connect};

#[derive(Default)]
struct Inner {
    /// One shared multiplexed connection per origin.
    h2: HashMap<Origin, http2::SendRequest<Full<Bytes>>>,
    /// Idle single-use connections, newest last.
    h1_idle: HashMap<Origin, Vec<http1::SendRequest<Full<Bytes>>>>,
}

/// A connection borrowed from the pool for one request.
///
/// An HTTP/1 lease has to be returned once the response body is read, or the
/// connection is dropped rather than reused. An HTTP/2 lease is a clone of the
/// shared sender and needs no return; `checkin` on it does nothing.
pub(crate) struct Lease {
    pub sender: Sender,
    origin: Origin,
    pool: Pool,
    done: bool,
}

impl Lease {
    /// Return the connection to the pool if it can serve another request.
    ///
    /// Only HTTP/1 keeps idle connections; an HTTP/2 sender stays in the pool
    /// the whole time and is not held here.
    pub fn checkin(mut self) {
        self.done = true;
        if let Sender::H1(s) = std::mem::replace(&mut self.sender, Sender::Gone) {
            if s.is_ready() {
                self.pool
                    .inner
                    .borrow_mut()
                    .h1_idle
                    .entry(self.origin.clone())
                    .or_default()
                    .push(s);
            }
        }
    }
}

/// Handle to the worker's connection pool.
#[derive(Clone)]
pub(crate) struct Pool {
    inner: Rc<RefCell<Inner>>,
    tls: Option<Rc<Tls>>,
    connect_timeout: Millis,
}

impl Pool {
    pub fn new(tls: Option<Tls>, connect_timeout: Millis) -> Pool {
        Pool {
            inner: Rc::new(RefCell::new(Inner::default())),
            tls: tls.map(Rc::new),
            connect_timeout,
        }
    }

    /// Borrow a connection to `origin`, opening one if none can be reused.
    pub async fn checkout(&self, origin: &Origin) -> Result<Lease, Fail> {
        // A live shared h2 connection is cloned without a round trip.
        if let Some(sender) = self.inner.borrow().h2.get(origin) {
            if sender.is_ready() {
                return Ok(self.lease(origin.clone(), Sender::H2(sender.clone())));
            }
        }
        // A dead one is cleared so the reconnect below replaces it.
        self.inner.borrow_mut().h2.remove(origin);

        // An idle h1 connection that is still ready serves the request.
        loop {
            let idle = self
                .inner
                .borrow_mut()
                .h1_idle
                .get_mut(origin)
                .and_then(Vec::pop);
            match idle {
                Some(s) if s.is_ready() => {
                    return Ok(self.lease(origin.clone(), Sender::H1(s)));
                }
                // Not ready: drop it and look at the next.
                Some(_) => continue,
                None => break,
            }
        }

        // Nothing to reuse. Open a connection; if it is h2, share it.
        let sender = connect(origin, self.tls.as_deref(), self.connect_timeout).await?;
        if let Sender::H2(s) = &sender {
            self.inner.borrow_mut().h2.insert(origin.clone(), s.clone());
        }
        Ok(self.lease(origin.clone(), sender))
    }

    fn lease(&self, origin: Origin, sender: Sender) -> Lease {
        Lease {
            sender,
            origin,
            pool: self.clone(),
            done: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use geario::service::cfg::SharedCfg;
    use geario::tls::rustls::TlsServerFilter;
    use geario_http::hyper_rt::{GearioExecutor, GearioTransport};
    use http_body_util::BodyExt;
    use hyper::service::service_fn;
    use hyper::{Request, Response};
    use tls_rustls::ServerConfig;
    use tls_rustls::pki_types::{CertificateDer, PrivateKeyDer};

    fn issue() -> (String, CertificateDer<'static>, PrivateKeyDer<'static>) {
        let mut ca_params = rcgen::CertificateParams::new(Vec::new()).unwrap();
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_key = rcgen::KeyPair::generate().unwrap();
        let ca = ca_params.self_signed(&ca_key).unwrap();
        let leaf_params = rcgen::CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
        let leaf_key = rcgen::KeyPair::generate().unwrap();
        let leaf = leaf_params.signed_by(&leaf_key, &ca, &ca_key).unwrap();
        (
            ca.pem(),
            leaf.der().clone(),
            PrivateKeyDer::try_from(leaf_key.serialize_der()).unwrap(),
        )
    }

    async fn drain(lease: &mut Lease) -> hyper::http::Version {
        let req = Request::builder()
            .uri("https://localhost/x")
            .header("host", "localhost")
            .body(Full::new(Bytes::new()))
            .unwrap();
        let res = match &mut lease.sender {
            Sender::H1(s) => s.send_request(req).await.unwrap(),
            Sender::H2(s) => s.send_request(req).await.unwrap(),
            Sender::Gone => unreachable!(),
        };
        let v = res.version();
        let _ = res.into_body().collect().await.unwrap();
        v
    }

    /// A listener that counts how many connections it accepts, so reuse is
    /// observable rather than assumed.
    fn counting_server(http2: bool, close_after_one: bool) -> (Origin, Arc<AtomicU32>) {
        let lst = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = lst.local_addr().unwrap().port();
        let accepts = Arc::new(AtomicU32::new(0));
        let seen = accepts.clone();
        geario::rt::spawn(async move {
            loop {
                let Ok(Ok((s, _))) = geario::rt::spawn_blocking({
                    let lst = lst.try_clone().unwrap();
                    move || lst.accept()
                })
                .await
                else {
                    return;
                };
                seen.fetch_add(1, Ordering::SeqCst);
                s.set_nonblocking(true).ok();
                let Ok(io) = geario::net::from_tcp_stream(s, SharedCfg::new("SRV").into()) else {
                    continue;
                };
                geario::rt::spawn(async move {
                    let svc = service_fn(move |_req| async move {
                        let mut b = Response::builder();
                        if close_after_one {
                            b = b.header("connection", "close");
                        }
                        Ok::<_, Infallible>(b.body(Full::new(Bytes::from_static(b"ok"))).unwrap())
                    });
                    if http2 {
                        let _ = hyper::server::conn::http2::Builder::new(GearioExecutor)
                            .serve_connection(GearioTransport::new(io), svc)
                            .await;
                    } else {
                        let _ = hyper::server::conn::http1::Builder::new()
                            .serve_connection(GearioTransport::new(io), svc)
                            .await;
                    }
                });
            }
        });
        (
            Origin {
                tls: false,
                host: "localhost".into(),
                port,
            },
            accepts,
        )
    }

    #[geario::test]
    async fn two_sequential_http1_requests_reuse_one_connection() {
        let (origin, accepts) = counting_server(false, false);
        let pool = Pool::new(None, Millis(5_000));

        let mut a = pool.checkout(&origin).await.unwrap();
        assert_eq!(drain(&mut a).await, hyper::http::Version::HTTP_11);
        a.checkin();

        let mut b = pool.checkout(&origin).await.unwrap();
        drain(&mut b).await;
        b.checkin();

        // Give the accept loop a moment in case a second connection was opened.
        geario::util::time::sleep(Millis(100)).await;
        assert_eq!(
            accepts.load(Ordering::SeqCst),
            1,
            "the connection was not reused"
        );
    }

    /// A TLS listener that negotiates h2 and counts connections.
    fn counting_tls_h2_server(
        pki: &(String, CertificateDer<'static>, PrivateKeyDer<'static>),
    ) -> (Origin, Arc<AtomicU32>) {
        let mut cfg = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![pki.1.clone()], pki.2.clone_key())
            .unwrap();
        cfg.alpn_protocols = vec![b"h2".to_vec()];
        let cfg = Arc::new(cfg);
        let lst = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = lst.local_addr().unwrap().port();
        let accepts = Arc::new(AtomicU32::new(0));
        let seen = accepts.clone();
        geario::rt::spawn(async move {
            loop {
                let Ok(Ok((s, _))) = geario::rt::spawn_blocking({
                    let lst = lst.try_clone().unwrap();
                    move || lst.accept()
                })
                .await
                else {
                    return;
                };
                seen.fetch_add(1, Ordering::SeqCst);
                s.set_nonblocking(true).ok();
                let Ok(io) = geario::net::from_tcp_stream(s, SharedCfg::new("SRV").into()) else {
                    continue;
                };
                let cfg = cfg.clone();
                geario::rt::spawn(async move {
                    let Ok(io) = TlsServerFilter::create(io, cfg, Millis(5_000)).await else {
                        return;
                    };
                    let _ = hyper::server::conn::http2::Builder::new(GearioExecutor)
                        .serve_connection(
                            GearioTransport::new(io),
                            service_fn(|_req| async {
                                Ok::<_, Infallible>(
                                    Response::builder()
                                        .body(Full::new(Bytes::from_static(b"ok")))
                                        .unwrap(),
                                )
                            }),
                        )
                        .await;
                });
            }
        });
        (
            Origin {
                tls: true,
                host: "localhost".into(),
                port,
            },
            accepts,
        )
    }

    #[geario::test]
    async fn concurrent_http2_requests_share_one_connection() {
        let pki = issue();
        let (origin, accepts) = counting_tls_h2_server(&pki);
        let tls = Tls::new(Some(pki.0.as_bytes()), true, false).unwrap();
        let pool = Pool::new(Some(tls), Millis(5_000));

        // First request opens the shared connection.
        let mut first = pool.checkout(&origin).await.unwrap();
        assert_eq!(drain(&mut first).await, hyper::http::Version::HTTP_2);
        first.checkin();

        // Now several at once all clone the one sender.
        let mut leases = Vec::new();
        for _ in 0..8 {
            leases.push(pool.checkout(&origin).await.unwrap());
        }
        for mut l in leases {
            assert_eq!(drain(&mut l).await, hyper::http::Version::HTTP_2);
            l.checkin();
        }

        geario::util::time::sleep(Millis(100)).await;
        assert_eq!(
            accepts.load(Ordering::SeqCst),
            1,
            "h2 opened more than one connection"
        );
    }

    #[geario::test]
    async fn a_closed_http1_connection_is_not_reused() {
        let (origin, accepts) = counting_server(false, true);
        let pool = Pool::new(None, Millis(5_000));

        let mut a = pool.checkout(&origin).await.unwrap();
        drain(&mut a).await;
        // The server said Connection: close, so this must not go back to idle.
        a.checkin();
        geario::util::time::sleep(Millis(100)).await;

        let mut b = pool.checkout(&origin).await.unwrap();
        drain(&mut b).await;
        b.checkin();

        geario::util::time::sleep(Millis(100)).await;
        assert_eq!(
            accepts.load(Ordering::SeqCst),
            2,
            "a closed connection was handed back out"
        );
    }
}

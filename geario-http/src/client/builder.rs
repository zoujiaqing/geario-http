use crate::Uri;
use geario::error::Error;
use geario::io::IoBoxed;
use geario::net::connect::{self, Connect as TcpConnect, Connector as TcpConnector};
use geario::service::cfg::SharedCfg;
use geario::service::{Identity, IntoService, Middleware, Pipeline, Service, Stack, apply_fn};

use super::connector::Connector;
use super::error::{ClientError, ConnectError};
use super::pool::ConnectionPool;
use super::service::{ServiceRequest, ServiceResponse};
use super::{Client, ClientConfig, Connect, ConnectorPipeline, sender::Sender};

#[cfg(feature = "rustls")]
use tls_rustls::ClientConfig as RustlsClientConfig;

/// An HTTP Client builder.
///
/// This type can be used to construct an instance of `Client` through a
/// builder-like pattern.
#[derive(Debug)]
pub struct ClientBuilder<M = Identity> {
    middleware: M,
    svc: ConnectorPipeline,
    secure_svc: Option<ConnectorPipeline>,
    proxy: Option<super::proxy::ProxyTarget>,
}

impl Default for ClientBuilder<Identity> {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientBuilder<Identity> {
    #[must_use]
    /// Create new client builder instance.
    pub fn new() -> Self {
        let svc = ConnectorPipeline::new(
            apply_fn(TcpConnector::new(), async move |msg: Connect, svc| {
                svc.call(TcpConnect::new(msg.uri).set_addr(msg.addr)).await
            })
            .map(IoBoxed::from)
            .map_err(|e| e.map(ConnectError::from)),
        );

        let builder = ClientBuilder {
            svc,
            secure_svc: None,
            proxy: None,
            middleware: Identity,
        };

        #[cfg(feature = "rustls")]
        {
            use tls_rustls::RootCertStore;

            // Only HTTP/1.1 is implemented, so advertising h2 would invite a
            // server to pick a protocol this build cannot speak.
            let protos = vec![b"http/1.1".to_vec()];
            let cert_store =
                RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let mut config = RustlsClientConfig::builder()
                .with_root_certificates(cert_store)
                .with_no_client_auth();
            config.alpn_protocols = protos;
            builder.rustls(config)
        }
        #[cfg(not(feature = "rustls"))]
        {
            builder
        }
    }
}

impl<M> ClientBuilder<M> {
    #[must_use]
    #[cfg(feature = "rustls")]
    /// Use rustls connector for secured connections.
    pub fn rustls(self, config: RustlsClientConfig) -> Self {
        use geario::tls::rustls::TlsConnector;

        self.secure_connector(TlsConnector::new(config))
    }

    #[must_use]
    /// Send plaintext requests through an HTTP proxy.
    ///
    /// The connection goes to the proxy and the request line carries the whole
    /// URI, which is how the proxy learns where it is going.
    ///
    /// TLS targets are **not** tunnelled yet. They are refused rather than
    /// sent direct: quietly bypassing a configured proxy would defeat whatever
    /// the proxy was there to do.
    pub fn proxy(mut self, target: super::proxy::ProxyTarget) -> Self {
        let uri: Uri = format!("http://{}", target.addr())
            .parse()
            .expect("a validated ProxyTarget always forms a URI");

        self.svc = ConnectorPipeline::new(
            apply_fn(TcpConnector::new(), async move |_msg: Connect, svc| {
                svc.call(TcpConnect::new(uri.clone())).await
            })
            .map(IoBoxed::from)
            .map_err(|e| e.map(ConnectError::from)),
        );
        self.proxy = Some(target);
        self
    }

    #[must_use]
    /// Use custom connector to open un-secured connections.
    pub fn connector<T>(mut self, f: impl IntoService<T, SharedCfg, TcpConnect<Uri>>) -> Self
    where
        T: Service<SharedCfg, TcpConnect<Uri>, Error = Error<connect::ConnectError>> + 'static,
        IoBoxed: From<T::Res>,
    {
        self.svc = ConnectorPipeline::new(
            apply_fn(f.into_service(), async move |msg: Connect, svc| {
                svc.call(TcpConnect::new(msg.uri).set_addr(msg.addr)).await
            })
            .map(IoBoxed::from)
            .map_err(|e| e.map(ConnectError::from)),
        );
        self
    }

    #[must_use]
    /// Use custom connector to open secure connections.
    pub fn secure_connector<T>(mut self, f: impl IntoService<T, SharedCfg, TcpConnect<Uri>>) -> Self
    where
        T: Service<SharedCfg, TcpConnect<Uri>, Error = Error<connect::ConnectError>> + 'static,
        IoBoxed: From<T::Res>,
    {
        self.secure_svc = Some(ConnectorPipeline::new(
            apply_fn(f.into_service(), async move |msg: Connect, svc| {
                svc.call(TcpConnect::new(msg.uri).set_addr(msg.addr)).await
            })
            .map(IoBoxed::from)
            .map_err(|e| e.map(ConnectError::from)),
        ));
        self
    }

    #[must_use]
    /// Apply middleware.
    ///
    /// Use middleware when you need to read or modify *every* request or
    /// response in some way.
    ///
    /// ```rust
    /// use geario_http::client::{Client, ServiceRequest};
    /// use geario::service::{fn_layer, cfg::SharedCfg};
    ///
    /// #[geario::main]
    /// async fn main() {
    ///     let client = Client::builder()
    ///         .middleware(fn_layer(
    ///             async move |mut req: ServiceRequest, svc| {
    ///                 println!("{:?}", req.head().uri);
    ///                 svc.call(req).await
    ///             }
    ///         ))
    ///         .build(SharedCfg::default());
    /// }
    /// ```
    pub fn middleware<U>(self, mw: U) -> ClientBuilder<Stack<U, M>> {
        ClientBuilder {
            middleware: Stack::new(mw, self.middleware),
            svc: self.svc,
            secure_svc: self.secure_svc,
            proxy: self.proxy,
        }
    }

    /// Finish build process and create `Client` instance.
    pub fn build(self, cfg: impl Into<SharedCfg>) -> Client
    where
        M: Middleware<Sender, SharedCfg, SharedCfg>,
        M::Service: Service<SharedCfg, ServiceRequest, Res = ServiceResponse, Error = Error<ClientError>>
            + 'static,
    {
        let cfg = cfg.into();
        let config = cfg.get::<ClientConfig>();
        let proxy = self.proxy;

        let connector = Connector {
            tcp_pool: ConnectionPool::new(self.svc, config.clone()),
            ssl_pool: self
                .secure_svc
                .map(|svc| ConnectionPool::new(svc, config.clone())),
        };
        let svc = self.middleware.create(Sender::new(connector, proxy), &cfg);

        Client::with_service(config, Pipeline::with(cfg, svc))
    }
}

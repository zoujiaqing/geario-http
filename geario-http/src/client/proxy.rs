//! HTTP proxy support.
//!
//! Plaintext targets go through the proxy in absolute-form: one connection,
//! and the proxy reads the destination out of the request line.
//!
//! TLS targets get a CONNECT tunnel first and then the usual handshake over
//! it, so certificate verification still happens against the real host. A
//! proxy that could terminate TLS itself would be a man in the middle.

use std::fmt;

use geario_http::Uri;

/// Where the proxy is, once its URL has been checked.
#[derive(Clone, PartialEq, Eq)]
pub struct ProxyTarget {
    pub host: String,
    pub port: u16,
}

impl fmt::Debug for ProxyTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ProxyTarget({}:{})", self.host, self.port)
    }
}

/// Why a proxy URL was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum ProxyError {
    /// Not a URL at all.
    Malformed,
    /// A scheme other than `http`. An `https` proxy needs TLS to the proxy
    /// itself, which is a different connection shape.
    UnsupportedScheme,
    /// No host to connect to.
    MissingHost,
    /// Credentials in the URL. Silently dropping them would look like the
    /// proxy was authenticated when it was not.
    CredentialsNotSupported,
}

impl ProxyTarget {
    /// Parse `http://host[:port]`.
    pub fn parse(raw: &str) -> Result<ProxyTarget, ProxyError> {
        let uri: Uri = raw.parse().map_err(|_| ProxyError::Malformed)?;

        match uri.scheme_str() {
            Some("http") => {}
            _ => return Err(ProxyError::UnsupportedScheme),
        }

        let authority = uri.authority().ok_or(ProxyError::MissingHost)?;
        if authority.as_str().contains('@') {
            return Err(ProxyError::CredentialsNotSupported);
        }

        let host = authority.host();
        if host.is_empty() {
            return Err(ProxyError::MissingHost);
        }

        Ok(ProxyTarget {
            host: host.to_owned(),
            port: authority.port_u16().unwrap_or(80),
        })
    }

    /// `host:port`, ready to connect to.
    pub fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Whether a target reached through this proxy needs a CONNECT tunnel.
    pub fn needs_tunnel(target: &Uri) -> bool {
        matches!(target.scheme_str(), Some("https" | "wss"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_and_default_port() {
        let p = ProxyTarget::parse("http://proxy.example").unwrap();
        assert_eq!(p.host, "proxy.example");
        assert_eq!(p.port, 80);
        assert_eq!(p.addr(), "proxy.example:80");

        let p = ProxyTarget::parse("http://127.0.0.1:3128").unwrap();
        assert_eq!(p.port, 3128);
    }

    #[test]
    fn refuses_what_it_cannot_honour() {
        // Credentials would be dropped, which would look like the proxy was
        // authenticated when it was not.
        assert_eq!(
            ProxyTarget::parse("http://user:pass@proxy.example"),
            Err(ProxyError::CredentialsNotSupported)
        );
        // An https proxy needs TLS to the proxy itself.
        assert_eq!(
            ProxyTarget::parse("https://proxy.example"),
            Err(ProxyError::UnsupportedScheme)
        );
        assert_eq!(
            ProxyTarget::parse("socks5://proxy.example"),
            Err(ProxyError::UnsupportedScheme)
        );
        assert_eq!(ProxyTarget::parse("not a url"), Err(ProxyError::Malformed));
    }

    #[test]
    fn only_tls_targets_need_a_tunnel() {
        let https: Uri = "https://example.com/x".parse().unwrap();
        let http: Uri = "http://example.com/x".parse().unwrap();
        assert!(ProxyTarget::needs_tunnel(&https));
        assert!(!ProxyTarget::needs_tunnel(&http));
    }
}

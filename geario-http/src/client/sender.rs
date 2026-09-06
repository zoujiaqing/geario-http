use crate::body::MessageBody;
#[cfg(feature = "compress")]
use crate::{Payload, encoding::Decoder};
use geario::error::Error;
use geario::service::Ctx;
use geario::service::Service;
use geario::service::cfg::SharedCfg;

use super::{ClientConfig, ClientRawRequest, Connect, ServiceRequest, ServiceResponse};
use super::{connector::Connector, error::ClientError};

#[derive(Debug)]
pub struct Sender {
    connector: Connector,
    proxy: Option<super::proxy::ProxyTarget>,
}

impl Sender {
    pub(super) fn new(connector: Connector, proxy: Option<super::proxy::ProxyTarget>) -> Self {
        Self { connector, proxy }
    }
}

#[allow(unused_variables)]
impl Service<SharedCfg, ServiceRequest> for Sender {
    type Res = ServiceResponse;
    type Error = Error<ClientError>;

    geario::forward_ready!(SharedCfg, connector);
    geario::forward_shutdown!(SharedCfg, connector);

    async fn call(
        &self,
        req: ServiceRequest,
        ctx: Ctx<'_, Self, SharedCfg>,
    ) -> Result<Self::Res, Self::Error> {
        let ServiceRequest {
            mut head,
            addr,
            body,
            headers,
            mut timeout,
            response_decompress,
        } = req;

        let uri = head.uri.clone();

        if self.proxy.is_some() {
            if super::proxy::ProxyTarget::needs_tunnel(&uri) {
                // Falling back to a direct connection would quietly defeat
                // whatever the proxy was there to do.
                return Err(Error::from(ClientError::ProxyTunnelNotSupported));
            }
            // The connection goes to the proxy, so the request line has to
            // carry the whole URI or the proxy has no idea where to forward.
            head.set_absolute_uri(true);
        }

        let con = ctx.call(&self.connector, Connect { uri, addr }).await?;
        let config = ctx.st().get::<ClientConfig>();

        if timeout.is_zero() {
            timeout = config.timeout();
        }

        let req = ClientRawRequest {
            head,
            headers,
            size: body.size(),
        };

        let (head, payload) = con.send_request(req, body, timeout).await?;

        #[cfg(feature = "compress")]
        if response_decompress {
            let payload = Payload::from_stream(Decoder::from_headers(payload, &head.headers));
            return Ok(ServiceResponse {
                head,
                payload,
                config: config.clone(),
            });
        }

        Ok(ServiceResponse {
            head,
            payload,
            config,
        })
    }
}

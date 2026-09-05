use std::{error::Error, marker::PhantomData, rc::Rc};

use geario::io::{Filter, Io, types};
use geario::service::{IntoServiceFactory, RequestState, pipeline::PipelineFactory};
use geario::service::{Ctx, Service, ServiceFactory};
use geario::util::dyn_rc_err;

use super::error::{DispatchError, ResponseError};
use super::{Request, Response, config::DispatcherConfig, h1};

type HttpPipeline<St, Cfg, Err> = PipelineFactory<St, Request, Response, Err, Cfg, String>;
type Ctl1Pipeline<St, F, Cfg, Err> =
    PipelineFactory<St, h1::Control<F, Err>, h1::ControlAck<F>, Rc<dyn Error>, Cfg, String>;

/// HTTP/1.1 transport implementation
#[derive(derive_more::Debug)]
#[debug("HttpService")]
pub struct HttpService<St, F, Req: RequestState<Io<F>>, Err> {
    sf: HttpPipeline<Req::State, St, Err>,
    h1_ctl: Ctl1Pipeline<Req::State, F, St, Err>,
    config: DispatcherConfig,
    ph: PhantomData<(St, F, Req)>,
}

impl<St, F, Req, Err> HttpService<St, F, Req, Err>
where
    St: 'static,
    F: Filter + 'static,
    Req: RequestState<Io<F>>,
    Req::State: Clone,
    Err: ResponseError + 'static,
{
    #[must_use]
    /// Create new `HttpService` instance.
    pub fn new<H>(sf: impl IntoServiceFactory<H, Req::State, Request, St>) -> Self
    where
        H: ServiceFactory<Req::State, Request, St, Error = Err> + 'static,
        H::Res: Into<Response>,
        H::InitError: Error,
    {
        HttpService {
            sf: PipelineFactory::new(
                sf.into_factory()
                    .map(Into::into)
                    .map_init_err(|e| format!("{e:?}")),
            ),
            h1_ctl: PipelineFactory::new(
                h1::DefaultControlService
                    .map_init_err(|_| "error".to_string())
                    .map_err(dyn_rc_err),
            ),
            config: DispatcherConfig::default(),
            ph: PhantomData,
        }
    }

    #[must_use]
    /// Create *http service* for HTTP/1 protocol.
    pub fn h1<H>(
        sf: impl IntoServiceFactory<H, Req::State, Request, St>,
    ) -> h1::H1Service<St, F, Req, H>
    where
        H: ServiceFactory<Req::State, Request, St, Error = Err> + 'static,
        H::Res: Into<Response>,
        H::InitError: Error,
    {
        h1::H1Service::new(sf)
    }

}

impl<St, F, Req, Err> HttpService<St, F, Req, Err>
where
    St: 'static,
    F: Filter,
    Req: RequestState<Io<F>>,
    Err: ResponseError + 'static,
{
    #[must_use]
    /// Provide http/1 control service.
    pub fn h1_control<Ctl>(
        self,
        ctl: impl IntoServiceFactory<Ctl, Req::State, h1::Control<F, Err>, St>,
    ) -> Self
    where
        Ctl: ServiceFactory<Req::State, h1::Control<F, Err>, St, Res = h1::ControlAck<F>> + 'static,
        Ctl::Error: Error,
        Ctl::InitError: Error,
    {
        HttpService {
            sf: self.sf,
            h1_ctl: PipelineFactory::new(
                ctl.into_factory()
                    .map_err(dyn_rc_err)
                    .map_init_err(|e| format!("{e:?}")),
            ),
            config: self.config,
            ph: self.ph,
        }
    }
}

impl<St, F, Req, Err> HttpService<St, F, Req, Err>
where
    St: 'static,
    F: Filter + 'static,
    Req: RequestState<Io<F>>,
    Req::State: Clone,
    Err: ResponseError + 'static,
{
    pub fn build(self) -> impl Service<St, Req, Res = (), Error = DispatchError> {
        self
    }
}

impl<St, F, Req, Err> Service<St, Req> for HttpService<St, F, Req, Err>
where
    St: 'static,
    F: Filter + 'static,
    Req: RequestState<Io<F>>,
    Req::State: Clone,
    Err: ResponseError + 'static,
{
    type Res = ();
    type Error = DispatchError;

    async fn call(&self, req: Req, ctx: Ctx<'_, Self, St>) -> Result<Self::Res, Self::Error> {
        let (io, st) = req.unpack();

        let svc = self.sf.create(ctx.st(), st.clone()).await.map_err(|e| {
            log::error!("Cannot construct handler service: {e}");
            DispatchError::Control
        })?;

        let id = self.config.next_id();
        let ioref = io.get_ref();
        let inflight = self.config.insert_io(&ioref);

        let result = {
            log::trace!(
                "{}: New http1 connection {id}, peer address {:?}, in-flight: {inflight}",
                io.tag(),
                io.query::<types::PeerAddr>().get(),
            );

            let ctl = self.h1_ctl.create(ctx.st(), st).await.map_err(|e| {
                log::error!("Cannot construct h1 control service: {e}");
                DispatchError::Control
            })?;

            h1::handle_io(id, io, svc, ctl, self.config.clone()).await
        };

        let inflight = self.config.remove_io(&ioref);
        if inflight == 0 && self.config.is_shutdown() {
            self.config.notify_shutdown();
        }
        result
    }

    async fn ready(&self, _: Ctx<'_, Self, St>) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn shutdown(&self, _: Ctx<'_, Self, St>) {
        // check inflight connections
        let inflight = self.config.shutdown();
        if inflight != 0 {
            log::trace!("Shutting down service, in-flight connections: {inflight}");

            self.config.wait_shutdown().await;
            log::trace!("Shutting down is complected");
        }
    }
}

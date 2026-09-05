use std::time::Instant;
use std::{cell::Cell, cell::RefCell, collections::VecDeque, fmt, future, rc::Rc};

use crate::uri::{Authority, Uri};
use geario::error::Error;
use geario::io::IoBoxed;
use geario::rt::spawn;
use geario::service::pipeline::{PipelineBinding, PipelineCall};
use geario::service::{Ctx, Service, cfg::Cfg, cfg::SharedCfg};
use geario::util::channel::inplace;
use geario::util::channel::oneshot;
use geario::util::channel::pool;
use geario::util::future::{Either, select};
use geario::util::time::now;
use geario::util::{HashMap, HashSet};

use super::connection::{Connection, ConnectionType};
use super::{ClientConfig, Connect, ConnectorPipeline, error::ConnectError};

#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub(super) struct Key {
    authority: Authority,
}

impl From<Authority> for Key {
    fn from(authority: Authority) -> Key {
        Key { authority }
    }
}

type Waiter = pool::Sender<Result<Connection, Error<ConnectError>>>;
type WaiterReceiver = pool::Receiver<Result<Connection, Error<ConnectError>>>;

enum Acquire {
    Acquired(ConnectionType, Instant),
    Available,
    NotAvailable,
}

#[derive(Debug)]
struct AvailableConnection {
    io: ConnectionType,
    used: Instant,
    created: Instant,
}

/// Connections pool
pub(super) struct ConnectionPool(Rc<ConnectionPoolInner>);

struct ConnectionPoolInner {
    cfg: SharedCfg,
    svc: ConnectorPipeline,
    inner: Rc<RefCell<Inner>>,
    waiters: Rc<RefCell<Waiters>>,
    stop: Rc<Cell<Option<oneshot::Sender<()>>>>,
}

#[derive(Debug)]
pub(super) struct Inner {
    cfg: Cfg<ClientConfig>,
    stopped: bool,
    acquired: usize,
    available: HashMap<Key, VecDeque<AvailableConnection>>,
    connecting: HashSet<Key>,
    waker: inplace::Inplace<()>,
    waiters: Rc<RefCell<Waiters>>,
}

impl ConnectionPool {
    pub(super) fn new(svc: ConnectorPipeline, cfg: Cfg<ClientConfig>) -> Self {
        let shared = cfg.shared();
        let waiters = Rc::new(RefCell::new(Waiters {
            waiters: HashMap::default(),
            pool: pool::new(),
        }));
        let inner = Rc::new(RefCell::new(Inner {
            cfg,
            stopped: false,
            acquired: 0,
            available: HashMap::default(),
            connecting: HashSet::default(),
            waker: inplace::channel(),
            waiters: waiters.clone(),
        }));

        // start connection pool
        let (stop, stop_rx) = oneshot::channel();
        geario::rt::spawn(run_connection_pool(
            shared.clone(),
            svc.bind_state(shared.clone()),
            inner.clone(),
            waiters.clone(),
            stop_rx,
        ));

        ConnectionPool(Rc::new(ConnectionPoolInner {
            svc,
            inner,
            waiters,
            cfg: shared,
            stop: Rc::new(Cell::new(Some(stop))),
        }))
    }
}

impl Drop for ConnectionPool {
    fn drop(&mut self) {
        if Rc::strong_count(&self.0) == 1 {
            self.0.stop.take();
            self.0.waiters.borrow_mut().waiters.clear();
            let mut inner = self.0.inner.borrow_mut();
            inner.stopped = true;
            let _ = inner.waker.send(());
        }
    }
}

impl Clone for ConnectionPool {
    fn clone(&self) -> Self {
        ConnectionPool(self.0.clone())
    }
}

impl fmt::Debug for ConnectionPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionPool")
            .field("svc", &self.0.svc)
            .field("inner", &self.0.inner)
            .field("waiters", &self.0.waiters)
            .finish()
    }
}

impl Service<SharedCfg, Connect> for ConnectionPool {
    type Res = Connection;
    type Error = Error<ConnectError>;

    #[inline]
    async fn ready(&self, ctx: Ctx<'_, Self, SharedCfg>) -> Result<(), Self::Error> {
        self.0.svc.ready(ctx.st()).await
    }

    #[inline]
    async fn shutdown(&self, ctx: Ctx<'_, Self, SharedCfg>) {
        self.0.stop.take();
        self.0.inner.borrow_mut().stopped = true;
        self.0.svc.shutdown(ctx.st()).await;
    }

    async fn call(
        &self,
        req: Connect,
        ctx: Ctx<'_, Self, SharedCfg>,
    ) -> Result<Self::Res, Self::Error> {
        log::trace!("{}: Get connection for {:?}", ctx.st().tag(), req.uri);

        let inner = self.0.inner.clone();
        let waiters = self.0.waiters.clone();

        let key = if let Some(authority) = req.uri.authority() {
            authority.clone().into()
        } else {
            return Err(ConnectError::Unresolved.into());
        };

        // acquire connection
        let result = inner.borrow_mut().acquire(&key);
        match result {
            // use existing connection
            Acquire::Acquired(io, created) => {
                log::trace!(
                    "{}: Use existing {:?} connection for {:?}",
                    ctx.st().tag(),
                    io,
                    req.uri
                );
                Ok(Connection::new(
                    io,
                    created,
                    Some(Acquired::new(key, inner)),
                ))
            }
            // open new tcp connection
            Acquire::Available => {
                log::trace!("{}: Connecting to {:?}", ctx.st().tag(), req.uri);
                let uri = req.uri.clone();
                let (tx, rx) = waiters.borrow_mut().pool.channel();
                open_connection(
                    self.0.cfg.clone(),
                    req,
                    key,
                    tx,
                    uri,
                    inner,
                    self.0.svc.bind_state(self.0.cfg.clone()),
                );

                match rx.await {
                    Err(_) => Err(ConnectError::Disconnected(None).into()),
                    Ok(result) => result,
                }
            }
            // pool is full, wait
            Acquire::NotAvailable => {
                log::trace!(
                    "{}: Pool is full, waiting for available connections for {:?}",
                    ctx.st().tag(),
                    req.uri
                );
                let rx = waiters.borrow_mut().wait_for(req);
                match rx.await {
                    Err(_) => Err(ConnectError::Disconnected(None).into()),
                    Ok(result) => result,
                }
            }
        }
    }
}

#[derive(Debug)]
struct Waiters {
    waiters: HashMap<Key, VecDeque<(Connect, Waiter)>>,
    pool: pool::Pool<Result<Connection, Error<ConnectError>>>,
}

impl Waiters {
    /// connection is not available, wait
    fn wait_for(&mut self, connect: Connect) -> WaiterReceiver {
        let (tx, rx) = self.pool.channel();
        let key: Key = connect.uri.authority().unwrap().clone().into();
        self.waiters
            .entry(key)
            .or_default()
            .push_back((connect, tx));
        rx
    }

    /// cleanup dropped waiters
    fn cleanup(&mut self) {
        let mut keys = Vec::new();

        // cleanup waiters
        for (key, waiters) in &mut self.waiters {
            while !waiters.is_empty() {
                let (req, tx) = waiters.front().unwrap();
                // check if waiter is still alive
                if tx.is_canceled() {
                    log::trace!("Waiter for {:?} is gone, remove waiter", req.uri);
                    waiters.pop_front();
                    continue;
                }
                break;
            }

            if waiters.is_empty() {
                keys.push(key.clone());
            }
        }

        for key in keys {
            self.waiters.remove(&key);
        }
    }
}

impl Inner {
    fn acquire(&mut self, key: &Key) -> Acquire {
        // check limits
        if self.cfg.limit > 0 && self.acquired >= self.cfg.limit {
            return Acquire::NotAvailable;
        }

        // check if open connection is available
        // cleanup stale connections at the same time
        if let Some(ref mut connections) = self.available.get_mut(key) {
            let now = now();
            while let Some(conn) = connections.pop_back() {
                // check if it still usable
                if (now - conn.used) > self.cfg.conn_keep_alive
                    || (now - conn.created) > self.cfg.conn_lifetime
                {
                    let ConnectionType::H1(io) = conn.io;
                    spawn(async move {
                        let _ = io.shutdown().await;
                    });
                    continue;
                }

                let io = conn.io;
                match io {
                    ConnectionType::H1(ref s) => {
                        if s.is_closed() {
                            continue;
                        }
                        let is_valid = s.with_read_buf(|buf| {
                            if buf.is_empty() || (buf.len() == 2 && &buf[..] == b"\r\n") {
                                buf.clear();
                                true
                            } else {
                                false
                            }
                        });
                        if !is_valid {
                            continue;
                        }
                    }
                }
                return Acquire::Acquired(io, conn.created);
            }
        }

        if self.connecting.contains(key) {
            Acquire::NotAvailable
        } else {
            Acquire::Available
        }
    }

    fn check_availibility(&mut self) {
        let mut waiters = self.waiters.borrow_mut();
        waiters.cleanup();
        if !waiters.waiters.is_empty() && self.acquired < self.cfg.limit {
            let _ = self.waker.send(());
        }
    }
}

async fn run_connection_pool(
    cfg: SharedCfg,
    svc: PipelineBinding<Connect, IoBoxed, Error<ConnectError>>,
    inner: Rc<RefCell<Inner>>,
    waiters: Rc<RefCell<Waiters>>,
    mut stop: oneshot::Receiver<()>,
) {
    log::trace!("{}: Starting connection pool support task", cfg.tag());

    loop {
        {
            let mut cleanup = false;
            let mut waiters = waiters.borrow_mut();

            // check waiters
            for (key, waiters) in &mut waiters.waiters {
                while let Some((req, tx)) = waiters.front() {
                    // is waiter still alive
                    if tx.is_canceled() {
                        log::trace!("{}: Waiter for {:?} is gone, cleanup", cfg.tag(), req.uri);
                        cleanup = true;
                        waiters.pop_front();
                        continue;
                    }

                    let result = inner.borrow_mut().acquire(key);
                    match result {
                        Acquire::NotAvailable => break,
                        Acquire::Acquired(io, created) => {
                            log::trace!(
                                "{}: Use existing {:?} connection for {:?}, wake up waiter",
                                cfg.tag(),
                                io,
                                req.uri
                            );
                            cleanup = true;
                            let (_, tx) = waiters.pop_front().unwrap();
                            let _ = tx.send(Ok(Connection::new(
                                io,
                                created,
                                Some(Acquired::new(key.clone(), inner.clone())),
                            )));
                        }
                        Acquire::Available => {
                            log::trace!(
                                "{}: Connecting to {:?} and wake up waiter",
                                cfg.tag(),
                                req.uri
                            );
                            cleanup = true;
                            let (connect, tx) = waiters.pop_front().unwrap();
                            let uri = connect.uri.clone();
                            open_connection(
                                cfg.clone(),
                                connect,
                                key.clone(),
                                tx,
                                uri,
                                inner.clone(),
                                svc.clone(),
                            );
                        }
                    }
                }
            }

            if cleanup {
                waiters.cleanup();
            }
        }

        let result = select(
            &mut stop,
            future::poll_fn(|cx| inner.borrow().waker.poll_recv(cx)),
        )
        .await;

        if matches!(result, Either::Left(_)) || inner.borrow().stopped {
            log::trace!("{}: Stopping connection pool support task", cfg.tag());
            break;
        }
    }
}

pin_project_lite::pin_project! {
    struct OpenConnection {
        key: Key,
        #[pin]
        fut: PipelineCall<Connect, IoBoxed, Error<ConnectError>>,
        uri: Uri,
        tx: Option<Waiter>,
        guard: Option<OpenGuard>,
        inner: Rc<RefCell<Inner>>,
    }
}

// cfg and uri are only read on the HTTP/2 path, which is not part of this
// phase; the signature is kept so that path can come back unchanged.
#[allow(unused_variables)]
fn open_connection(
    cfg: SharedCfg,
    connect: Connect,
    key: Key,
    tx: Waiter,
    uri: Uri,
    inner: Rc<RefCell<Inner>>,
    pl: PipelineBinding<Connect, IoBoxed, Error<ConnectError>>,
) {
    let guard = OpenGuard::new(key.clone(), inner.clone());

    spawn(async move {
        // open tcp connection
        match pl.call(connect).await {
            Err(err) => {
                log::trace!(
                    "Failed to open client connection for {:?} with error {:?}",
                    key.authority,
                    err
                );
                let _ = tx.send(Err(err));
            }
            Ok(io) => {
                if inner.borrow().stopped {
                    return;
                }

                log::trace!(
                    "{}: Connection for {:?} is established, init http1 connection",
                    io.tag(),
                    key.authority
                );
                let conn = Connection::new(ConnectionType::H1(io), now(), Some(guard.consume()));
                if let Err(Ok(conn)) = tx.send(Ok(conn)) {
                    // waiter is gone, return connection to pool
                    conn.release(false);
                }
                inner.borrow_mut().check_availibility();
            }
        }
    });
}

struct OpenGuard {
    key: Key,
    inner: Option<Rc<RefCell<Inner>>>,
}

impl OpenGuard {
    fn new(key: Key, inner: Rc<RefCell<Inner>>) -> Self {
        inner.borrow_mut().connecting.insert(key.clone());
        OpenGuard {
            key,
            inner: Some(inner),
        }
    }

    fn consume(mut self) -> Acquired {
        let inner = self.inner.take().unwrap();
        inner.borrow_mut().connecting.remove(&self.key);
        Acquired::new(self.key.clone(), inner)
    }
}

impl Drop for OpenGuard {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            let mut pool = inner.borrow_mut();
            pool.connecting.remove(&self.key);
            pool.check_availibility();
        }
    }
}

pub(super) struct Acquired(Key, Option<Rc<RefCell<Inner>>>);

impl Acquired {
    fn new(key: Key, inner: Rc<RefCell<Inner>>) -> Self {
        inner.borrow_mut().acquired += 1;
        Acquired(key, Some(inner))
    }

    // Used when cloning an h2 connection guard, which returns with HTTP/2.
    #[allow(dead_code)]
    fn clone(&self) -> Self {
        Acquired::new(self.0.clone(), self.1.as_ref().unwrap().clone())
    }

    pub(super) fn release(&mut self, conn: Connection, close: bool) {
        if let Some(inner) = self.1.take() {
            let (io, created, _) = conn.into_inner();
            let mut inner = inner.borrow_mut();
            inner.acquired -= 1;
            if close {
                log::trace!(
                    "{:?}: Releasing and closing connection for {:?}",
                    io.tag(),
                    self.0.authority
                );
                match io {
                    ConnectionType::H1(io) => {
                        spawn(async move {
                            let _ = io.shutdown().await;
                        });
                    }
                }
            } else {
                log::trace!(
                    "{:?}: Releasing connection for {:?}",
                    io.tag(),
                    self.0.authority
                );
                inner
                    .available
                    .entry(self.0.clone())
                    .or_insert_with(VecDeque::new)
                    .push_back(AvailableConnection {
                        io,
                        created,
                        used: now(),
                    });
            }
            inner.check_availibility();
        }
    }
}

impl Drop for Acquired {
    fn drop(&mut self) {
        if let Some(inner) = self.1.take() {
            let mut inner = inner.borrow_mut();
            inner.acquired -= 1;
            inner.check_availibility();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{future::Future, pin::Pin};

    use super::*;
    use geario::io as nio;
    use geario::io::testing::IoTest;
    use geario::service::{Pipeline, boxed, fn_service};
    use geario::util::future::lazy;
    use geario::util::time::{Millis, Seconds, sleep};

    #[geario::test]
    async fn test_basics() {
        let store = Rc::new(RefCell::new(Vec::new()));
        let store2 = store.clone();

        let cfg = SharedCfg::new("C")
            .add(
                ClientConfig::new()
                    .set_keep_alive(Seconds(10))
                    .set_lifetime(Seconds(10))
                    .set_limit(1),
            )
            .build();

        let pool = ConnectionPool::new(
            ConnectorPipeline::new(boxed::service(fn_service(move |req| {
                let (client, server) = IoTest::create();
                store2.borrow_mut().push((req, server));
                Box::pin(
                    async move { Ok(IoBoxed::from(nio::Io::new(client, SharedCfg::default()))) },
                )
            }))),
            cfg.get(),
        );
        let pipe = Pipeline::with(cfg, pool.clone());

        // uri must contain authority
        let req = Connect {
            uri: Uri::try_from("/test").unwrap(),
            addr: None,
        };
        let _err = Error::from(ConnectError::Unresolved);
        assert!(matches!(pipe.call(req).await, Err(_err)));

        // connect one
        let req = Connect {
            uri: Uri::try_from("http://localhost/test").unwrap(),
            addr: None,
        };
        let conn = pipe.call(req.clone()).await.unwrap();
        assert_eq!(store.borrow().len(), 1);
        assert!(format!("{conn:?}").contains("Connection(h1)"));
        assert_eq!(conn.protocol(), geario::io::types::HttpProtocol::Http1);
        assert_eq!(pool.0.inner.borrow().acquired, 1);
        assert!(pool.0.inner.borrow().connecting.is_empty());

        // pool is full, waiting
        let mut fut = std::pin::pin!(pipe.call(req.clone()));
        assert!(lazy(|cx| fut.as_mut().poll(cx)).await.is_pending());
        assert_eq!(pool.0.waiters.borrow().waiters.len(), 1);

        // release connection and push it to next waiter
        conn.release(false);
        assert_eq!(pool.0.inner.borrow().acquired, 0);
        let conn = fut.await.unwrap();
        assert_eq!(store.borrow().len(), 1);
        assert!(pool.0.waiters.borrow().waiters.is_empty());
        drop(conn);

        // close connnection
        let conn = pipe.call(req.clone()).await.unwrap();
        assert_eq!(store.borrow().len(), 2);
        assert_eq!(pool.0.inner.borrow().acquired, 1);
        assert!(pool.0.inner.borrow().connecting.is_empty());
        let mut fut = std::pin::pin!(pipe.call(req.clone()));
        assert!(lazy(|cx| fut.as_mut().poll(cx)).await.is_pending());
        assert_eq!(pool.0.waiters.borrow().waiters.len(), 1);

        // release and close
        conn.release(true);
        assert_eq!(pool.0.inner.borrow().acquired, 0);
        assert!(pool.0.inner.borrow().connecting.is_empty());

        let conn = fut.await.unwrap();
        assert_eq!(store.borrow().len(), 3);
        assert!(pool.0.waiters.borrow().waiters.is_empty());
        assert!(pool.0.inner.borrow().connecting.is_empty());
        assert_eq!(pool.0.inner.borrow().acquired, 1);

        // drop waiter, no interest in connection
        let mut fut = Box::pin(pipe.call(req.clone()));
        assert!(lazy(|cx| Pin::new(&mut fut).poll(cx)).await.is_pending());
        drop(fut);
        sleep(Millis(50)).await;
        pool.0.inner.borrow_mut().check_availibility();
        assert!(pool.0.waiters.borrow().waiters.is_empty());

        // different uri
        let req = Connect {
            uri: Uri::try_from("http://localhost2/test").unwrap(),
            addr: None,
        };
        let mut fut = std::pin::pin!(pipe.call(req.clone()));
        assert!(lazy(|cx| fut.as_mut().poll(cx)).await.is_pending());
        assert_eq!(pool.0.waiters.borrow().waiters.len(), 1);
        conn.release(false);
        assert_eq!(pool.0.inner.borrow().acquired, 0);
        assert_eq!(pool.0.inner.borrow().available.len(), 1);

        let conn = fut.await.unwrap();
        assert_eq!(store.borrow().len(), 4);
        assert!(pool.0.waiters.borrow().waiters.is_empty());
        assert!(pool.0.inner.borrow().connecting.is_empty());
        assert_eq!(pool.0.inner.borrow().acquired, 1);
        conn.release(false);
        assert_eq!(pool.0.inner.borrow().acquired, 0);
        assert_eq!(pool.0.inner.borrow().available.len(), 2);

        assert!(lazy(|cx| pipe.poll_ready(cx)).await.is_ready());
        assert!(lazy(|cx| pipe.poll_shutdown(cx)).await.is_ready());
    }
}

//! hyper on a thread-per-core runtime, to separate two explanations.
//!
//! The multi-threaded build loses to geario by 72% at four connections and
//! only 11% at sixteen. That shape fits a scheduling cost that gets amortised
//! as concurrency rises, rather than a slower protocol implementation.
//!
//! This build runs one current-thread runtime per worker, each with its own
//! listener behind SO_REUSEPORT.
//!
//! That is NOT the shape geario has. geario runs a single accept loop and
//! hands connections to its workers; it sets SO_REUSEADDR, never
//! SO_REUSEPORT. So this build changes the runtime and the connection
//! distribution at the same time, and a difference against it cannot be
//! attributed to either one alone.
//!
//! Run it with BENCH_WORKERS=1 to remove distribution from the comparison.
use std::convert::Infallible;
use std::net::SocketAddr;

use bytes::Bytes;
use http_body_util::Full;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;

const BODY: &[u8] = b"hello from the benchmark";

async fn handle(_req: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    Ok(Response::builder()
        .header("content-type", "text/plain")
        .body(Full::new(Bytes::from_static(BODY)))
        .unwrap())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = std::env::var("BENCH_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:18091".into())
        .parse()?;

    if let Ok(secs) = std::env::var("BENCH_SECONDS") {
        if let Ok(secs) = secs.parse::<u64>() {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(secs));
                std::process::exit(0);
            });
        }
    }

    let count: usize = std::env::var("BENCH_WORKERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()));
    let mut workers = Vec::new();

    for _ in 0..count {
        // SO_REUSEPORT so each worker owns a listener. With one worker this
        // is just an ordinary listener.
        let sock = socket2::Socket::new(
            socket2::Domain::for_address(addr),
            socket2::Type::STREAM,
            None,
        )?;
        sock.set_reuse_address(true)?;
        sock.set_reuse_port(true)?;
        sock.bind(&addr.into())?;
        sock.listen(2048)?;
        sock.set_nonblocking(true)?;
        let std_lst: std::net::TcpListener = sock.into();

        workers.push(std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");
            // spawn_local needs a LocalSet to run in; without one the tasks
            // are created and never polled, which looks like a server that
            // accepts and then answers nothing.
            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, async move {
                let listener = tokio::net::TcpListener::from_std(std_lst).expect("listener");
                loop {
                    let Ok((stream, _)) = listener.accept().await else { continue };
                    stream.set_nodelay(true).ok();
                    tokio::task::spawn_local(async move {
                        let _ = http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), service_fn(handle))
                            .await;
                    });
                }
            })
        }));
    }

    for w in workers {
        let _ = w.join();
    }
    Ok(())
}

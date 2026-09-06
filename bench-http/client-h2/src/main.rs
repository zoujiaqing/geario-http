//! HTTP/2 load generator.
//!
//! One tokio runtime drives both arms of a comparison, so the generator is
//! never part of what is being compared. Concurrency here is streams, not
//! connections: that is the thing h2 exists to do, and a server that
//! serialises them would otherwise look the same as one that does not.
//!
//! Responses are checked, not counted. A server answering with the wrong
//! body is not doing the same work.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};

fn expect_len() -> usize {
    std::env::var("BENCH_BODY_SIZE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24)
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args = std::env::args().skip(1);
    let addr = args.next().unwrap_or_else(|| "127.0.0.1:18093".into());
    let conns: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(4);
    let secs: u64 = args.next().and_then(|v| v.parse().ok()).unwrap_or(5);
    let streams: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(16);
    let want = expect_len();

    let stop = Arc::new(AtomicBool::new(false));
    let mismatches = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));
    let latencies: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));

    let mut tasks = Vec::new();
    for _ in 0..conns {
        let stream = match tokio::net::TcpStream::connect(&addr).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("connect: {e}");
                std::process::exit(1);
            }
        };
        stream.set_nodelay(true).ok();
        let (sender, conn) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
            .timer(TokioTimer::new())
            .handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .expect("h2 handshake");
        tokio::spawn(async move {
            let _ = conn.await;
        });

        for _ in 0..streams {
            let mut sender = sender.clone();
            let (stop, mismatches, errors, latencies) = (
                stop.clone(),
                mismatches.clone(),
                errors.clone(),
                latencies.clone(),
            );
            let uri = format!("http://{addr}/bench");
            tasks.push(tokio::spawn(async move {
                let mut mine = Vec::new();
                while !stop.load(Ordering::Relaxed) {
                    let started = Instant::now();
                    let req = Request::builder()
                        .uri(&uri)
                        .body(Full::new(Bytes::new()))
                        .unwrap();
                    match sender.send_request(req).await {
                        Ok(res) => {
                            if res.status() != 200 {
                                mismatches.fetch_add(1, Ordering::Relaxed);
                                continue;
                            }
                            match res.into_body().collect().await {
                                Ok(body) => {
                                    if body.to_bytes().len() == want {
                                        mine.push(started.elapsed().as_micros() as u64);
                                    } else {
                                        mismatches.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                Err(_) => {
                                    errors.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                        Err(_) => {
                            errors.fetch_add(1, Ordering::Relaxed);
                            break;
                        }
                    }
                }
                latencies.lock().unwrap().extend(mine);
            }));
        }
    }

    let started = Instant::now();
    tokio::time::sleep(Duration::from_secs(secs)).await;
    stop.store(true, Ordering::Relaxed);
    for t in tasks {
        let _ = t.await;
    }
    let elapsed = started.elapsed().as_secs_f64();

    let mut lat = std::mem::take(&mut *latencies.lock().unwrap());
    lat.sort_unstable();
    let pick = |q: f64| -> f64 {
        if lat.is_empty() {
            return 0.0;
        }
        lat[((lat.len() as f64 * q) as usize).min(lat.len() - 1)] as f64
    };

    println!("target      {addr}");
    println!("conns       {conns}");
    println!("streams     {streams}");
    println!("resp_bytes  {want}");
    println!("duration    {elapsed:.2} s");
    println!("requests    {}", lat.len());
    println!("qps         {:.0}", lat.len() as f64 / elapsed);
    println!("p50         {:.1} us", pick(0.50));
    println!("p99         {:.1} us", pick(0.99));
    println!("mismatches  {}", mismatches.load(Ordering::Relaxed));
    println!("errors      {}", errors.load(Ordering::Relaxed));
}

//! HTTP/1.1 keep-alive load generator.
//!
//! Blocking sockets on plain threads, so nothing about the measurement depends
//! on an async runtime that might favour one server over the other.
//!
//! The response is checked, not just counted: a server that answers with the
//! wrong body or a different status is not doing the same work, and comparing
//! its throughput would be meaningless.
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const EXPECT_BODY: &[u8] = b"hello from the benchmark";

struct Outcome {
    latencies: Vec<u64>,
    mismatches: u64,
    errors: u64,
}

fn read_response(sock: &mut TcpStream, buf: &mut Vec<u8>) -> Result<(), &'static str> {
    buf.clear();
    let mut tmp = [0u8; 4096];
    loop {
        // Headers first: find the blank line, then honour content-length.
        if let Some(pos) = find_headers_end(buf) {
            let head = &buf[..pos];
            let len = content_length(head).ok_or("no content-length")?;
            let body_start = pos + 4;
            if buf.len() >= body_start + len {
                if !head.starts_with(b"HTTP/1.1 200") {
                    return Err("status");
                }
                if &buf[body_start..body_start + len] != EXPECT_BODY {
                    return Err("body");
                }
                return Ok(());
            }
        }
        let n = sock.read(&mut tmp).map_err(|_| "read")?;
        if n == 0 {
            return Err("eof");
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

fn find_headers_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn content_length(head: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(head).ok()?;
    text.lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse().ok())?
        })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let addr = args.next().unwrap_or_else(|| "127.0.0.1:18090".into());
    let conns: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(8);
    let secs: u64 = args.next().and_then(|v| v.parse().ok()).unwrap_or(10);

    let host = addr.clone();
    let request = format!("GET /bench HTTP/1.1\r\nHost: {host}\r\n\r\n").into_bytes();

    let stop = Arc::new(AtomicBool::new(false));
    let total = Arc::new(AtomicU64::new(0));
    let mut handles = Vec::new();

    for _ in 0..conns {
        let addr = addr.clone();
        let request = request.clone();
        let stop = stop.clone();
        let total = total.clone();
        handles.push(std::thread::spawn(move || {
            let mut sock = match TcpStream::connect(&addr) {
                Ok(s) => s,
                Err(_) => {
                    return Outcome { latencies: Vec::new(), mismatches: 0, errors: 1 }
                }
            };
            sock.set_nodelay(true).ok();
            // Without these a server that stops answering leaves the thread
            // blocked in read forever, and the run never finishes.
            let t = Duration::from_secs(5);
            sock.set_read_timeout(Some(t)).ok();
            sock.set_write_timeout(Some(t)).ok();
            let mut buf = Vec::with_capacity(8192);
            let mut out = Outcome {
                latencies: Vec::with_capacity(1 << 16),
                mismatches: 0,
                errors: 0,
            };

            while !stop.load(Ordering::Relaxed) {
                let t = Instant::now();
                if sock.write_all(&request).is_err() {
                    out.errors += 1;
                    break;
                }
                match read_response(&mut sock, &mut buf) {
                    Ok(()) => {
                        out.latencies.push(t.elapsed().as_nanos() as u64);
                        total.fetch_add(1, Ordering::Relaxed);
                    }
                    Err("status") | Err("body") => {
                        out.mismatches += 1;
                        break;
                    }
                    Err(_) => {
                        out.errors += 1;
                        break;
                    }
                }
            }
            out
        }));
    }

    let start = Instant::now();
    std::thread::sleep(Duration::from_secs(secs));
    stop.store(true, Ordering::Relaxed);

    let mut all = Vec::new();
    let (mut mismatches, mut errors) = (0u64, 0u64);
    for h in handles {
        let o = h.join().unwrap();
        all.extend(o.latencies);
        mismatches += o.mismatches;
        errors += o.errors;
    }
    let elapsed = start.elapsed().as_secs_f64();
    all.sort_unstable();

    let pct = |p: f64| -> f64 {
        if all.is_empty() {
            return 0.0;
        }
        all[((all.len() as f64 * p) as usize).min(all.len() - 1)] as f64 / 1000.0
    };

    let count = total.load(Ordering::Relaxed);
    println!("target      {addr}");
    println!("conns       {conns}");
    println!("duration    {elapsed:.2} s");
    println!("requests    {count}");
    println!("qps         {:.0}", count as f64 / elapsed);
    println!("p50         {:.1} us", pct(0.50));
    println!("p99         {:.1} us", pct(0.99));
    println!("mismatches  {mismatches}");
    println!("errors      {errors}");
}

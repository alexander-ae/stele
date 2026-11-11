//! Closed-loop load generator for the benchmarks in `docs/benchmarks.md`.
//! One TCP connection per request, N threads, prints throughput and
//! percentiles. It is a development tool, not part of the mailbox.
//!
//! cargo run --release --example loadgen -- \
//!   HOST:PORT '/messages?unread=1&limit=200' 5 8 GET '' 'Authorization: Bearer TOKEN'

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let addr = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:3900".into());
    let path = args.get(2).cloned().unwrap_or_else(|| "/".into());
    let seconds: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let threads: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(8);
    let method = args.get(5).cloned().unwrap_or_else(|| "GET".into());
    let body = args.get(6).cloned().unwrap_or_default();
    let header = args.get(7).cloned().unwrap_or_default();

    let stop = Arc::new(AtomicBool::new(false));
    let all: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
    let sequence = Arc::new(AtomicUsize::new(0));
    let mut workers = Vec::new();

    for _ in 0..threads {
        let (addr, path, method, body, header) = (
            addr.clone(),
            path.clone(),
            method.clone(),
            body.clone(),
            header.clone(),
        );
        let (stop, all, sequence) = (stop.clone(), all.clone(), sequence.clone());
        workers.push(std::thread::spawn(move || {
            let mut local = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                // A `%SEQ%` in the body or the header is a per-request value,
                // which is what a unique client_id needs.
                let n = sequence.fetch_add(1, Ordering::Relaxed);
                let body = body.replace("%SEQ%", &n.to_string());
                let header = header.replace("%SEQ%", &n.to_string());
                let started = Instant::now();
                let Ok(mut stream) = TcpStream::connect(&addr) else {
                    continue;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut request =
                    format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
                if !header.is_empty() {
                    request.push_str(&header);
                    request.push_str("\r\n");
                }
                if !body.is_empty() {
                    request.push_str(&format!("Content-Length: {}\r\n", body.len()));
                }
                request.push_str("\r\n");
                request.push_str(&body);
                if stream.write_all(request.as_bytes()).is_ok() {
                    let mut buf = [0u8; 8192];
                    while let Ok(n) = stream.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                    }
                    local.push(started.elapsed().as_nanos() as u64);
                }
            }
            all.lock().unwrap().extend(local);
        }));
    }

    std::thread::sleep(Duration::from_secs_f64(seconds));
    stop.store(true, Ordering::Relaxed);
    for worker in workers {
        let _ = worker.join();
    }

    let mut times = all.lock().unwrap().clone();
    times.sort_unstable();
    let count = times.len();
    let pct = |p: f64| {
        if count == 0 {
            return 0.0;
        }
        let i = ((count - 1) as f64 * p) as usize;
        times[i] as f64 / 1_000_000.0
    };
    println!(
        "n={} {:.0} req/s  p50={:.3}ms p95={:.3}ms p99={:.3}ms",
        count,
        count as f64 / seconds,
        pct(0.50),
        pct(0.95),
        pct(0.99)
    );
}

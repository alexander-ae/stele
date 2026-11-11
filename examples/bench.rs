//! A benchmark that uses the same client code as the CLI.
//!
//! cargo run --release --example bench -- lat http://127.0.0.1:3900 TOKEN 1000
//! cargo run --release --example bench -- seed http://127.0.0.1:3900 TOKEN 200
//!
//! `lat` sends and acknowledges one message per iteration, so the pending
//! queue never reaches its cap, and prints the percentiles of each call.
//! `seed` leaves the messages unread, to measure a full inbox read with the
//! separate load generator.

use std::time::Instant;

use anyhow::{bail, Context, Result};
use stele::api::{MeResponse, SendRequest, SendResponse};
use stele::http::{call, Endpoint};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        bail!("usage: bench <lat|seed> <url> <token> <n>");
    }
    let mode = args[1].as_str();
    let endpoint = Endpoint::parse(&args[2])?;
    let token = args[3].clone();
    let count: usize = args[4].parse().context("n has to be a number")?;

    let (status, raw) = call(&endpoint, "GET", "/me", Some(&token), None)?;
    if status != 200 {
        bail!("/me answered {status}: {}", String::from_utf8_lossy(&raw));
    }
    let me: MeResponse = serde_json::from_slice(&raw)?;

    let mut send_times = Vec::with_capacity(count);
    let mut ack_times = Vec::with_capacity(count);
    let started = Instant::now();
    let run = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    for i in 0..count {
        let request = SendRequest {
            client_id: format!("bench-{run}-{i}"),
            to: me.mailbox.clone(),
            project: "bench".into(),
            body: "The benchmark body is a short sentence of the kind a note has.".into(),
        };
        let payload = serde_json::to_vec(&request)?;
        let t = Instant::now();
        let (status, raw) = call(&endpoint, "POST", "/messages", Some(&token), Some(&payload))?;
        send_times.push(t.elapsed());
        if status != 201 {
            bail!("POST answered {status}: {}", String::from_utf8_lossy(&raw));
        }
        let response: SendResponse = serde_json::from_slice(&raw)?;
        if mode == "seed" {
            continue;
        }
        let t = Instant::now();
        let (status, raw) = call(
            &endpoint,
            "POST",
            &format!("/messages/{}/ack", response.id),
            Some(&token),
            Some(b""),
        )?;
        ack_times.push(t.elapsed());
        if status != 204 {
            bail!("ACK answered {status}: {}", String::from_utf8_lossy(&raw));
        }
    }
    let elapsed = started.elapsed();

    println!(
        "{} x {} in {:.2}s ({:.0} msg/s)",
        mode,
        count,
        elapsed.as_secs_f64(),
        count as f64 / elapsed.as_secs_f64()
    );
    report("send", &mut send_times);
    if mode == "lat" {
        report("ack", &mut ack_times);
    }
    Ok(())
}

fn report(label: &str, times: &mut [std::time::Duration]) {
    if times.is_empty() {
        return;
    }
    times.sort_unstable();
    let pct = |p: f64| {
        let i = ((times.len() - 1) as f64 * p) as usize;
        times[i].as_secs_f64() * 1_000.0
    };
    println!(
        "{label:>5}: p50={:.3}ms p95={:.3}ms p99={:.3}ms max={:.3}ms",
        pct(0.50),
        pct(0.95),
        pct(0.99),
        times[times.len() - 1].as_secs_f64() * 1_000.0
    );
}

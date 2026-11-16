# Benchmarks

Measured on 2026-11-11, stele 0.1.0, on the Dell running Ubuntu 26.04
(kernel 7.0.0-42, Intel i5-6300HQ, 4 cores, 15 GB), over loopback. Release
profile: `opt-level = 3`, `lto = "fat"`, `codegen-units = 1`, `strip = true`.
Every number below is a real run, not an estimate; where a single run is not
representative, it says so.

## Reproduce

```bash
cargo build --release

# 1000 sequential send + ack pairs, against a server on 127.0.0.1:3900
cargo run --release --example bench -- lat http://127.0.0.1:3900 "$TOKEN" 1000

# fill an inbox (the cap is 200), then read it with 8 threads for 5 s
cargo run --release --example bench -- seed http://127.0.0.1:3900 "$TOKEN" 200
cargo run --release --example loadgen -- \
  127.0.0.1:3900 '/messages?unread=1&limit=200' 5 8 GET '' "Authorization: Bearer $TOKEN"
```

`bench` uses the same HTTP code as the CLI. `loadgen` opens one connection per
request and holds none open.

## Footprint

| | measured |
|---|---|
| binary, stripped | 2,859,760 bytes (2.86 MB) |
| RSS, `serve` idle | 5,716 KB, one thread |
| RSS, `serve` after the read benchmark | 5,804 KB |
| CPU, `serve` idle | 0 ticks over 10 s (`utime+stime` from `/proc`) |
| RSS, `stele pull` with an empty inbox | 3.2 MB, under 10 ms |
| RSS, `stele send` including the CLI | 3.3 MB, under 10 ms |

The proposal's targets were 2.5–3 MB, ~6 MB of RSS, 0% idle CPU and under
1 ms. Four of the five hold. The database target did not; see below.

## Latency

Sequential, one call at a time, 1000 send + ack pairs:

| | p50 | p95 | p99 | max |
|---|---|---|---|---|
| `POST /messages` | 0.238 ms | 0.370 ms | 0.819 ms | 1.249 ms |
| `POST /messages/{id}/ack` | 0.153 ms | 0.254 ms | 0.566 ms | 0.674 ms |

That is 2,214 send+ack pairs per second from one thread. p50 moves by about
±0.03 ms between runs; p99 moved between 0.55 and 0.82 ms across the runs
taken, and one request in a thousand hit 1.25 ms. The under-a-millisecond
target holds at p99, not at the maximum.

With 200 messages already pending, `POST` was p50 0.288 ms, p99 0.385 ms
(3,384/s for the seed loop), so the pending count does not change the shape.

## Reading a full inbox

200 unread messages, each body about 200 bytes (a ~40 KB JSON response),
8 threads for 5 seconds:

| | |
|---|---|
| requests | 11,647 in 5 s |
| throughput | 2,329 req/s |
| p50 | 3.35 ms |
| p95 | 3.64 ms |
| p99 | 3.72 ms |

The server is single-threaded, so those 3 ms are queueing behind eight
clients, not the cost of one read. Per request the server needs about 0.43 ms
of wall time. For a mailbox that receives tens of messages a day this is
irrelevant; the number is here to show that a blocking accept loop does not
fall over.

## Database growth

10,000 rows with ~200-byte bodies, after `pragma wal_checkpoint(truncate)`:

```
3,682,304 bytes  (3.68 MB, ~368 bytes per message including two indexes)
```

The proposal said ~200 KB. That target was wrong: it did not count the row,
the body or the indexes. Eighteen times the estimate is still a file that fits
in a phone photo, and the caps (32 KB body, 200 pending per recipient, 7-day
TTL) bound it; the number in `design.md` is now the measured one.

## The `mem` child

`stele pull` itself is small, but it hands each body to `mem`, and on a
database with the vocabulary pack installed `mem save` reached **66 MB of
RSS** in one of these runs (the pack is mmapped and the new memory is
embedded). That is anathem's cost, not stele's, and it is the same cost as
running `mem save` by hand. At a handful of messages a day it is a short
spike; a mailbox that delivered hundreds per hour would want to batch it.

## Dependency comparison

Each row is a hello-world built with the same profile (`panic = "abort"` in
these scratch builds, so their baseline is a little smaller than stele's;
compare the deltas, not the absolute sizes).

| choice | bytes | delta |
|---|---|---|
| server with `tiny_http` 0.12 | 519,576 | — |
| server with `httparse` 1.10 + 100 lines | 308,712 | **−210,864 (−41%)** |
| client with `ureq` 3.5.1 (no TLS) + `serde_json` | 720,288 | — |
| client with `httparse` + `serde_json` | 376,896 | **−343,392 (−48%)** |
| CLI with `clap` 4 | 584,496 | — |
| CLI with `lexopt` 0.3 | 304,512 | **−279,984 (−48%)** |
| timestamp with `chrono` 0.4 | 328,016 | — |
| timestamp with 25 lines of `std` | 288,464 | **−39,552** |
| `rusqlite` with system SQLite | — | does not link here |
| `rusqlite` bundled | 2,358,824 | +2.07 MB over a 288 KB baseline |

The first three rows are the same hello-world at rest, so the server row can
also be compared on idle RSS and on a 5-second load run (8 threads):

| hello-world server | binary | idle RSS | p99 |
|---|---|---|---|
| `tiny_http` | 519,576 | 3,236 KB | 0.578 ms |
| `httparse` | 308,712 | 2,156 KB | 0.373 ms |

The system-SQLite row is not missing a number: `libsqlite3.so` is not
installed on this machine and installing it needs sudo, so the link fails.
Compiling SQLite in is what makes the binary a single file with nothing to
match on the host — and it is what anathem does. That is 2 MB of the 2.86 MB.

## For scale: anathem

| | |
|---|---|
| `mem` binary, stripped | 8.0 MB |
| `mem serve`, idle | 9.2–14.2 MB of RSS **per agent session** |
| six sessions, observed | ~60 MB |
| `mem recent` | 9.4 MB, under 10 ms |

One `stele serve` for all of them, at 5.7 MB, is the cheaper shape. This is
the argument for a separate daemon rather than another mode inside `mem`.

## What this does not measure

- **The network.** Loopback has no latency. A direct link on a private network
  adds a fraction of a millisecond; a relayed VPN adds tens. That dominates
  everything above, and no code in stele changes it.
- **Concurrent writers.** The server serialises requests, so the write path is
  never concurrent by construction.
- **Soak and disk-full behaviour.** The TTL and caps are tested for logic, not
  by running for a week or filling a filesystem.
- **A 10k-message inbox.** It cannot exist: the pending cap is 200. The 10k
  row was inserted directly into SQLite to measure file growth, not delivery.

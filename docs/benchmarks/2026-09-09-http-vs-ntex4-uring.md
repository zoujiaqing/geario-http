# HTTP/1.1: geario-http against ntex 4.0 on io_uring

- Date: 2026-09-09
- Companion to the neon+polling comparison. Same versions, same host, io_uring
  turned on.

## Setup (evidence)

- io_uring enabled on the Rocky host: `sysctl -w kernel.io_uring_disabled=0`.
- Both servers built with the explicit `uring` feature (`= neon-uring`) and
  confirmed at run time:
  - `server-geario driver=neon-uring workers=4` / `io_uring_enter` present
  - `server-ntex4 ntex@8af6d027 driver=neon-uring workers=4` / `io_uring_enter` present
- Same host (Rocky 9, 4 cores), `BENCH_WORKERS=4`, raw HttpService, same body.
- ntex pinned to SHA `8af6d0271c1f4a3226c79a92740f7e9bf31347cd` (crate 4.0.0-beta.9).

## Results

`ab2.sh`, twelve paired rounds at the knee (4 connections). Positive means
geario faster.

| body | geario vs ntex 4.0 (both neon+io_uring) |
| --- | --- |
| 24 B | +3.57% [+0.66%, +6.65%] |
| 1 KB | -0.47% [-4.36%, +2.52%] |
| 16 KB | -0.47% [-2.92%, +2.06%] |
| POST 1 KB | +1.21% [-0.82%, +3.28%] |

Even. 24 B is a small edge whose interval just clears zero; 1 KB, 16 KB and
POST all straddle zero.

## Reading

This is the same story io_uring told for echo, now confirmed for HTTP:
geario's io_uring driver is a near-verbatim port of ntex's, so on io_uring
the two stacks are the same speed. geario's lead over ntex is on the polling
driver (echo +10-19%, HTTP +6-11%), where the forked poller spends fewer
syscalls per wakeup; on io_uring there is no such divergence and the two tie.

To lead on io_uring the io_uring driver itself would have to diverge from
ntex's (multishot recv, provided buffer rings, or similar), which has not
been done.

## Aside

At 16 KB both servers drop to ~113k qps on io_uring versus ~168-178k on
polling on this host. Both drop together, so it is a shared characteristic
of the ported io_uring driver, not a geario-specific regression; worth a
look later but out of scope for this comparison.

## What is not covered

One host, four workers at the knee, HTTP/1.1. p99, CPU per request, memory,
and worker-count scaling not recorded here.

# HTTP/1.1: geario-http against ntex 4.0 on the same runtime

- Date: 2026-09-09
- This is the architecturally-fair comparison: ntex on **its own neon
  runtime** (not tokio), same version line geario was forked from.

## Versions (pinned)

| | source | commit / version |
| --- | --- | --- |
| ntex | local checkout, main | SHA `8af6d0271c1f4a3226c79a92740f7e9bf31347cd`, `git describe` = `ntex-v4.0.0-beta.2` + main, crate version 4.0.0-beta.9 |
| geario | this repo's sibling | SHA `4626539cc7d75ae1cf2b04332775a121c604c7e7` |
| geario-http | this repo | SHA `d6200836578d220ae942a1a706d670ad1ee1a6cc` |

## Build (evidence)

- Toolchain: rustc 1.98.1, built natively on the Rocky host (cross-compile
  from macOS is blocked: the homebrew toolchain's glibc lacks `getrandom`,
  which ntex pulls through std).
- ntex server: `bench-http/server-ntex4`, in this repo. ntex is a
  git dependency pinned to the SHA above, so it builds without a local
  checkout wherever github is reachable: `server-ntex4/build.sh polling`
  (`= --features polling` with `RUSTFLAGS=--cap-lints=allow`; cap-lints
  relaxes ntex's `warnings = deny` only, no codegen change). The Rocky host
  has no github egress, so its binary was built from an rsync of the same
  pinned checkout; the SHA is the anchor and is identical.
- geario server: `server-geario` built `--features polling` = `geario/neon-polling`.
- Both raw `HttpService` (h1), no web/routing layer, same fixed-size
  `application/json` body, request body drained on POST.

## Runtime (evidence)

Both servers print their configuration at startup:

    server-geario driver=neon-polling workers=4 body=1024
    server-ntex4  ntex@8af6d027... driver=neon-polling workers=4 body=1024

Both built with the explicit `polling` feature (`= neon-polling`), confirmed
by that line and by strace (`epoll_pwait` present, no `io_uring_enter`). Same
host (Rocky 9, 4 cores), `BENCH_WORKERS=4` for both, same client, same
release profile (opt-level 3, lto, codegen-units 1, panic=abort; debug and
frame-pointers match too and do not affect codegen).

The four-point table below was measured with the ntex server built
`neon-default` (which falls back to polling on this host, io_uring being
off) -- both ran polling, confirmed by strace. Rebuilding the ntex server
with the explicit `polling` feature and re-running the 1 KB point gives
+9.17% [+6.68%, +11.64%], matching the +9.29% below, so the four-point
result stands.

## Results

`ab2.sh`, twelve paired rounds at the knee (4 connections). Positive means
geario faster.

| body | geario vs ntex 4.0 (both neon+polling) |
| --- | --- |
| 24 B | **+11.18%** [+7.75%, +15.25%] |
| 1 KB | **+9.29%** [+6.58%, +12.49%] |
| 16 KB | **+5.73%** [+3.69%, +7.81%] |
| POST 1 KB | **+10.02%** [+8.08%, +11.82%] |

geario-http is 6-11% faster than ntex 4.0 across all four, every interval
clear of zero, no bad rounds.

What this is and is not: both are neon-family thread-per-core stacks on a
polling backend, but geario's runtime and poller are the modified fork, so
the two are not byte-identical below the HTTP layer. The honest claim is a
**throughput difference between two complete HTTP/1.1 stacks in this
configuration**, observed and reproducible, not a proven attribution to the
poller. The echo syscall attribution does not transfer automatically; to
attribute the HTTP lead to the poller specifically needs a toggle A/B under
HTTP load (geario built with the poller change reverted vs not, same
server), which has not been run. It is consistent with the echo finding,
which is a weaker statement than caused by it.

## What is not covered

Polling only (io_uring not measured here), one host, four workers at the
knee, HTTP/1.1. p99, CPU per request and memory not recorded in this pass;
pipelining and HTTP/2 against ntex not yet run.

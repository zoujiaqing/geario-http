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
- ntex server: `cargo build --release --features polling` with
  `RUSTFLAGS=--cap-lints=allow`. `polling` = `ntex/neon-polling`. cap-lints
  only relaxes ntex's `warnings = deny`; it does not change codegen. The
  ntex sub-crates are `[patch]`-redirected to the local checkout so one
  consistent set builds (without the patch they resolve to crates.io and the
  versions skew).
- geario server: `server-geario` built `--features polling` = `geario/neon-polling`.
- Both raw `HttpService` (h1), no web/routing layer, same fixed-size
  `application/json` body, request body drained on POST.

## Runtime (evidence)

Both confirmed by strace at run time: `epoll_pwait` present, no
`io_uring_enter`. So both are the neon runtime on the polling driver. Same
host (Rocky 9, 4 cores), `BENCH_WORKERS=4` for both, same client.

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
clear of zero, no bad rounds. Because the runtime and driver are identical,
this is the framework/IO layer, not a runtime difference. It is consistent
with the echo result (same poller-fork advantage: fewer syscalls per
wakeup) now shown to carry into the HTTP hot path.

## What is not covered

Polling only (io_uring not measured here), one host, four workers at the
knee, HTTP/1.1. p99, CPU per request and memory not recorded in this pass;
pipelining and HTTP/2 against ntex not yet run.

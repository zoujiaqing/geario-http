> SUPERSEDED. This compared against ntex 2.18 on tokio, which is not ntex's
> own runtime and not the 4.0 line geario was forked from. The real,
> same-runtime comparison is 2026-09-09-http-vs-ntex4-neon.md (ntex 4.0 on
> neon+polling): geario +6-11%. The 2.x/tokio numbers below are kept only as
> a secondary observation against the installable stable release.

# HTTP/1.1: geario-http against ntex

- Date: 2026-09-09
- Host: Rocky 9 (kernel 5.14, 4 cores), io_uring off, all four workers.
- Servers: `server-geario` (geario-http's native HTTP/1 stack) and
  `server-ntex-http` (`ntex` 2.18.0). Same handler: a fixed-size
  `application/json` body, request body drained on POST.
- Method: `ab2.sh`, fourteen paired rounds at the knee (4 connections),
  bootstrap 95% CI. Positive means geario faster.

## Results

| body | geario vs ntex |
| --- | --- |
| 24 B | +2.58% [-5.70%, +9.88%] |
| 1 KB | **+8.39%** [+6.41%, +10.36%] |
| 16 KB | **+9.79%** [+8.01%, +11.73%] |
| POST 1 KB | **+7.56%** [+4.68%, +10.59%] |

geario-http is 8-10% ahead on 1 KB, 16 KB and POST, with intervals clear of
zero and no bad rounds. At 24 bytes the two are a tie (interval spans zero).

## The runtime caveat, stated plainly

This is **not** a same-runtime comparison, and the win cannot be split
between framework and runtime here. geario-http runs on geario's neon
runtime with the polling driver; `ntex` 2.18 runs on **tokio**. ntex's own
thread-per-core "neon" runtime exists only in the unreleased 4.0 beta line,
whose HTTP layer does not compile (the umbrella crate has a generic-arity
error in `http/service.rs` and missing types in `web/`), so a same-runtime
ntex HTTP server could not be built. Stable ntex has no neon runtime.

So this measures what a user actually gets choosing between geario-http and
the ntex they can install: geario-http, on its own runtime, serves HTTP/1.1
8-10% faster here. It does not prove the HTTP framework alone is faster; the
clean same-runtime geario-vs-ntex result is the echo/IO comparison, where
geario's polling driver beats ntex's by 10-19% at the same operating point.

## What is not covered

Single host, polling only (io_uring is off on this box, and tokio is epoll
regardless), four workers at the knee. Small-response (24 B) is a tie. No
keep-alive-pipelining or HTTP/2 comparison against ntex yet. p99 and CPU per
request were not recorded in this pass.

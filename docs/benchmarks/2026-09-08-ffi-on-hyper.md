# The FFI server on hyper

- Date: 2026-09-08
- Host: Fedora 44, kernel 7.0.12, 2 cores, io_uring available
- Method: `ab2.sh`, twelve paired rounds at the knee (4 connections, 16 KB
  responses), bootstrap 95% CI. The server is the C `bench_server.c` linked
  against `libgeario_http_ffi.a`, so it goes through the whole ABI: the
  request reaches a C callback and the response comes back through a
  responder ticket.

## The question

The FFI used to run geario-http's native HTTP/1 stack, which has no HTTP/2.
It now runs hyper over geario, which gains HTTP/2 and the rest of hyper4k's
capability set. What does that cost at the HTTP/1 path it was already good
at?

## Numbers

| comparison | result |
| --- | --- |
| FFI (hyper) vs the native stack it replaced | **-5.31%** [-7.35%, -3.24%] |
| FFI (hyper) vs hyper on tokio | **+3.63%** [+2.55%, +4.62%] |

Zero mismatches or errors in either run.

## Reading

Moving the FFI off the native stack costs about five percent at 16 KB on
this host. The C ABI now reports, and delivers, the same capability *set*
hyper4k does: server HTTP/1.1 and h2c on one port; client HTTP/1.1, HTTP/2
by ALPN over TLS, a custom CA bundle, a proxy with CONNECT for TLS targets,
cancellation and streaming.

## What the second row is, and is not

`server-hyper-st` is a single-threaded Rust hyper-on-tokio benchmark
server. It is NOT hyper4k: there is no Kotlin/Native, no cinterop, no GC,
and no Neton scheduling chain. So +3.63% supports one thing only -- that
hyper on geario's IO is not behind hyper on tokio's at this operating
point, which was the original question about the IO layer. It does **not**
support "replacing hyper4k is a net gain"; that comparison has not been
run.

## Why this is not a drop-in replacement for hyper4k

Aligning the capability bits and error numbers was real, but it does not
make the ABI interchangeable, and earlier notes here that implied it were
wrong. Four concrete gaps remain:

- **Exported symbols differ** (`geario_http_*` vs `hyper4k_*`), so the
  Kotlin cinterop bindings do not resolve against this library as-is.
- **`respond` success value differs.** `hyper4k_respond` returns
  `HYPER4K_OK = 1` and the Kotlin binding treats `!= 0` as success;
  `geario_http_respond` returns `0`. The same binding would read every
  successful geario respond as a failure and geario's negative error codes
  as success.
- **Options layout differs**, and hyper4k has `read_idle_timeout_ms` and
  `max_buffered_bytes` with no geario counterpart.
- **Thread model differs, and this is the deep one.** hyper4k delivers an
  async response through a global concurrent map, so a Kotlin coroutine may
  resume on any thread and answer. geario is thread-per-core with `!Send`
  handles and answers only on the owning worker, returning `WRONG_THREAD`
  otherwise. The synchronous GET measured here never leaves one worker, so
  it proves nothing about async handlers, streaming backpressure, or
  cancellation under the real Kotlin dispatcher.

HTTP/2 is a gain over geario's *previous* native-h1 FFI, not over hyper4k,
which already has h2c and h2.

## What this does not claim

One host, two cores, 16 KB at the knee, synchronous handler. It says the
ABI hop over hyper is cheap and that hyper-on-geario is not behind
hyper-on-tokio here. It does not measure many workers, small responses,
TLS, async handlers, or hyper4k itself, and the native-stack gap is a real
few percent that a latency-sensitive HTTP/1-only consumer would notice --
which is why the native stack stays.

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
this host. In exchange the C ABI now reports, and delivers, the same
capabilities hyper4k does: server HTTP/1.1 and h2c on one port; client
HTTP/1.1, HTTP/2 by ALPN over TLS, a custom CA bundle, a proxy with CONNECT
for TLS targets, cancellation and streaming.

The comparison that decides whether geario-ffi can replace hyper4k is the
second row. hyper4k runs hyper on tokio; geario-ffi runs the same hyper on
geario. At this shape geario-ffi is a few percent ahead and speaks the same
ABI, so replacing hyper4k is a link-line change that does not lose HTTP/2
and does not lose throughput.

## What this does not claim

One host, two cores, 16 KB at the knee. It says the ABI hop over hyper is
cheap and that hyper-on-geario is not behind hyper-on-tokio here. It does
not measure many workers, small responses, or TLS, and the native-stack
gap is a real few percent that a latency-sensitive HTTP/1-only consumer
would still notice -- which is why the native stack stays.

# Shared benchmark contract

All servers answer identically so the client can verify it is comparing the
same work:

- `GET  /*`  -> 200, `content-type: application/json`, a body of
  `BENCH_BODY_SIZE` bytes (default 24) made of a repeating pattern.
- `POST /*`  -> the request body is read to completion first, then the same
  response. Reading it matters: a server that ignores the body is not doing
  the work being measured.

Environment:

| var | meaning |
| --- | --- |
| `BENCH_ADDR` | listen address |
| `BENCH_WORKERS` | worker count |
| `BENCH_SECONDS` | exit after N seconds, so a profiler can write its output |
| `BENCH_BODY_SIZE` | response body length in bytes |

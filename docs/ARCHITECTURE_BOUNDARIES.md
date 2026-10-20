# Transport and protocol boundaries

Status: current-source audit and incremental direction, not an HTTP/3 implementation.

## Current paths

There are two independent HTTP/1 implementations in this repository, not two
wrappers around the same parser:

1. Native Rust: geario accept/worker -> Io -> geario-http H1 dispatcher ->
   ntex-httparse/codec -> HttpService handler.
2. Hyper/FFI: geario accept/worker -> Io -> GearioTransport -> Hyper H1/H2 ->
   FFI request/response bridge. The FFI server currently uses serve_auto.

The September 18 Arena baseline measured path 1. Its result does not measure
path 2 or the cost of Kotlin/Native. Hyper is responsible for much more than
calling a parser: connection state, framing, bodies, flow control and protocol
errors remain protocol-engine responsibilities.

## Applied cleanup

- geario selects a reactor separately from running it. Polling, explicit
  io_uring, automatic Linux fallback and Windows IOCP share one runtime scope
  and unwind boundary. Driver construction still occurs outside that boundary,
  preserving failure behavior. Public APIs and worker placement are unchanged.
- GearioTransport uses one read-buffer transfer helper before and after
  readiness. The copy, EOF handling and wakeup registration are unchanged.
  Removing duplicate code is not a claim of zero-copy or measured speedup.
- Removed unused rcgen from geario-http's normal dependencies. FFI certificate
  fixtures retain their own dev-dependency. This reduces the normal dependency
  graph; it does not prove a reduction in runtime allocations or linked size.

## Next removals, ordered by risk

### Feature boundaries first

The http1 feature currently does not gate the public h1 module in lib.rs.
Hyper-only users therefore still compile native protocol code. The FFI
dependency also currently enables geario-http defaults. Before claiming a lean
Hyper-only build, gate native modules and dependent exports consistently, update
internal consumers and test no-default/server/client/hyper-only/FFI combinations.
Do not merely disable defaults in Cargo.toml and call the split complete.

The public geario test server resides in server/net/test.rs and uses UUIDs;
it is not cfg(test)-only. Moving UUID to dev-dependencies alone breaks normal
builds. A future test-support feature or separate crate needs an explicit
migration for consumers of TestServer. Likewise, do not remove service, config,
buffer or shutdown machinery solely because the baseline route is trivial.

### Optimize the measured path

Measure native-H1 and Hyper-on-geario with the same query/body handler first,
then FFI and Kotlin separately. Record copies, allocations, wakeups and queue
waits with bounded diagnostic overhead. Only optimize a layer implicated by
that comparison. Keep TLS filters, partial writes, cancellation and response
ownership covered when changing buffers or dispatch.

The native H1 engine should not be deleted solely to unify implementation.
Nor should the FFI maintain a second independent protocol implementation.
Decide which backend to retain for each role only after correctness and
equivalent-load measurements; expose backend capability differences explicitly.

## HTTP/3 direction

HTTP/3 maps HTTP semantics onto QUIC, not TCP. See
[RFC 9114](https://www.rfc-editor.org/rfc/rfc9114.html) and
[RFC 9000](https://www.rfc-editor.org/rfc/rfc9000.html).

- Keep geario responsible for execution, timers and transport I/O, not HTTP
  routing, headers, status codes or QPACK.
- Add a QUIC transport integration beside the TCP path. The current Reactor
  interface supplies TCP/Unix stream operations, not a QUIC-ready UDP API.
  Evaluate datagram ownership, batched receive/send, timers and cancellation
  against the chosen QUIC library before freezing a new public abstraction.
- Keep QUIC connections and bidirectional streams explicit. Do not emulate an
  entire QUIC connection as one TCP Io byte stream or add H3 to TCP preface
  sniffing. QUIC transport TLS is not the current TCP TLS filter.
- Share HTTP-facing semantics (headers, body chunks, trailers, cancellation,
  errors) at the protocol boundary, not a universal transport object. Borrowed
  FFI memory and completion/close rules must remain explicit for every backend.
- Do not add an H3 capability bit to runtime results until actual H3 tests pass.
  Required tests include ALPN, multi-stream isolation, stream reset, connection
  close, flow control/backpressure, slow consumers and clean shutdown. A UDP
  socket or a QUIC handshake alone is not HTTP/3 support.

No QUIC dependency, H3 ABI or speculative generic engine interface is added by
this cleanup. This leaves room for HTTP/3 without making H1 carry its complexity.

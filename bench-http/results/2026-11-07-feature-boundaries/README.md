# Feature isolation validation — 2026-11-07

This is build/correctness evidence, not a performance result.

Toolchain: Rust 1.99.0. Source identities and file hashes are in
`source-manifest.json`; the HTTP repository includes uncommitted changes.

## Local checks

- Production library checks: no features, proxy, native h1, server, client,
  full, TLS alone, compression, cookies, websocket/full extras, hyper runtime,
  hyper h1 server, hyper h2 client, hyper full.
- All-target checks cover the same principal combinations.
- FFI production library and tests: no features, server, client, native-h1,
  and server+client+native-h1.
- HTTP tests: hyper-full with defaults disabled; full+rustls+hyper-full.
- Windows GNU cross-check: HTTP hyper-full library with defaults disabled.
- `dependency-boundaries.txt`: independent consumer graphs reject native
  parsers in hyper-only/FFI hyper builds, TLS in plaintext FFI server-only,
  and hyper in native-h1-only builds. Dev dependencies are excluded.

Two HTTPS tests remain explicitly ignored; no passing result is claimed for
those external-network tests. Fixed an existing collision between two FFI
lifecycle tests using port 8129; the cross-thread test now uses 8131.

## Reproduce

```sh
python3 scripts/check-feature-boundaries.py
cargo test -p geario-http --no-default-features --features hyper-full
cargo test -p geario-http --features full,rustls,hyper-full
cargo test -p geario-http-ffi --no-default-features --features server,client,native-h1
```

CI additionally checks the library-only feature graph before enabling test
features, so test dependencies cannot hide missing production feature flags.


## Linux core runtime execution

Host: Fedora, Linux 7.0.12, 2 logical CPUs. Rust 1.99.0 installed under the
isolated validation directory; compilation used vendored locked dependencies.
Forced `neon-uring`, with no driver fallback. Full output: `linux-geario-tests.log`.

- Core library: 163 passed, 1 ignored.
- Direct write: 2 passed (TLS feature was not enabled in this core run).
- Multipage/partial write and disconnect: 2 passed.
- Directed byte-integrity test observed 5 vectored submits and 5 short writes.
  Its cancellation count was 0: this is not evidence of cancellation coverage.

This does not establish throughput, fixed-rate p99, or parity with Tokio.

Linux hyper-only execution also passed with defaults disabled and forced
`geario/neon-uring`: 7 unit tests and 12 integration tests (including one Tokio
control), with h1 keep-alive, h2 concurrency, slow readers and stream-cancel
isolation. Full output: `linux-hyper-only-tests.log`. This stream-cancel test
is distinct from forcing a pending kernel write cancellation.

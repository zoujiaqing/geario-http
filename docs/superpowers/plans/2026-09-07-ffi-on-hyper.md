# geario-http-ffi on hyper Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run the C ABI (`geario-http-ffi`) on hyper-over-geario so it gains HTTP/2 on both ends and can replace hyper4k without losing capability; leave the native HTTP/1 stack untouched.

**Architecture:** The FFI keeps its shape (responder tickets, callbacks, thread-per-core workers) and swaps the protocol engine: the server runs hyper's http1/http2 over `GearioTransport` with h2c prior-knowledge detection; the client gets a small connector (TCP, rustls with ALPN, protocol from ALPN) and a per-origin pool (one shared h2 `SendRequest` per origin, a list of idle h1 ones). The native `geario_http::{HttpService, client}` are no longer dependencies of the FFI.

**Tech Stack:** geario (net, rt, tls::rustls), geario-http `hyper_rt` (GearioTransport/Executor/Timer), hyper 1.11 (http1, http2, client, server), http-body-util, rustls 0.23 + webpki-roots + rustls-pki-types PEM.

**Spec:** docs/benchmarks/2026-09-07-native-h1-against-hyper.md (why), docs/superpowers/specs/2026-09-06-geario-http-ffi-design.md (ABI shape), hyper4k `lib/include/hyper4k.h` (numbering to match).

## Global Constraints

- Commit messages in English, no AI attribution of any kind.
- No provenance comments in source; attribution only in README/NOTICE.
- TLS is rustls only.
- `warnings = deny` is on: every unused import is a build failure.
- ABI numbering follows hyper4k where both define the same thing: capability bits, flags, error kinds. geario-only values live outside hyper4k's ranges.
- `panic = "abort"` in the FFI: nothing may panic across the boundary; every FFI entry validates its inputs.
- Formatting: `cargo fmt --all` before each commit; CI checks it.

---

### Task 1: h2c-aware `serve_auto` in `geario_http::hyper_rt`

**Files:**
- Create: `geario-http/src/hyper_rt/serve.rs`
- Modify: `geario-http/src/hyper_rt/mod.rs` (export)
- Test: `geario-http/tests/hyper_serve_auto.rs`

**Interfaces:**
- Produces: `pub enum Protocol { Http1, Http2 }`, `pub async fn detect<F: Filter>(io: &Io<F>) -> io::Result<Protocol>`, `pub async fn serve_auto<F, S, B>(io: Io<F>, service: S) -> Result<(), Box<dyn Error + Send + Sync>>` where `S: hyper::service::HttpService<Incoming, ResBody = B>`, `B: Body + 'static`, `B::Error: Into<BoxError>`, `B::Data: Send`.

- [ ] **Step 1: failing tests** — real sockets, both ends geario: (a) hyper http1 client request → 200; (b) hyper http2 client (prior knowledge, `GearioExecutor`) → 200 with `version() == HTTP_2`; (c) a peer that sends the first 10 bytes of the preface, waits 50 ms, then the rest → still Http2 (partial-prefix case).
- [ ] **Step 2: run, expect unresolved import.**
- [ ] **Step 3: implement.** `detect`: loop { `io.read_ready().await?` ; `let verdict = io.with_read_buf(|b| { if b.len() >= 24 { Some(&b[..24] == PREFACE) } else if PREFACE.starts_with(&b[..]) { None } else { Some(false) } })`; return on Some }. Do not consume bytes: hyper's h2 server expects the preface. `serve_auto`: match detect → `http1::Builder::new().timer(GearioTimer).serve_connection(GearioTransport::new(io), service).with_upgrades()` or `http2::Builder::new(GearioExecutor).timer(GearioTimer).serve_connection(...)`.
- [ ] **Step 4: tests pass.** `cargo test -p geario-http --features full,hyper-full --test hyper_serve_auto`
- [ ] **Step 5: commit** `Add serve_auto: h2c prior knowledge detected on the read buffer`.

### Task 2: ABI numbering aligned with hyper4k

**Files:** Modify `geario-http-ffi/src/abi.rs`, `geario-http-ffi/include/geario_http.h`, `geario-http-ffi/tests/header_matches.rs`.

**Interfaces (produces):**
```rust
pub const GEARIO_HTTP_SERVER_CAP_HTTP1: u64 = 1 << 0;  pub const GEARIO_HTTP_SERVER_CAP_H2C: u64 = 1 << 1;  pub const GEARIO_HTTP_SERVER_CAP_STREAMING: u64 = 1 << 2;
pub const GEARIO_HTTP_CLIENT_CAP_HTTP1: u64 = 1 << 0;  HTTP2 1<<1; TLS 1<<2; CUSTOM_CA 1<<3; CANCEL 1<<4; STREAMING 1<<5; PROXY 1<<6;
pub const GEARIO_HTTP_CLIENT_HTTP2_REQUIRED: u64 = 1 << 0;  pub const GEARIO_HTTP_CLIENT_CA_REPLACE_SYSTEM: u64 = 1 << 1;
// error kinds: NONE 0, DNS 1, CONNECT 2, TLS_CA 3, TLS_HOSTNAME 4, TLS_EXPIRED 5, TLS_OTHER 6, ALPN_NO_H2 7, PROTOCOL 8, TIMEOUT 9, IDLE_TIMEOUT 10, CANCELLED 11, TRUNCATED 12, OUTCOME_UNKNOWN 13; geario-only: INVALID_URL 40, UNSUPPORTED 41
```
- [ ] Step 1: write the pinning test (spelled-out numbers) + header drift test entries → fail. Step 2: apply constants (remove the shared `GEARIO_HTTP_CAP_*`), header, capability functions return the new server bits (H2C set only once Task 3 lands: leave it unset here and add in Task 3). Step 3: pass. Step 4: commit `Number the FFI capabilities, flags and error kinds the way hyper4k does`.

### Task 3: FFI server on hyper

**Files:** Modify `geario-http-ffi/src/server.rs`, `geario-http-ffi/Cargo.toml` (deps: `hyper` http1+http2+server+client, `http-body-util`, `http`, `geario-http` features `hyper-full,rustls` only), `tests/lifecycle.rs`.

- [ ] Step 1: tests: (a) existing lifecycle tests keep passing; (b) `curl --http2-prior-knowledge` GET returns body `ok` and `-w %{http_version}` prints `2`; (c) 200 KB POST over `--http2-prior-knowledge` echoes; (d) 17 MiB POST → 413 and handler not called (existing).
- [ ] Step 3: `dispatch(handler, req: hyper::Request<Incoming>) -> hyper::Response<BoxBody<Bytes, Infallible>>`: read body with `BodyExt::collect` bounded by `MAX_BODY` (use `http_body_util::Limited`), render headers, borrow slices, callback, await reply; `Reply::Once` → `Full::new(Bytes::from(body))`; `Reply::Stream` → `StreamBody::new(ChunkStream(rx))` mapping `Bytes` → `Ok(Frame::data(b))`. Connection: `serve_auto(io, service_fn(move |req| dispatch(handler, req)))`. Set `GEARIO_HTTP_SERVER_CAP_H2C`.
- [ ] Step 5: commit `Serve the FFI over hyper, with h2c`.

### Task 4: client connector

**Files:** Create `geario-http-ffi/src/client/connect.rs`; move `client.rs` → `client/mod.rs`.

**Interfaces (produces):**
```rust
pub(crate) struct Origin { scheme_tls: bool, host: String, port: u16 }
pub(crate) enum Sender { H1(hyper::client::conn::http1::SendRequest<Full<Bytes>>), H2(hyper::client::conn::http2::SendRequest<Full<Bytes>>) }
pub(crate) struct Tls { config: Arc<rustls::ClientConfig>, require_h2: bool }
pub(crate) async fn connect(origin: &Origin, tls: Option<&Tls>, connect_timeout: Millis, proxy: Option<&ProxyTarget>) -> Result<Sender, Fail>   // Fail { kind: GearioHttpErrorKind, message: String }
```
- [ ] Tests (real sockets): plaintext h1 → `Sender::H1`; TLS server offering ALPN h2 (rcgen CA, geario `TlsServerFilter`, hyper http2) → `Sender::H2`; TLS server without ALPN + `require_h2` → `Fail { kind: ALPN_NO_H2 }`; unknown host → `DNS`; closed port → `CONNECT`; wrong CA → `TLS_CA`; connect timeout `Millis(1)` to a blackhole address → `TIMEOUT`.
- [ ] Implement: resolve+connect via `geario::net::connect::connect(Connect::new(host).set_port(port))` under `timeout_checked`; TLS via `TlsClientFilter::create` with `ServerName::try_from(host)`; protocol from `io.query::<HttpProtocol>().get()`; h2 → `http2::Builder::new(GearioExecutor).timer(GearioTimer).handshake(GearioTransport::new(io))`, spawn `conn`; h1 → `http1::handshake`, spawn `conn`. Classify rustls errors: `InvalidCertificate(UnknownIssuer|BadSignature)` → TLS_CA, `NotValidForName` → TLS_HOSTNAME, `Expired` → TLS_EXPIRED, `NoApplicationProtocol` → ALPN_NO_H2, else TLS_OTHER.
- [ ] Commit `Add the FFI client connector: TCP, rustls with ALPN, protocol from the handshake`.

### Task 5: pool

**Files:** Create `geario-http-ffi/src/client/pool.rs`.

**Interfaces:** `pub(crate) struct Pool` (thread-local, `Rc<RefCell<...>>`), `async fn checkout(&self, origin: &Origin) -> Result<Lease, Fail>`, `Lease { sender: Sender, origin }`, `fn checkin(lease)`. h2: one `SendRequest` per origin, cloned on checkout, replaced when `is_ready()` is false and `ready().await` errs; h1: `Vec<SendRequest>`, pop idle, checkin pushes back if `is_ready()`.
- [ ] Tests: two sequential h1 requests reuse one connection (server counts accepts); two concurrent h2 requests share one connection; a closed h1 connection is discarded and a new one opened.
- [ ] Commit `Add a per-origin pool: shared h2 connection, idle h1 connections`.

### Task 6: `run_job` on hyper

**Files:** Modify `geario-http-ffi/src/client/mod.rs`.

- [ ] Replace `geario_http::client::Client` with `Pool`: build `hyper::Request<Full<Bytes>>` (absolute-form only when via plaintext proxy), `sender.send_request`; retries only for idempotent methods and only when `Fail.kind` is DNS/CONNECT/TLS_* (nothing was sent); `on_headers` with version `2` for `HTTP_2` else `1`; body via `Incoming` frames (`frame().await`, `into_data()`), pause = stop polling (flow control), cancel unchanged; request timeout via `timeout_checked(request_ms, ..)` → TIMEOUT; error after headers → TRUNCATED; error before response on a non-idempotent method → OUTCOME_UNKNOWN.
- [ ] Tests: existing `client_abi.rs` plus a real roundtrip against an FFI server (h1) and against an h2c server is *not* possible (no h2c client, matches hyper4k) — test h2 through the TLS test server from Task 4 instead; `version` byte asserted 1 and 2.
- [ ] Commit `Run FFI client requests through hyper`.

### Task 7: options: custom CA and flags

**Files:** `client/mod.rs`, header.
- [ ] Extend `GearioHttpClientOptions` after `proxy_url_len`: `custom_ca_pem: *const u8, custom_ca_pem_len: usize` (prefix rule keeps old callers working; `MIN_SIZE` unchanged). Flags accepted: `HTTP2_REQUIRED`, `CA_REPLACE_SYSTEM`; anything else → UNKNOWN_FLAGS. Roots: webpki-roots unless REPLACE_SYSTEM; PEM parsed with `rustls_pki_types::pem::PemObject` (`CertificateDer::pem_slice_iter`); a PEM that parses to zero certificates → INVALID_ARG at `client_new` (fail at construction, not first request).
- [ ] Tests: garbage PEM → INVALID_ARG; valid CA → https to the rcgen server succeeds; REPLACE_SYSTEM without CA → https to the same server fails TLS_CA.
- [ ] Commit `Accept a custom CA bundle and the HTTP2_REQUIRED / CA_REPLACE_SYSTEM flags`.

### Task 8: proxy, including CONNECT for TLS

**Files:** `client/connect.rs`.
- [ ] Plaintext: connect to proxy, absolute-form request line (hyper does this when the URI has scheme+authority and we set `.uri(full)`). TLS: `CONNECT host:port HTTP/1.1\r\nHost: host:port\r\n\r\n` on the raw Io, read until `\r\n\r\n`, require 2xx, then TLS as usual. Tests: a tiny proxy in the test (accept, parse CONNECT, dial, splice) for TLS; plaintext proxy sees absolute-form.
- [ ] Commit `Tunnel TLS through the proxy with CONNECT`.

### Task 9: docs, header, CI, C examples
- [ ] README/README.zh-Hans: FFI now on hyper; capabilities; flags; error kinds. Header comments updated. `ffi` CI job already runs C examples; add `--http2-prior-knowledge` curl to a Rust test rather than to CI shell. Remove `geario-http/client` + `server` features from the FFI's dependency (FFI uses `hyper-full,rustls`); make sure `cargo check -p geario-http-ffi --no-default-features --features server` and `client` still build.
- [ ] Commit `Document the FFI on hyper`.

### Task 10: measure
- [ ] `bench-http`: an FFI-server binary (C example `server` at BENCH_BODY_SIZE) vs `srv-polling` at the knee on Fedora; record in docs/benchmarks. Expectation from the h1 data: FFI ≈ hyper-on-geario, i.e. two to four points below the native stack it replaces.

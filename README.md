# geario-http

HTTP protocol layer for [geario](https://github.com/zoujiaqing/geario).

Currently speaks HTTP/1.1, server and client. Derived from the HTTP layer of
the [ntex](https://github.com/ntex-rs/ntex) framework.

## Crates

| Crate | What it is |
| --- | --- |
| `geario-http` | The protocol layer: types, HTTP/1.1 codec, server, client |
| `geario-http-ffi` | C ABI over `geario-http`, for embedding in non-Rust hosts |

## Layout

`geario` is a path dependency on a sibling checkout:

    projects/
    ├── geario/          # the IO stack
    └── geario-http/     # this repository

That keeps the two moving together while the IO API is still settling. It
becomes a version dependency once geario stabilises.

## Features

| Feature | Default | What it pulls in |
| --- | --- | --- |
| `http1` | yes | HTTP/1.1 codec, decoder, encoder |
| `server` | yes | `HttpService`, dispatcher, control service |
| `client` | no | `Client`, request builder, connection pool |
| `full` | no | `http1` + `server` + `client` |
| `compress` | no | gzip/deflate transfer encoding |
| `cookie` | no | cookie parsing and building |

A server-only build is about 46% smaller than `full`, which matters when the
library is linked into an FFI target.

Reserved but not implemented: `http2`, `openssl`, `rustls`, `ws`, `test-server`.

## Trying it

    cargo run -p geario-http --example hello        # then curl localhost:8080
    cargo run -p geario-http --example roundtrip    # client against own server

    cargo build -p geario-http-ffi --release
    make -C geario-http-ffi/examples && ./geario-http-ffi/examples/version

## License

MIT OR Apache-2.0, matching upstream ntex. See `NOTICE` for attribution.

## Documentation

- [中文说明](README.zh-Hans.md)

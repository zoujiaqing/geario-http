# geario-http

[geario](https://github.com/zoujiaqing/geario) 的 HTTP 协议层。

目前支持 HTTP/1.1 的服务端与客户端。源自 [ntex](https://github.com/ntex-rs/ntex)
框架的 HTTP 层。

## Crates

| Crate | 内容 |
| --- | --- |
| `geario-http` | 协议层：类型、HTTP/1.1 codec、服务端、客户端 |
| `geario-http-ffi` | 基于 `geario-http` 的 C ABI，供非 Rust 宿主嵌入 |

## 目录约定

`geario` 以同级检出的 path 依赖引入：

    projects/
    ├── geario/          # IO 底座
    └── geario-http/     # 本仓库

这样在 IO API 尚未稳定期间两者可以同步演进。geario 稳定后改为版本依赖。

## Feature

| Feature | 默认 | 内容 |
| --- | --- | --- |
| `http1` | 是 | HTTP/1.1 codec、decoder、encoder |
| `server` | 是 | `HttpService`、dispatcher、control service |
| `client` | 否 | `Client`、请求构建器、连接池 |
| `full` | 否 | `http1` + `server` + `client` |
| `compress` | 否 | gzip/deflate 传输编码 |
| `cookie` | 否 | cookie 解析与构建 |

只开 server 的构建比 `full` 小约 46%，这在把库链进 FFI 目标时是实际收益。

已预留但尚未实现：`http2`、`openssl`、`rustls`、`ws`、`test-server`。

## 试用

    cargo run -p geario-http --example hello        # 然后 curl localhost:8080
    cargo run -p geario-http --example roundtrip    # 客户端打自己的服务端

    cargo build -p geario-http-ffi --release
    make -C geario-http-ffi/examples && ./geario-http-ffi/examples/version

## 许可证

MIT OR Apache-2.0，与上游 ntex 一致。归属信息见 `NOTICE`。

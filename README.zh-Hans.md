# geario-http

[geario](https://github.com/zoujiaqing/geario) 的 HTTP 协议层，
从设计之初就同时面向 Rust 与 Kotlin/Native。

目前支持 HTTP/1.1 的服务端与客户端。源自 [ntex](https://github.com/ntex-rs/ntex)
框架的 HTTP 层。

## 两个入口

C ABI 在这里不是补丁。Kotlin/Native 是一等消费者，所以 `geario-http-ffi`
就放在本仓库，与 Rust API 同步演进，而不是事后再补。

| Crate | 面向 | 内容 |
| --- | --- | --- |
| `geario-http` | Rust | 协议层：类型、HTTP/1.1 codec、服务端、客户端 |
| `geario-http-ffi` | C、Kotlin/Native | 基于 `geario-http` 的 C ABI，产出 staticlib 与 cdylib |

这套 ABI 与 [hyper4k](https://github.com/netonstream/hyper4k) 已经在用的一致，
宿主换引擎只需改链接参数。能力位由 cargo feature 推导，宿主可以在运行时
问出某个构建到底带了什么。

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
| `rustls` | 否 | TLS，走 geario 的 rustls 层 |
| `ws` | 否 | WebSocket 客户端与 codec |

只开 server 的构建比 `full` 小约 46%，这在把库链进 FFI 目标时是实际收益。

已预留但尚未实现：`http2`、`openssl`、`test-server`。

明文目标支持 HTTP 代理。TLS 走代理需要 CONNECT 隧道，尚未实现，
会明确拒绝而不是绕过代理直连。

## 试用

    cargo run -p geario-http --example hello        # 然后 curl localhost:8080
    cargo run -p geario-http --example roundtrip    # 客户端打自己的服务端

    cargo build -p geario-http-ffi --release
    make -C geario-http-ffi/examples && ./geario-http-ffi/examples/version

## 许可证

MIT OR Apache-2.0，与上游 ntex 一致。归属信息见 `NOTICE`。

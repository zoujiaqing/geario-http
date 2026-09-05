# geario-http-ffi 设计:C ABI

- 日期:2026-09-06
- 状态:待实现
- 前置:geario 阶段一完成;geario-http 的 h1 服务端与客户端完成

## 1. 核心判断:实现 hyper4k 已有的 ABI,不另立一套

`hyper4k` 已经把最难的部分做完了并且发布到了 Maven Central:

- 20 个 `#[no_mangle]` 导出符号,覆盖服务端与客户端两侧
- 结构体版本化(`abi_version` + `struct_size` + `flags` 校验)
- 定宽状态码(`Hyper4kStatus = i32`),不用 C `enum`(宽度实现定义)
- `panic = "abort"`,panic 不跨 FFI 边界
- Kotlin/Native 侧 cinterop 已对接并验证

**geario 的 FFI 应当实现同一套 ABI,而不是另设计一套。** 这样换引擎对
Neton 的 Kotlin 代码是零改动:只把链接的静态库从 `libhyper4k.a` 换成
`libgeario_http_ffi.a`。ABI 层的对错已经用生产验证过了,重新设计只会重新踩坑。

已有的符号面(来自 `hyper4k/lib/src`):

| 分组 | 符号 |
| --- | --- |
| 元信息 | `abi_version`、`version`、`server_capabilities`、`client_capabilities` |
| 服务端 | `server_start`、`server_stop` |
| 响应 | `respond`、`response_begin`、`response_write`、`response_finish` |
| 客户端 | `client_new`、`client_free`、`client_close`、`client_options_init`、`client_request_init`、`client_send`、`client_cancel`、`client_resume` |
| 背压观测 | `client_inflight_count`、`client_paused_stream_count` |

回调驱动 + 显式背压(`cancel`/`resume`/`paused_stream_count`)这个形状是对的,
照搬。

## 2. 与 hyper 自己的 C API 的差别

hyper 的 `src/ffi/`(2,293 行)是另一种取向,geario 不跟:

| | hyper ffi | hyper4k ABI |
| --- | --- | --- |
| 执行模型 | 手动 `hyper_executor` + `hyper_task` 轮询 | 回调 |
| 调用方负担 | 调用方自己驱动事件循环 | 库内持有运行时 |
| 稳定性 | 显式 unstable,需 `--cfg hyper_unstable_ffi` | 版本化结构体 |

hyper 的 task/executor 模型把异步调度暴露给了 C 调用方,对 Kotlin/Native 这种
本身有协程调度器的宿主是负担。hyper4k 的回调模型把运行时关在库内,宿主只提供
函数指针 —— 这对 Neton 是正确的取舍,保留。

## 3. 包结构

```
geario/
└── geario-http-ffi/
    ├── Cargo.toml          # crate-type = ["staticlib", "cdylib", "rlib"]
    ├── build.rs
    ├── cbindgen.toml
    ├── include/geario.h    # 生成产物，入库
    └── src/
        ├── lib.rs
        ├── abi.rs          # 状态码、结构体版本、能力位
        ├── server.rs
        ├── client.rs
        ├── response.rs
        └── slice.rs        # 借出的字节视图
```

独立 crate 而不是 `geario-http` 的 feature:FFI 需要 `staticlib`/`cdylib`
产物类型和 `panic = "abort"`,这些是 crate 级设置,塞进 feature 会污染
普通 Rust 使用者的构建。

`rlib` 也产出,让集成测试能链接这个 crate —— 与 hyper4k 同一做法。

## 4. Cargo 配置要点

```toml
[lib]
crate-type = ["staticlib", "cdylib", "rlib"]

[dependencies]
geario = { workspace = true }
geario-http = { workspace = true, features = ["full"] }

[profile.release]
panic = "abort"   # panic 不得跨 FFI 边界，那是 UB
lto = true
codegen-units = 1
strip = true
```

`features = ["full"]` 是当前值。FFI 构建可以按需只开 `server` 或 `client` ——
第 5 节的能力位就是为此存在的,让宿主在运行时问清楚这个 `.a` 到底带了什么。

## 5. 能力位:让 feature 分层对宿主可见

geario-http 的 `server` / `client` / `http1` feature 决定了链进来的东西。
宿主必须能在运行时问出来,否则调一个没编进去的函数只能得到空指针,无从诊断。

```c
uint64_t geario_server_capabilities(void);
uint64_t geario_client_capabilities(void);
```

位定义与 `Cargo.toml` 的 feature 一一对应,`#[cfg(feature = ...)]` 直接决定
是否置位。宿主在 `dlopen`/启动时读一次即可。

这是 geario 相对 hyper4k 的一处改进:hyper4k 的能力位是手写常量,geario 的
由 feature 直接推导,不会出现"编译时关了但能力位还报有"的偏差。

## 6. 线程模型

**这是 FFI 层最需要想清楚的一件事,也是与 hyper4k 差别最大的地方。**

hyper4k 走的是 `tokio::runtime::Builder::new_multi_thread()` —— work-stealing
多线程运行时。geario 的运行时是 thread-per-core、不共享、`!Send` 的:
`Io`、`Service`、`Pipeline` 全部基于 `Rc`。

后果:

1. **回调会在 worker 线程上被调用,且同一连接始终在同一线程。** 这比
   work-stealing 更容易推理,宿主的 per-connection 状态不需要跨线程同步。
2. **宿主的回调必须是线程安全的**,因为不同连接在不同 worker 上。
   与 hyper4k 的约束相同,文档措辞可以照搬。
3. **不能把 `Io`/`Response` 句柄跨线程传递。** hyper4k 的 ABI 里句柄是
   `*mut c_void`,类型系统管不到,只能靠文档和运行时检查。
   建议:句柄内嵌 worker id,跨线程使用时返回明确的状态码而不是 UB。

第 3 点是 geario 相对 hyper4k 必须新增的检查项,写进实现计划。

## 6b. 信号:库不得抢宿主的处理器

geario 的 server 默认装 SIGINT/SIGTERM/SIGQUIT 处理器(`no_signals` 默认
`false`)。在被链接的库里这会**静默替换掉宿主已装的处理器** —— 实测中一个 C
程序装了自己的 SIGINT handler,起完服务后就再也收不到 Ctrl-C,无法自行退出。

FFI 层必须调 `.disable_signals()`。信号是宿主的事,库无权接管。

回归测试见 `geario-http-ffi/tests/lifecycle.rs`:它不发信号,而是用
`signal()` 的返回值读回 SIGINT 当前归属 —— 发信号的测试要么不可靠、要么会
打死测试进程。该测试已双向验证:去掉 `.disable_signals()` 会失败。

## 7. 实施顺序

1. ~~**骨架 + 元信息**~~ 已完成:crate、`abi_version`、`version`、能力位。
   验收通过:C 程序链接并正确报告 feature 推导的能力位。
2. ~~**服务端**~~ 已完成:`server_start`/`server_stop`/`respond`。
   验收通过:C 程序起服务,`curl` 拿到 200,自定义响应头透传,
   keep-alive 下 `user_data` 计数正确,SIGINT 后干净退出。
3. ~~**流式响应**~~ 已完成:`response_begin`/`write`/`finish`。
   验收通过:`curl` 收到 `transfer-encoding: chunked` 与全部 chunk;
   一次性与流式在同一 responder 上互斥,两个方向都拒绝。
4. ~~**客户端**~~ 已完成:`client_new`/`options_init`/`request_init`/`send`/
   `close`/`free`。验收通过:C 程序同时起服务端与客户端并完成三次往返。
5. ~~**背压**~~ 已完成:`cancel`/`resume`/`inflight_count`/
   `paused_stream_count`,以及 `max_inflight_requests` 上限。

   实现中踩到一个真 bug:`inflight` 原本在 worker 线程的任务里递增,而
   `client_send` 排完队就返回,导致八次发送在计数器仍为 0 时全部放行 ——
   **上限形同虚设**。改为在 `client_send` 里用 `fetch_update` 原子地
   "检查并占位",排队失败时归还。回归测试见
   `geario-http-ffi/tests/client_abi.rs`。
6. **句柄的线程归属检查**(第 6 节第 3 点)。
7. **与 hyper4k 的 ABI 一致性测试**:同一份 C 测试程序分别链接
   `libhyper4k.a` 与 `libgeario_http_ffi.a`,行为必须一致。

第 7 步是整件事的验收:**如果同一份 C 代码在两个库上表现一致,
Neton 换引擎就是改一行链接参数。**

## 8. 前置未决项

- **HTTP/2**。hyper4k 的 ABI 有 h2 相关能力位与 `client_paused_stream_count`
  这类 h2 概念(stream)。geario-http 目前只有 h1,这些位应报 0,
  相关函数返回"不支持"状态码而不是假装工作。
- **TLS**。hyper4k 内置 rustls,geario 尚未移植 `ntex-tls`。
  没有 TLS 的客户端只能打 `http://`,能力位必须如实反映。

这两项决定了 geario-http-ffi 首版能替换 hyper4k 的哪些场景,不能含糊。

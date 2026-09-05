# geario-http 阶段二设计:HTTP/1.1 协议层

- 日期:2026-09-05
- 状态:待实现
- 前置:geario 阶段一已完成(10 个模块、42,098 行、测试全绿)

## 1. 目标与范围

把 ntex 的 HTTP 类型层与 HTTP/1.1 协议栈移植成 `geario-http`,共约 11,170 行。
与阶段一同样是**纯移植**:只改目录与路径,不改代码逻辑。

### 本期做

| 组成 | 上游位置 | 行数 |
|---|---|---|
| 类型层 | `ntex-http/src/` | 2,988 |
| 公共层 | `ntex/src/http/*.rs`(不含 `test.rs`) | 3,387 |
| HTTP/1.1 | `ntex/src/http/h1/` | 4,795 |
| | **合计** | **11,170** |

### 本期不做

- **HTTP/2**。`ntex/src/http/h2/`(693 行)只是适配层,真正的实现在
  `ntex-h2` —— **17,191 行的独立仓库**(commit `5c7005b`,分支 `service-updates`),
  且**对着 ntex 自己当前的 API 就编译不过**(15 个错误,主要是 `Pipeline::new`
  由 2 参数变 1 参数、`IntoService` 约束变化)。移植它等于接手一个自己没写过、
  上游也没修好的 h2 实现。另开一期决策。
- **`ntex/src/http/test.rs`**(511 行)。它依赖 `crate::client`、`crate::ws`、
  `ntex_tls`,三者都不在 geario 生态内。
- TLS、压缩(`encoding/`,`compress` feature)、cookie。
- 精简、改类型名、改公开 API 形状 —— 与阶段一同样的护栏。

## 2. 依赖核查(已实测)

`ntex/src/http/` 对 ntex façade 的引用分布:

| 引用 | 处数 | geario 是否已具备 |
|---|---|---|
| `crate::http`(内部) | 66 | — |
| `crate::util` | 15 | ✅ `geario::util` / `geario::bytes` |
| `crate::io` | 13 | ✅ `geario::io` |
| `crate::rt` | 11 | ✅ `geario::rt` |
| `crate::service` | 9 | ✅ `geario::service` |
| `crate::server` | 4 | ✅ `geario::server` |
| `crate::channel` | 4 | ✅ `geario::util::channel` |
| `crate::error` | 2 | ✅ `geario::error` |
| `crate::connect` | 2 | ✅ `geario::net::connect` |
| `crate::time` | 1 | ✅ `geario::util::time` |
| `crate::codec` | 1 | ✅ `geario::codec` |
| `crate::client` / `crate::ws` | 3 | ❌ 仅出现在 `test.rs` 与一处 `#[cfg(test)]` |
| `ntex_tls` | 1 | ❌ 仅 `test.rs` |

**结论:去掉 `test.rs` 和 h2 之后,geario-http 对 geario 之外只依赖
`ntex-httparse` 一个 crate。** 这是本期风险低的根本原因。

## 3. 包结构

`geario-http` 作为 **geario workspace 的成员**,不另开仓库:

```
geario/
├── Cargo.toml            # members = ["geario", "geario-macros", "geario-http"]
├── geario/
├── geario-macros/
└── geario-http/
    ├── Cargo.toml        # geario = { workspace = true }
    └── src/
        ├── lib.rs        # 来自 ntex/src/http/mod.rs（façade）
        ├── types/        # 来自 ntex-http/src/
        ├── config.rs
        ├── error.rs
        ├── helpers.rs
        ├── httpcodes.rs
        ├── httpmessage.rs
        ├── message.rs
        ├── payload.rs
        ├── request.rs
        ├── response.rs
        ├── service.rs
        └── h1/
```

同 workspace 的理由:两者锁步开发,`geario-http` 用 path 依赖 `geario`,
避免迭代期反复发版;单一 CI。

## 4. 模块布局与命名冲突

### 冲突:两个 `error`

- `ntex-http/src/error.rs` —— header/body 层的 `Error`
- `ntex/src/http/error.rs` —— 协议层的 `PayloadError`/`DispatchError`/`ResponseError`

两者在 geario-http 里都想叫 `error`。

**解法:`ntex-http` 的内容整体放进 `src/types/`,协议层占据 crate 根。**

| 上游 | geario-http |
|---|---|
| `ntex-http/src/{body,error,header,map,serde,value}.rs` | `src/types/` |
| `ntex/src/http/*.rs` | crate 根 |
| `ntex/src/http/h1/` | `src/h1/` |

### 对外 API 保持不变

`lib.rs` 照抄上游 `ntex/src/http/mod.rs` 的再导出,把 `ntex_http::` 换成
`crate::types::`:

```rust
pub use crate::types::uri::{self, Uri};
pub use crate::types::{HeaderMap, Method, StatusCode, Version, body, header};
```

于是用户看到的仍是 `geario_http::HeaderMap`、`geario_http::header::*`、
`geario_http::body::*`,与上游 `ntex::http::*` 一一对应。`types` 是内部组织手段,
不是新增的对外层级。

## 5. h2 缺席处的处理

h2 在 `h2/` 目录之外还有 5 处耦合。本期**不删除、不猜测**,逐处按下表处理,
并在实现计划中逐条列出,使阶段 2b 成为已知量:

| 文件 | 构造 | 本期处理 |
|---|---|---|
| `mod.rs`(→ `lib.rs`) | `pub mod h2;` | 不声明 |
| `mod.rs` | `ALPN_PROTO_H2`、`ALPN_PROTOS` | 保留常量(纯字符串,无依赖) |
| `error.rs` | `H2Error` enum、`PayloadError::Http2Payload` | 不移植该 variant 与 enum |
| `config.rs:303` | `ntex_h2::ServiceConfig::shutdown()` | 移除该调用 |
| `payload.rs` | `Payload::H2` variant | 不移植该 variant |
| `service.rs` | `HttpService::h2()`、`h2_ctl` 字段 | 不移植 |

`service.rs`(232 行)是耦合最深的一个 —— `HttpService` 是 h1/h2 合一并做 ALPN
协商的入口。本期它退化为只有 h1 分支。**阶段 2b 恢复 h2 时,这个文件需要重新对照
上游移植,而不是增量修补。** 这一点必须写进阶段 2b 的前置说明。

## 6. 外部依赖

| crate | 用途 | 处理 |
|---|---|---|
| `ntex-httparse` (2.1.0) | h1 请求行/头解析 | 保持原名引用,不 vendor |
| `http` (1.5.0) | `Uri`/`Method`/`StatusCode`/`Version` 底座 | 直接依赖 |
| `httpdate` | Date 头格式化 | 直接依赖 |
| `itoa` | 数字快速格式化 | 直接依赖 |
| `serde` | 类型层 serde 支持 | 直接依赖 |
| `foldhash`、`log`、`thiserror`、`bitflags`、`pin-project-lite` | — | 直接依赖 |

依赖包名带 `ntex` 不影响 geario-http 自身的命名,与阶段一同一原则。

## 7. 重命名规程

沿用阶段一的两条规则,顺序同样不可交换(A 先 B 后)。

**规则 A —— 内部 `crate::` 提升**

- `ntex-http/src/` 内的 `crate::` → `crate::types::`
- `ntex/src/http/` 内的 `crate::http::` → `crate::`(它本来是 ntex 的子模块,
  现在自己就是 crate 根,**这条是降级不是提升**)
- `ntex/src/http/` 内其余 `crate::X` → `geario::X`(见规则 B 的映射表)

**规则 B —— 跨 crate 路径映射**

| 上游 | geario-http |
|---|---|
| `ntex_http::` | `crate::types::` |
| `crate::http::` | `crate::` |
| `crate::util::{Bytes,BytesMut,ByteString,Buf,BufMut,BytePages}` | `geario::bytes::*` |
| `crate::util::{lazy,select,Either,...}` | `geario::util::future::*` |
| `crate::util::{HashMap,HashSet,dyn_err,...}` | `geario::util::*` |
| `crate::channel::` | `geario::util::channel::` |
| `crate::time::` | `geario::util::time::` |
| `crate::io::` | `geario::io::` |
| `crate::rt::` | `geario::rt::` |
| `crate::service::` / `crate::{Service,Ctx,...}` | `geario::service::` |
| `crate::server::` | `geario::server::net::` |
| `crate::connect::` | `geario::net::connect::` |
| `crate::error::` | `geario::error::` |
| `crate::codec::` | `geario::codec::` |
| `ntex_httparse::` | 不动(外部 crate) |
| `#[ntex::test]` | `#[geario::test]` |
| `crate::rt_test` | `#[geario::test]` |

**阶段一学到的三类合并误伤,本期同样适用:**

1. `#[macro_export]` 的宏永远在 crate 根,规则 A/B 不得给它们加模块段
2. `self::` 只在 `mod.rs` 里等价于所在模块,子模块里会指向自己
3. `pub(super)` 在原 depth-1 模块里等价于 `pub(crate)`,合并后会收窄

**新增第 4 类(本期特有):** `ntex/src/http/` 原本是 depth-1 模块,移植后成为 crate 根,
所以它内部的 `pub(super)` 会**放宽**而不是收窄,不需要调整;但 `pub(crate)` 的
含义从"整个 ntex"缩小为"整个 geario-http",若有跨模块引用会暴露为编译错误。

## 8. 验收标准

1. `cargo build -p geario-http` 通过(macOS/polling)
2. `cargo test -p geario-http` 全绿
3. `grep -rE '\bntex\b|ntex_|ntex-' geario-http/src/` 只剩 `ntex-httparse`
4. **端到端**:`geario-http/examples/hello.rs` 起一个 HTTP/1.1 服务,
   `curl` 拿到 200 与正确 body
5. 移植过程中不得引入任何 `TODO`/`unimplemented!()`

第 4 条是本期真正的成功信号 —— 阶段一验证了 IO 抽象能跑通字节,
本期要验证它能跑通一个真实协议。

## 9. 与阶段一的已知遗留

以下三项在阶段一发现,不阻塞本期,记录以免遗忘:

1. **对标压测未完成**(阶段一 Task 13/14)。因 `ntex` 主 crate 编译不过
   (`ntex-h2` 损坏),原计划让 `bench-echo/server-ntex` 依赖 `ntex` 不可行,
   需改为直接依赖 `ntex-io`/`ntex-net`/`ntex-server`/`ntex-codec` 四个子 crate。
2. **上游 fork 点保持 `48eef5bd`**。上游已前进到 `732dbce6`("Remove unused code",
   删除 `map_config.rs` 157 行与 `variant.rs` 425 行)。不跟进的理由:
   那是删减,而 geario 的精简阶段排在 geario-http 之后。
3. **微基准的两个用例被禁用**,原因写在 `geario/benches/` 的注释里:
   上游 bench 本身对当前 buffer API 已过时。

# geario-http Rust baseline（2026-09-18）

## 范围

仅测试 geario-http 原生 HTTP/1.1 + geario io_uring 运行时。不经过
geario-http-ffi、Hyper、Kotlin/Native 或 Neton，不代表完整框架成绩。
只新增测试入口，没有修改引擎实现或发布版本。

测试入口为 `../../server-geario/src/bin/arena-baseline.rs`：解析查询参数 a/b，
完整读取 POST 请求体并解析整数，再逐请求计算返回值。GET、Content-Length
POST 和 chunked POST 都运行；不是固定响应，也没有缓存计算结果。

## 环境与方法

- 两台 Fedora 44 / 16 vCPU / 32 GiB 云主机，来宾拓扑为 8 核 x 2 SMT。
  一台服务端，一台独立负载端，流量走内网。
- geario：`5a2bf6d53c2ae5f140313c5c97bcc2756181bb0a`。
- geario-http：`70e36d791c50748707c23ff14273477dbc6d8661`，外加本测试入口。
- HttpArena：`758c775a64960f778fe0b136cecf1b45c31d1a8a`。
- gcannon v0.5.3，`43b30bc24c8cc6aab37dd7d718b47ffb985e6558`。
- Rust 1.98.1，release / LTO / codegen-units=1，`-C target-cpu=native`。
- 显式启用 `--features uring`，禁止驱动失败后静默降级；运行中的
  io_uring 描述符见 `runtime-fds.txt`。16 个服务 worker。
- 原始 `scripts/benchmark.sh geario-rust baseline`，4096 连接、16 个
  负载线程、pipeline=1、5 秒 x 3 轮，共执行三组。每轮日志有
  `Templates: 3` 和三项 `Per-template-ok`。没有 `--save`。
- 和之前本机对照一样，Docker Hub 不可达，使用原生二进制及 Fedora
  共享库打包的本地容器。不是公开 Arena 硬件、镜像和排名环境。
- sysctl 与此前双机对照一致，结束后恢复备份；受限 SSH 压测授权和临时
  防火墙规则已清理，测试容器已停止。

## 结果

| 组 | gcannon 三轮吞吐（K RPS） | Arena 脚本最佳 RPS |
|---|---|---:|
| 1 | 835.95 / 842.88 / 839.50 | 843,098 |
| 2 | 841.89 / 841.85 / 844.48 | 844,613 |
| 3 | 844.98 / 844.06 / 842.98 | 845,104 |

九轮 gcannon 中位数约 **842,880 RPS**，范围 **835,950–844,980 RPS**。
脚本最佳值与 gcannon 展示值口径略有差异，分别原样记录，不混用。
脚本采样 CPU 为 **933.9%–1060.1%**，内存 **83–92 MiB**。
全部响应状态计数为 2xx，三模板完成计数与成功计数一致。

同一套机器此前 Actix 的三轮最佳为 732,835 RPS，Neton beta18 为
375,521 RPS。这是**历史参考，不是本轮交错对照**；不能据此宣布公开
榜单胜负，也不能推断 Neton 更换引擎后将达到 84 万。

## 正确性与复跑

两个入口单元测试通过。真实 socket 验证覆盖负数、百分号编码查询键、
Content-Length、拆成两块的 chunked body，同一连接连续请求；
改变输入后输出随之改变。见 `tests.log` 和 `run.log`。

```sh
cd geario-http/bench-http
# 复现这次依赖时，先将本目录的 Cargo.lock.snapshot 放到 bench-http/Cargo.lock。
RUSTFLAGS='-C target-cpu=native' cargo build --locked --release \
  -p server-geario --bin arena-baseline --features uring
RUSTFLAGS='-C target-cpu=native' cargo test --locked --release \
  -p server-geario --bin arena-baseline --features uring
```

`run.py` 是本次原始脚本驱动，依赖先前双机实验的目录、受限 SSH 代理、
请求模板和容器环境，不是通用一键部署脚本。基准二进制 SHA-256：

`aa6690c135bdd14262c960c432b2fbe23234a7be9b1a32c5337695bf80d4e3b0`

`tested-source.rs` 保留实际编译时入口；主入口后续仅做了 rustfmt 格式化。
Cargo 提示仓库原有 `profile.release.force-frame-pointers` 无效，本次没有
据此声称启用了帧指针，且没有采样归因。

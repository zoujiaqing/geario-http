# Native H1 vs Hyper H1 on geario

## 结论

本机双机内网 Arena baseline 对照中，**原生 H1 路径领先约 19.2%**。
这比较的是整个 H1 调用链，而非 ntex-httparse 与 httparse 的纯解析性能。

| 实现 | 六轮中位数 RPS | 六轮范围 RPS | 脚本内存采样 |
|---|---:|---:|---:|
| geario-http 原生 H1 / ntex-httparse | 851,535 | 846,160–861,510 | 82–95 MiB |
| Hyper H1 / GearioTransport | 714,610 | 706,630–719,720 | 113–148 MiB |

原生 H1 的最慢一轮也高于 Hyper H1 的最快一轮。建议当前保留原生 H1
作为性能路径，不为了统一封装而直接替换成 Hyper H1。这里不证明 Hyper
在其他运行时/适配层、协议或负载下同样落后，也不能预告 Kotlin/FFI 收益。

## 可比性

- 两台 Fedora 44，服务端、负载端各 16 vCPU / 32 GiB，来宾拓扑为
  8 核 x 2 SMT；走内网，不是 16 个物理核或公开 Arena 排名机器。
- 都是纯 Rust，都使用 geario 的 accept、16 worker、io_uring、默认
  SharedCfg 和默认 backlog。Hyper 不启动 Tokio runtime。
- 使用同一份 `bench-http/arena-common.rs` 解析查询参数、整数和求和，
  相同 path/method 检查、1 MiB 请求体上限和相同 geario BytesMut 聚合。
  动态生成响应，不缓存业务结果。
- Hyper 使用 `http1::Builder`、GearioTimer、GearioTransport；原生 H1
  使用 HttpService、原生 dispatcher 和 codec。这些是待比较的差异。
- 源码基于 geario `5a2bf6d53c2ae5f140313c5c97bcc2756181bb0a`、
  geario-http `70e36d791c50748707c23ff14273477dbc6d8661`，双方都包含
  本地运行时入口合并、读缓冲 helper 合并和移除未用 rcgen 的清理改动。
- Rust 1.98.1，release、LTO、codegen-units=1、target-cpu=native，
  同一次构建生成两个二进制。Hyper 1.11.1，ntex-httparse 2.1.0，
  httparse 1.10.1；完整依赖见 Cargo.lock.snapshot，制品哈希见 artifacts.json。
- Docker Hub 不可达，二者均以原生二进制和 Fedora 共享库打包本地镜像。

## 测量

运行原始 HttpArena `scripts/benchmark.sh`，源码
`758c775a64960f778fe0b136cecf1b45c31d1a8a`。gcannon v0.5.3，
4096 连接、16 负载线程、pipeline=1，GET、定长 POST、chunked POST 三模板。
顺序 A/B/B/A，每组三轮五秒，没有 --save，没有改公开成绩。

| 组 | 路径 | gcannon 三轮 K RPS | Arena 脚本最佳 RPS |
|---|---|---|---:|
| A1 | 原生 H1 | 847.71 / 856.18 / 852.16 | 856,299 |
| B1 | Hyper H1 | 717.32 / 716.10 / 719.72 | 719,805 |
| B2 | Hyper H1 | 713.12 / 708.25 / 706.63 | 713,240 |
| A2 | 原生 H1 | 850.91 / 861.51 / 846.16 | 861,626 |

中位数取所有 gcannon 样本，不拿组内最佳值计算；Arena 脚本最佳值单列，
不混用两个展示口径。脚本 CPU 采样原生 944.1–1067.3%，Hyper
1014.9–1121.3%，不能仅凭占用率推导瓶颈。

## 正确性及异常

两个入口的共享单元测试各 2 项通过。`preflight.json` 验证真实 socket
的响应状态、HTTP 版本、响应头（排除变化的 Date 值）和 body 相同，覆盖
负数、编码查询键、定长及拆成两块的 chunked 请求，同一连接连续请求。
双方进程中的 io_uring 描述符已记录。十二轮均报告 2xx，3xx/4xx/5xx 为零。

**不隐藏的计数异常：** B2 第一轮第三模板 Per-template 为 1,119,626，
Per-template-ok 为 1,119,627，成功数比完成数多 1。原因未定位，不能
将这两个计数声称为完全匹配，也不能直接将其解释为服务端响应错误。
原日志保留；summarize.py 显式输出 counter_anomalies，不删掉该轮数据。
本轮验证不替代完整协议一致性、安全性或长时间稳定性测试。

## 复跑

```sh
cd geario-http/bench-http
# 需要复现本次依赖时使用随报告保存的 Cargo.lock.snapshot。
RUSTFLAGS='-C target-cpu=native' cargo build --locked --release \
  -p server-geario -p server-hyper-on-geario \
  --bin arena-baseline --bin arena-hyper \
  --features server-geario/uring,server-hyper-on-geario/uring
```

run.py 依赖之前双机实验的受限 SSH 代理、原始 Arena 目录和请求模板，
不是通用部署脚本。summarize.py 可直接离线复算本目录日志。
测试容器已停止，临时 sysctl、压测公钥授权和防火墙表已恢复/删除。
只增加比较入口，没有切换正式引擎，也没有发布版本。

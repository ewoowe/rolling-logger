# rolling-logger

生产级滚动文件日志库，核心滚动写入器是框架无关的 `io::Write` 实现，可对接 [`tracing`](https://docs.rs/tracing)、[`log`](https://docs.rs/log) 与 [`slog`](https://docs.rs/slog) 三大主流日志门面。

> 本文档为中文版，英文版见 [README.md](README.md)。

## 特性

- **按日期 + 按大小双重滚动**：文件名形如 `{prefix}.{YYYY-MM-DD}.log`，大小超限时追加序号 `{seq}`。
- **历史日志自动 gzip 压缩归档**到 `history/` 目录：异步、限并发、原子写、崩溃安全。
- **归档等待天数可配置**：负数 = 滚动即归档；`0` = 昨天及更早归档；`1` = 前天及更早归档，依此类推。
- **可配置日志时区**（IANA 名，如 `"UTC"` / `"Asia/Shanghai"`），支持跨时区部署。
- **可选 fsync 强持久化**：`flush` 时强制落盘，崩溃不丢日志。
- **丢日志计数监控**：非阻塞写入因 channel 满而丢弃的行数可查询。
- **优雅关闭**：`LoggerGuard` drop 时自动 flush 缓冲区并等待归档线程完成。
- **同时输出控制台与文件**：控制台带 ANSI 颜色，文件为纯文本。
- **门面无关日志宏**：`trace!` / `debug!` / `info!` / `warn!` / `error!` 五个宏，按启用的门面自动代理，跨 `tracing`、`log`、`slog` 三门面业务代码保持一致。

## 安装

在 `Cargo.toml` 中添加：

```toml
[dependencies]
rolling-logger = "0.3"
```

## 门面支持（feature）

| feature | 默认 | 底层门面 |
| --- | --- | --- |
| `tracing` | ✅ | [`tracing`](https://docs.rs/tracing)（`tracing-subscriber`） |
| `log-backend` | ❌ | [`log`](https://docs.rs/log) |
| `slog-backend` | ❌ | [`slog`](https://docs.rs/slog) |

三个门面**互斥**，只能启用其一（同时启用会在编译期报错）。核心滚动写入器
`RollingFileWriter` 是框架无关的 `io::Write`，三个门面复用同一套滚动/归档能力。

无论选择哪个门面，初始化都调用同一个入口 `init`：

```rust
use rolling_logger::{init, LogConfig};

let config = LogConfig { /* ... */ };
let _guard = init(&config)?;
```

## 快速开始

```rust
use rolling_logger::{init, LogConfig};

fn main() -> anyhow::Result<()> {
    let config = LogConfig {
        dir: "./logs".into(),
        level: "info".into(),
        file_prefix: "app".into(),
        max_file_size_mb: 10,
        max_files: 30,
        archive_delay_days: 0,
        archive_batch_size: 100,
        fsync_on_flush: false,
        timezone: "UTC".into(),
    };

    // 统一的初始化入口，底层门面由 feature 决定（默认 tracing）
    let _guard = init(&config)?;

    tracing::info!("hello, rolling-logger!");
    tracing::warn!("this goes to both console and file");

    Ok(())
}
```

对接 `log` 门面（禁用默认 tracing）：

```toml
[dependencies]
rolling-logger = { version = "0.3", default-features = false, features = ["log-backend"] }
```

```rust
use rolling_logger::{init, LogConfig};

let config = LogConfig { /* ... */ };
let _guard = init(&config)?;
log::info!("hello via log facade");
```

对接 `slog` 门面（禁用默认 tracing）：

```toml
[dependencies]
rolling-logger = { version = "0.3", default-features = false, features = ["slog-backend"] }
```

```rust
use rolling_logger::{init, LogConfig};

let config = LogConfig { /* ... */ };
let guard = init(&config)?;
let log = guard.logger();   // slog 宏需显式传入 logger
slog::info!(log, "hello via slog"; "user_id" => 42);
```

完整的可运行示例见 [`examples/tracing.rs`](examples/tracing.rs)（tracing 门面）、[`examples/log.rs`](examples/log.rs)（log 门面）与 [`examples/slog.rs`](examples/slog.rs)（slog 门面）。

## 门面无关日志宏

本 crate 提供 5 个门面无关宏，按编译期启用的 feature 自动代理到对应门面，
跨 `tracing`、`log`、`slog` 三门面业务代码保持一致：

```rust
use rolling_logger::{debug, error, info, trace, warn};

trace!("trace 级别");
debug!("debug 级别，变量值 {}", x);
info!("info 级别");
warn!("warn 级别");
error!("error 级别");
```

代理规则：

| 启用 feature | 宏代理到 |
| --- | --- |
| `tracing`（默认） | `tracing::trace!` 等 |
| `log-backend` | `log::trace!` 等 |
| `slog-backend` | `slog::trace!` 等（自动注入全局 logger） |
| 无门面 | no-op（参数不求值、零开销） |

### 门面原生宏

门面无关宏与各门面自身的原生宏可在同一程序中并存。需要用到门面特有功能时，
直接使用门面原生 API：

```rust
// tracing —— 结构化字段和 span
tracing::info!(answer = 42, "结构化字段");
tracing::warn!(code = 1001, "带字段的警告");

// log —— 普通位置参数
log::info!("hello via log");
log::warn!(target: "my_component", "带 target 的警告");

// slog —— 结构化 key-value（需显式传入 logger）
let log = guard.logger();
slog::info!(log, "hello via slog"; "user_id" => 42);
```

> **边界**：门面无关宏只支持**公共子集**语法——位置参数（`info!("msg {}", x)`）。
> 各门面特有的语法需使用原生宏：
> - `tracing` 结构化字段：`tracing::info!(field = v, "msg")`
> - `slog` 结构化 key-value：`slog::info!(logger, "msg"; "k" => v)`
>
> （`target:` 语法在 `tracing` / `log` 下可用，但 `slog` 下不支持。）

## 配置说明

| 字段 | 类型 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `dir` | `String` | — | 日志文件存储目录 |
| `level` | `String` | — | 日志级别过滤规则，如 `"info,my_crate=debug"` |
| `file_prefix` | `String` | — | 日志文件名前缀 |
| `max_file_size_mb` | `u64` | — | 单个日志文件最大大小（MB） |
| `max_files` | `usize` | — | 最多保留多少个归档文件（`0` 不限制） |
| `archive_delay_days` | `i64` | `0` | 归档等待天数，负数 = 滚动即归档 |
| `archive_batch_size` | `usize` | `100` | 单次归档最多处理文件数 |
| `fsync_on_flush` | `bool` | `false` | flush 时是否强制 fsync 落盘 |
| `timezone` | `String` | `"UTC"` | 日志时间戳时区（IANA 名） |

## 目录结构

```
logs/
├── app.2026-08-28.log          # 历史日志（归档后移至 history/）
├── app.2026-08-29.log          # 当前日志
└── history/
    ├── app.2026-08-28.log.gz   # 压缩归档
    └── ...
```

## 高级用法

### 自定义日志级别过滤

`level` 字段在 `tracing` 门面下遵循 `tracing_subscriber::EnvFilter` 语法：

```rust
let config = LogConfig {
    level: "info,my_app=debug,hyper=warn".into(),
    // ...其余字段
};
```

> 在 `log` / `slog` 门面下，它们只支持单一全局级别，会取 `level` 的第一个 token
> （如上例中的 `"info"`），后续的 per-target 规则被忽略。

### 监控丢日志

```rust
let guard = init(&config)?;
// ...运行一段时间后
eprintln!("dropped file lines: {}", guard.dropped_file_lines());
```

> 仅 `tracing` 门面提供 `dropped_file_lines()`（`log` / `slog` 门面是同步写入，
> 无丢日志通道）。

### 手动解析时区

```rust
use rolling_logger::parse_timezone;

let tz = parse_timezone("Asia/Shanghai"); // 失败回退 UTC
```

## License

[MIT](LICENSE)

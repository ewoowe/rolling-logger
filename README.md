# rolling-logger

生产级滚动文件日志库，基于 [`tracing`](https://docs.rs/tracing) 生态构建，可被任何 Rust 项目直接引入复用。

## 特性

- **按日期 + 按大小双重滚动**：文件名形如 `{prefix}.{YYYY-MM-DD}.log`，大小超限时追加序号 `{seq}`。
- **历史日志自动 gzip 压缩归档**到 `history/` 目录：异步、限并发、原子写、崩溃安全。
- **归档等待天数可配置**：负数 = 滚动即归档；`0` = 昨天及更早归档；`1` = 前天及更早归档，依此类推。
- **可配置日志时区**（IANA 名，如 `"UTC"` / `"Asia/Shanghai"`），支持跨时区部署。
- **可选 fsync 强持久化**：`flush` 时强制落盘，崩溃不丢日志。
- **丢日志计数监控**：非阻塞写入因 channel 满而丢弃的行数可查询。
- **优雅关闭**：`LoggerGuards` drop 时自动 flush 缓冲区并等待归档线程完成。
- **同时输出控制台与文件**：控制台带 ANSI 颜色，文件为纯文本。

## 安装

在 `Cargo.toml` 中添加：

```toml
[dependencies]
rolling-logger = "0.1"
```

## 快速开始

```rust
use rolling_logger::{init_logger, LogConfig};

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

    // 初始化日志系统，返回值必须保持存活直到程序退出
    let _guard = init_logger(&config)?;

    tracing::info!("hello, rolling-logger!");
    tracing::warn!("this goes to both console and file");

    Ok(())
}
```

完整的可运行示例见 [`examples/basic.rs`](examples/basic.rs)。

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

`level` 字段遵循 `tracing_subscriber::EnvFilter` 语法：

```rust
let config = LogConfig {
    level: "info,my_app=debug,hyper=warn".into(),
    // ...其余字段
};
```

### 监控丢日志

```rust
let guard = init_logger(&config)?;
// ...运行一段时间后
eprintln!("dropped file lines: {}", guard.dropped_file_lines());
```

### 手动解析时区

```rust
use rolling_logger::parse_timezone;

let tz = parse_timezone("Asia/Shanghai"); // 失败回退 UTC
```

## License

[MIT](LICENSE)

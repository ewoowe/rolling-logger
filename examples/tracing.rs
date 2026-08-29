//! 滚动文件日志库的 tracing 门面示例（默认 feature）。
//!
//! 运行方式：
//! ```sh
//! cargo run --example tracing
//! ```

use rolling_logger::{debug, error, info, init, trace, warn, LogConfig};

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

    // 统一的初始化入口：底层门面由编译期 feature 决定（默认 tracing）。
    // 返回的 guard 必须保持存活到程序退出，否则非阻塞写入缓冲中的日志会在 drop 前被丢弃。
    let guard = init(&config)?;

    // 门面无关日志宏：在本示例（tracing 门面）下代理到 tracing::*!。
    // 若改用 log 门面（--no-default-features --features log-backend），
    // 这些宏会代理到 log::*!，业务代码无需任何改动（见 log.rs）。
    trace!("trace 级别（被 info 过滤）");
    debug!("debug 级别（被 info 过滤）");
    info!("hello, rolling-logger!");
    warn!("warning goes to both console and file");
    error!("error also goes to both");

    // 门面无关宏也支持 target 语法（两个门面的公共子集）
    info!(target: "my_component", "带 target 的日志");

    // 需要 tracing 特有功能（如结构化字段）时，直接使用原生宏即可
    tracing::info!(answer = 42, "tracing 结构化字段示例");

    // 查询文件日志因 channel 满而被丢弃的行数（仅 tracing 门面提供）
    let _dropped = guard.dropped_file_lines();

    Ok(())
}

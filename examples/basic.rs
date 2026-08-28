//! 滚动文件日志库的最简用法示例。
//!
//! 运行方式：
//! ```sh
//! cargo run --example basic
//! ```

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

    // 初始化日志系统。返回的 guard 必须保持存活到程序退出，
    // 否则非阻塞写入缓冲中的日志会在 drop 前被丢弃。
    let guard = init_logger(&config)?;

    tracing::info!("hello, rolling-logger!");
    tracing::debug!("this line is hidden by level filter 'info'");
    tracing::warn!("warning goes to both console and file");
    tracing::error!("error also goes to both");

    // 主动 flush（fsync_on_flush=true 时此处会强制落盘）
    // guard 在 main 返回时 drop，会自动 flush 并等待归档线程完成。

    // 查询文件日志因 channel 满而被丢弃的行数（监控用）
    let _dropped = guard.dropped_file_lines();

    Ok(())
}

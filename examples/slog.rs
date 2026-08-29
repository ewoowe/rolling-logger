//! 对接 `slog` 门面的示例。
//!
//! 运行方式（需启用 slog-backend feature，并禁用默认 tracing）：
//! ```sh
//! cargo run --example slog --no-default-features --features slog-backend
//! ```
//!
//! 注意：slog 的宏需要显式传入 logger 实例，且消息采用结构化 key-value 语法，
//! 因此**不纳入门面无关宏（`facade`）体系**，直接使用 slog 原生宏。

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

    // 统一的初始化入口：启用 slog-backend（且未启用其它门面）时底层为 slog 门面
    let guard = init(&config)?;

    // 通过 guard 获取 slog 的 Logger（Arc，clone 便宜）
    let log = guard.logger();

    // slog 原生宏：消息 + 结构化 key-value
    slog::info!(log, "hello, slog backend!");
    slog::debug!(log, "this line is hidden by level filter 'info'");
    slog::warn!(log, "warning via slog"; "code" => 1001);
    slog::error!(log, "error via slog"; "user_id" => 42, "action" => "login");

    Ok(())
}

//! 对接 `log` 门面的示例。
//!
//! 运行方式（需启用 log-backend feature，并禁用默认 tracing）：
//! ```sh
//! cargo run --example log --no-default-features --features log-backend
//! ```
//!
//! 注意：本示例与 `tracing.rs` 使用**完全相同**的门面无关日志宏，只是编译时切换了
//! feature。这体现了门面无关宏的价值——业务日志代码不依赖具体门面。

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

    // 统一的初始化入口：启用 log-backend（且未启用 tracing）时底层为 log 门面
    let _guard = init(&config)?;

    // 门面无关日志宏：在本示例（log 门面）下代理到 log::*!，
    // 无需引入 `log` crate 直接依赖（宏内部通过 $crate 定位门面）。
    trace!("trace 级别（被 info 过滤）");
    debug!("debug 级别（被 info 过滤）");
    info!("hello, log backend!");
    warn!("warning via log backend");
    error!("error via log backend");

    // 门面无关宏也支持 target 语法
    info!(target: "my_component", "带 target 的日志");

    Ok(())
}

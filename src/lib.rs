//! rolling-logger：可复用的滚动文件日志库
//!
//! 提供一套生产级的滚动文件日志能力，可被任何项目直接引入复用：
//!
//! - 按日期 + 按大小双重滚动的日志文件写入器
//! - 历史日志自动 gzip 压缩归档到 `history/` 目录（异步、限并发、原子写、崩溃安全）
//! - 归档等待天数可配置（负数 = 滚动即归档）
//! - 可配置日志时区（IANA 名，跨时区部署）
//! - 可选 fsync 强持久化、丢日志计数监控、优雅关闭
//! - 门面可选：默认对接 [`tracing`] 生态，另可通过 `log-backend` feature 对接 [`log`] 生态
//!
//! # 门面（feature）
//!
//! 核心滚动写入器 [`RollingFileWriter`] 是框架无关的 `io::Write` 实现，可被任何
//! 日志后端复用。本 crate 为两个主流门面提供了开箱即用的初始化，**二者互斥**，
//! 只能启用其一：
//!
//! | feature | 默认 | 底层门面 |
//! | --- | --- | --- |
//! | `tracing` | ✅ | [`tracing`]（`tracing-subscriber`） |
//! | `log-backend` | ❌ | [`log`] |
//!
//! 无论选择哪个门面，初始化都调用同一个入口 [`init`]：
//!
//! ```ignore
//! use rolling_logger::{init, LogConfig};
//!
//! let config = LogConfig {
//!     dir: "./logs".into(),
//!     level: "info".into(),
//!     file_prefix: "app".into(),
//!     max_file_size_mb: 10,
//!     max_files: 30,
//!     archive_delay_days: 0,
//!     archive_batch_size: 100,
//!     fsync_on_flush: false,
//!     timezone: "UTC".into(),
//! };
//! let guard = init(&config)?;   // 绑定到 main 作用域保持存活
//!
//! // tracing 门面（默认）：
//! tracing::info!("hello via tracing");
//! // log 门面（`--no-default-features --features log-backend`）：
//! // log::info!("hello via log");
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! 若需要显式控制（不通过统一入口），也可直接使用底层原语 [`init_logger`] /
//! [`init_log_logger`]。

// 严格二选一：tracing 与 log-backend 互斥，同时启用直接编译报错
#[cfg(all(feature = "tracing", feature = "log-backend"))]
compile_error!(
    "features `tracing` and `log-backend` are mutually exclusive: \
     enable exactly one of them"
);

mod config;
mod writer;
#[doc(hidden)]
pub mod facade;
#[cfg(feature = "tracing")]
mod tracing_layer;
#[cfg(feature = "log-backend")]
mod log_layer;

pub use config::LogConfig;
pub use writer::{parse_timezone, shutdown_archivers, RollingFileWriter};
#[cfg(feature = "tracing")]
pub use tracing_layer::{init_logger, LoggerGuards};
#[cfg(feature = "log-backend")]
pub use log_layer::init_log_logger;

/// 统一日志守卫：无论底层是哪个门面，`init` 都返回此类型。
///
/// 必须保持存活到程序结束（绑定到 `main` 作用域），否则：
/// - tracing 门面下，非阻塞写入缓冲会在 drop 前丢失；
/// - 任一门面下，drop 时都会优雅关闭归档线程。
pub struct LoggerGuard {
    #[cfg(feature = "tracing")]
    tracing: LoggerGuards,
}

impl Drop for LoggerGuard {
    fn drop(&mut self) {
        // 优雅关闭归档线程（幂等；tracing 门面下 LoggerGuards 的 Drop 也会调用）
        writer::shutdown_archivers();
    }
}

impl LoggerGuard {
    /// 返回文件日志因 channel 满而被丢弃的行数（仅 tracing 门面，供监控/告警）
    #[cfg(feature = "tracing")]
    pub fn dropped_file_lines(&self) -> usize {
        self.tracing.dropped_file_lines()
    }
}

/// 统一的日志系统初始化入口
///
/// 底层门面由编译期 feature 决定（`tracing` 或 `log-backend`，二者互斥）。
/// 返回 [`LoggerGuard`]，必须保持存活到程序结束。
pub fn init(config: &LogConfig) -> anyhow::Result<LoggerGuard> {
    #[cfg(feature = "tracing")]
    {
        Ok(LoggerGuard {
            tracing: init_logger(config)?,
        })
    }
    #[cfg(all(feature = "log-backend", not(feature = "tracing")))]
    {
        init_log_logger(config)?;
        Ok(LoggerGuard {})
    }
    #[cfg(not(any(feature = "tracing", feature = "log-backend")))]
    {
        // 未启用任何门面 feature：核心写入器仍可用，但无门面初始化能力
        let _ = config;
        anyhow::bail!("no logging facade feature enabled (enable `tracing` or `log-backend`)")
    }
}

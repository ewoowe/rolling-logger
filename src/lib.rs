//! rolling-logger：可复用的滚动文件日志库
//!
//! 提供一套生产级的滚动文件日志能力，可被任何项目直接引入复用：
//!
//! - 按日期 + 按大小双重滚动的日志文件写入器
//! - 历史日志自动 gzip 压缩归档到 `history/` 目录（异步、限并发、原子写、崩溃安全）
//! - 归档等待天数可配置（负数 = 滚动即归档）
//! - 可配置日志时区（IANA 名，跨时区部署）
//! - 可选 fsync 强持久化、丢日志计数监控、优雅关闭
//!
//! # 用法
//!
//! ```ignore
//! use rolling_logger::{init_logger, parse_timezone, LogConfig};
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
//! let guard = init_logger(&config)?;   // 绑定到 main 作用域保持存活
//! # Ok::<(), anyhow::Error>(())
//! ```

mod config;
mod writer;

pub use config::LogConfig;
pub use writer::{
    init_logger, parse_timezone, shutdown_archivers, LoggerGuards, RollingFileWriter,
};

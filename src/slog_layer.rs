//! slog 门面集成：`init_slog_logger`
//!
//! 仅在启用 `slog-backend` feature 时编译。将 [`RollingFileWriter`] 包装为
//! slog 的 [`Drain`]，复用同一套滚动/归档能力。
//!
//! 注意：slog 的宏（`slog::info!(logger, "msg"; "k" => v)`）需要显式传入
//! logger 实例，且消息采用结构化 key-value 语法，与 `log`/`tracing` 的
//! `info!("msg {}", x)` 位置参数语法不兼容，因此 **slog 不纳入 `facade` 宏体系**，
//! 请直接使用 slog 原生宏。
//!
//! [`RollingFileWriter`]: crate::RollingFileWriter
//! [`Drain`]: slog::Drain

use std::io::Write;
use std::sync::Mutex;

use chrono_tz::Tz;

use crate::config::LogConfig;
use crate::writer::{now_in, parse_timezone, RollingFileWriter};

/// 面向 slog 的滚动文件 Drain
///
/// 内部用 `Mutex<RollingFileWriter>` 获得 `Send + Sync`，从而满足 `Logger::root`
/// 对 `Drain: Send + Sync + 'static` 的要求。`Err = Never` 表示写入永不失败
/// （IO 错误被吞掉并丢弃该条日志）。
struct RollingDrain {
    writer: Mutex<RollingFileWriter>,
    tz: Tz,
    level: slog::Level,
}

impl slog::Drain for RollingDrain {
    type Ok = ();
    type Err = slog::Never;

    /// 运行时级别过滤：低于配置级别的日志直接跳过
    fn is_enabled(&self, level: slog::Level) -> bool {
        level <= self.level
    }

    fn log(
        &self,
        record: &slog::Record<'_>,
        _values: &slog::OwnedKVList,
    ) -> Result<Self::Ok, Self::Err> {
        // slog 的宏直接调 `Logger::log`（不经过 `is_enabled`），
        // 因此运行时级别过滤必须在此手动判断。
        if !self.is_enabled(record.level()) {
            return Ok(());
        }
        // 结构化 key-value（`_values`）暂不序列化，仅输出时间/级别/模块/消息。
        // 若需要完整结构化输出，可在后续扩展为 JSON 等序列化格式。
        let mut w = self.writer.lock().unwrap();
        let _ = writeln!(
            w,
            "{} [{}] {} - {}",
            now_in(self.tz).format("%Y-%m-%d %H:%M:%S%.3f"),
            record.level(),
            record.module(),
            record.msg()
        );
        Ok(())
    }
}

/// 将 `config.level` 解析为 slog 的 [`Level`]
///
/// `config.level` 遵循 `EnvFilter` 语法（如 `"info,my_crate=debug"`），
/// 而 slog 只支持单一全局级别，这里取第一个 token，解析失败回退 `Info`。
///
/// [`Level`]: slog::Level
fn parse_level(level: &str) -> slog::Level {
    level
        .split(',')
        .next()
        .and_then(|s| s.trim().parse::<slog::Level>().ok())
        .unwrap_or(slog::Level::Info)
}

/// 初始化 slog 门面日志系统
///
/// 返回 [`slog::Logger`]，调用方直接持有并传给 slog 宏：
///
/// ```ignore
/// let log = init_slog_logger(&config)?;
/// slog::info!(log, "hello"; "user_id" => 42);
/// ```
///
/// 说明：
/// - slog 不是全局单例模型（宏需显式传 logger），因此本函数返回 logger 本身；
/// - 推荐通过统一入口 [`init`](crate::init) 初始化，再用
///   [`LoggerGuard::logger`](crate::LoggerGuard::logger) 获取 logger，以便
///   guard 在 drop 时优雅关闭归档线程。若直接使用本函数，需自行调用
///   [`shutdown_archivers`](crate::shutdown_archivers)。
pub fn init_slog_logger(config: &LogConfig) -> anyhow::Result<slog::Logger> {
    let tz = parse_timezone(&config.timezone);
    let writer = RollingFileWriter::new(
        &config.dir,
        &config.file_prefix,
        config.max_file_size_mb,
        config.max_files,
        config.archive_delay_days,
        config.archive_batch_size,
        config.fsync_on_flush,
        tz,
    )?;

    let drain = RollingDrain {
        writer: Mutex::new(writer),
        tz,
        level: parse_level(&config.level),
    };

    // Logger::root 要求 Drain 的 Err = Never、Ok = ()，我们的 RollingDrain 直接满足
    let logger = slog::Logger::root(drain, slog::o!());
    Ok(logger)
}

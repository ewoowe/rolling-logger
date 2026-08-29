//! log 门面集成：`init_log_logger`
//!
//! 仅在启用 `log-backend` feature 时编译。将 [`RollingFileWriter`] 包装为
//! 全局 `log` 门面 logger，复用同一套滚动/归档能力。
//!
//! [`RollingFileWriter`]: crate::RollingFileWriter

use std::io::Write;
use std::sync::Mutex;

use chrono_tz::Tz;
use log::{LevelFilter, Log, Metadata, Record};

use crate::config::LogConfig;
use crate::writer::{now_in, parse_timezone, RollingFileWriter};

/// 面向 `log` 门面的滚动文件 logger
///
/// 内部用 `Mutex<RollingFileWriter>` 获得 `Send + Sync`，从而可注册为全局 logger。
struct RollingLog {
    writer: Mutex<RollingFileWriter>,
    level: LevelFilter,
    tz: Tz,
}

impl Log for RollingLog {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let mut w = self.writer.lock().unwrap();
        let _ = writeln!(
            w,
            "{} [{:5}] {} - {}",
            now_in(self.tz).format("%Y-%m-%d %H:%M:%S%.3f"),
            record.level(),
            record.target(),
            record.args()
        );
    }

    fn flush(&self) {
        let _ = self.writer.lock().unwrap().flush();
    }
}

/// 将 `config.level` 解析为 log 的 [`LevelFilter`]
///
/// `config.level` 遵循 `EnvFilter` 语法（如 `"info,my_crate=debug"`），
/// 而 `log` 门面只支持单个全局级别，这里取第一个 token，解析失败回退 `Info`。
fn parse_level(level: &str) -> LevelFilter {
    level
        .split(',')
        .next()
        .and_then(|s| s.trim().parse::<LevelFilter>().ok())
        .unwrap_or(LevelFilter::Info)
}

/// 初始化 `log` 门面日志系统
///
/// 注册全局 logger（同一进程只能调用一次），日志写入滚动文件。
/// 如需控制台输出，可自行额外配置 `log` 门面的终端后端（如 `env_logger`）。
pub fn init_log_logger(config: &LogConfig) -> anyhow::Result<()> {
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

    let logger = RollingLog {
        writer: Mutex::new(writer),
        level: parse_level(&config.level),
        tz,
    };

    log::set_boxed_logger(Box::new(logger))?;
    log::set_max_level(LevelFilter::Trace);
    Ok(())
}

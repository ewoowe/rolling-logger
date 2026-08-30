//! tracing 门面集成：`init_logger` 与 `LoggerGuards`
//!
//! 仅在启用 `tracing` feature 时编译。基于 `tracing-subscriber` +
//! `tracing-appender` 构建「控制台 + 滚动文件」双输出层。

use std::io;

use chrono_tz::Tz;
use tracing_appender::non_blocking::{ErrorCounter, WorkerGuard};
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::fmt::{self, format::Writer};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::config::LogConfig;
use crate::writer::{now_in, parse_timezone, shutdown_archivers, RollingFileWriter};

/// 自定义时间格式器（使用配置时区，精确到毫秒）
struct TzTimer(Tz);

impl FormatTime for TzTimer {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", now_in(self.0).format("%Y-%m-%d %H:%M:%S%.3f"))
    }
}

/// tracing 门面的内部守卫，持有控制台和文件两个非阻塞写入器的 guard。
///
/// 仅供 [`crate::LoggerGuard`] 内部持有；在 drop 时会 flush 缓冲区。
/// 不对外暴露——统一由 [`crate::LoggerGuard`] 封装并提供优雅关闭。
pub(crate) struct LoggerGuards {
    /// 控制台写入守卫（仅用于持有以保持生命周期，drop 时 flush）
    #[allow(dead_code)]
    console: WorkerGuard,
    /// 文件写入守卫（仅用于持有以保持生命周期，drop 时 flush）
    #[allow(dead_code)]
    file: WorkerGuard,
    /// 文件日志因 channel 满而被丢弃的行数计数器
    file_error_counter: ErrorCounter,
}

impl LoggerGuards {
    /// 返回文件日志因 channel 满而被丢弃的行数（供监控/告警）
    pub fn dropped_file_lines(&self) -> usize {
        self.file_error_counter.dropped_lines()
    }
}

impl Drop for LoggerGuards {
    fn drop(&mut self) {
        // 先等待归档线程完成（优雅关闭），再让字段 drop 时 flush 日志
        shutdown_archivers();
    }
}

/// 初始化 tracing 日志系统
///
/// 返回 [`crate::LoggerGuard`]，必须在程序生命周期内保持存活，否则非阻塞写入的日志会丢失。
/// 建议将返回值绑定到 `main` 函数的变量中。
pub fn init_logger(config: &LogConfig) -> anyhow::Result<crate::LoggerGuard> {
    // 解析时区（失败回退 UTC）
    let tz = parse_timezone(&config.timezone);

    // 创建滚动文件写入器
    let rolling_writer = RollingFileWriter::new(
        &config.dir,
        &config.file_prefix,
        config.max_file_size_mb,
        config.max_files,
        config.archive_delay_days,
        config.archive_batch_size,
        config.fsync_on_flush,
        tz,
    )?;

    // 创建非阻塞写入器（控制台 + 文件）
    let (console_writer, console_guard) = tracing_appender::non_blocking(io::stdout());
    let (file_writer, file_guard) = tracing_appender::non_blocking(rolling_writer);
    // 保存丢日志计数器（file_writer 稍后被 with_writer 消费，需先 clone 出来）
    let file_error_counter = file_writer.error_counter();

    // 构建日志级别过滤器
    let env_filter = EnvFilter::try_new(&config.level)
        .unwrap_or_else(|_| EnvFilter::new("info"));

    // 构建控制台日志层（带颜色）
    let console_layer = fmt::layer()
        .with_writer(console_writer)
        .with_ansi(true)
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_file(true)
        .with_line_number(true)
        .with_timer(TzTimer(tz));

    // 构建文件日志层（无颜色，纯文本）
    let file_layer = fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_file(true)
        .with_line_number(true)
        .with_timer(TzTimer(tz));

    // 初始化全局日志订阅器
    tracing_subscriber::registry()
        .with(env_filter)
        .with(console_layer)
        .with(file_layer)
        .init();

    Ok(crate::LoggerGuard {
        tracing: LoggerGuards {
            console: console_guard,
            file: file_guard,
            file_error_counter,
        },
    })
}

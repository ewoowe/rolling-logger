//! tracing facade integration: `init_logger` and `LoggerGuards`.
//!
//! Compiled only when the `tracing` feature is enabled. Builds a dual output
//! layer ("console + rolling file") on top of `tracing-subscriber` and
//! `tracing-appender`.

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

/// Custom time formatter (configured timezone, millisecond precision).
struct TzTimer(Tz);

impl FormatTime for TzTimer {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", now_in(self.0).format("%Y-%m-%d %H:%M:%S%.3f"))
    }
}

/// Internal guard for the tracing facade, holding the console and file
/// non-blocking writer guards.
///
/// Held only by [`crate::LoggerGuard`]; flushes buffers on drop. Not exposed
/// publicly — the unified [`crate::LoggerGuard`] wraps it and provides graceful
/// shutdown.
pub(crate) struct LoggerGuards {
    /// Console writer guard (held only to keep it alive; flushes on drop).
    #[allow(dead_code)]
    console: WorkerGuard,
    /// File writer guard (held only to keep it alive; flushes on drop).
    #[allow(dead_code)]
    file: WorkerGuard,
    /// Counter for file log lines dropped because the channel was full.
    file_error_counter: ErrorCounter,
}

impl LoggerGuards {
    /// Returns the number of file log lines dropped (for monitoring/alerting).
    pub fn dropped_file_lines(&self) -> usize {
        self.file_error_counter.dropped_lines()
    }
}

impl Drop for LoggerGuards {
    fn drop(&mut self) {
        // Wait for archiver threads first (graceful shutdown), then let the
        // fields flush logs on drop.
        shutdown_archivers();
    }
}

/// Initialize the tracing logging system.
///
/// Returns a [`crate::LoggerGuard`] that must stay alive for the program's
/// lifetime, otherwise non-blocking written logs would be lost. Bind the return
/// value to a variable in the `main` function.
pub fn init_logger(config: &LogConfig) -> anyhow::Result<crate::LoggerGuard> {
    // Parse timezone (falls back to UTC on failure).
    let tz = parse_timezone(&config.timezone);

    // Create the rolling file writer.
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

    // Create non-blocking writers (console + file).
    let (console_writer, console_guard) = tracing_appender::non_blocking(io::stdout());
    let (file_writer, file_guard) = tracing_appender::non_blocking(rolling_writer);
    // Save the drop counter before `file_writer` is consumed by `with_writer`.
    let file_error_counter = file_writer.error_counter();

    // Build the log level filter.
    let env_filter = EnvFilter::try_new(&config.level)
        .unwrap_or_else(|_| EnvFilter::new("info"));

    // Console layer (with color).
    let console_layer = fmt::layer()
        .with_writer(console_writer)
        .with_ansi(true)
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_file(true)
        .with_line_number(true)
        .with_timer(TzTimer(tz));

    // File layer (no color, plain text).
    let file_layer = fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_file(true)
        .with_line_number(true)
        .with_timer(TzTimer(tz));

    // Initialize the global subscriber.
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

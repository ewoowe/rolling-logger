//! log facade integration: `init_log_logger`.
//!
//! Compiled only when the `log-backend` feature is enabled. Wraps
//! [`RollingFileWriter`] as a global `log` facade logger, reusing the same
//! rolling/archival capability.
//!
//! [`RollingFileWriter`]: crate::RollingFileWriter

use std::io::Write;
use std::sync::Mutex;

use chrono_tz::Tz;
use log::{LevelFilter, Log, Metadata, Record};

use crate::config::LogConfig;
use crate::writer::{now_in, parse_timezone, RollingFileWriter};

/// ANSI reset code.
const RESET: &str = "\x1b[0m";

/// Returns the ANSI color code for a log level (console output).
fn level_color(level: log::Level) -> &'static str {
    match level {
        log::Level::Error => "\x1b[31m", // red
        log::Level::Warn => "\x1b[33m",  // yellow
        log::Level::Info => "\x1b[32m",  // green
        log::Level::Debug => "\x1b[34m", // blue
        log::Level::Trace => "\x1b[35m", // magenta
    }
}

/// Rolling file logger for the `log` facade.
///
/// Uses `Mutex<RollingFileWriter>` to obtain `Send + Sync`, so it can be
/// registered as the global logger.
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
        let ts = now_in(self.tz).format("%Y-%m-%d %H:%M:%S%.3f").to_string();

        // Console (colored).
        let color = level_color(record.level());
        println!(
            "{} {}{:5}{} {} - {}",
            ts,
            color,
            record.level(),
            RESET,
            record.target(),
            record.args()
        );

        // File (plain text).
        let mut w = self.writer.lock().unwrap();
        let _ = writeln!(
            w,
            "{} [{:5}] {} - {}",
            ts,
            record.level(),
            record.target(),
            record.args()
        );
    }

    fn flush(&self) {
        let _ = self.writer.lock().unwrap().flush();
    }
}

/// Parses `config.level` into a log [`LevelFilter`].
///
/// `config.level` follows `EnvFilter` syntax (e.g. "info,my_crate=debug"), but
/// the `log` facade only supports a single global level, so this takes the first
/// token and falls back to `Info` on failure.
fn parse_level(level: &str) -> LevelFilter {
    level
        .split(',')
        .next()
        .and_then(|s| s.trim().parse::<LevelFilter>().ok())
        .unwrap_or(LevelFilter::Info)
}

/// Initialize the `log` facade logging system.
///
/// Registers the global logger (callable once per process), outputting to both
/// console (colored) and rolling file. Returns a [`crate::LoggerGuard`] that must
/// stay alive until the program exits; dropping it gracefully shuts down the
/// archiver threads.
pub fn init_log_logger(config: &LogConfig) -> anyhow::Result<crate::LoggerGuard> {
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
    Ok(crate::LoggerGuard {})
}

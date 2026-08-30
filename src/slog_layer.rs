//! slog facade integration: `init_slog_logger`.
//!
//! Compiled only when the `slog-backend` feature is enabled. Wraps
//! [`RollingFileWriter`] as a slog [`Drain`], reusing the same rolling/archival
//! capability.
//!
//! Note: slog macros (`slog::info!(logger, "msg"; "k" => v)`) require an
//! explicit logger argument and use structured key-value syntax, which is
//! incompatible with the positional-argument syntax of `log`/`tracing`
//! (`info!("msg {}", x)`). Therefore slog is **not** part of the `facade` macro
//! layer; use slog's native macros directly.
//!
//! [`RollingFileWriter`]: crate::RollingFileWriter
//! [`Drain`]: slog::Drain

use std::fmt;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

use chrono_tz::Tz;
use slog::KV;

use crate::config::LogConfig;
use crate::writer::{now_in, parse_timezone, RollingFileWriter};

/// Global slog `Logger`, set once during [`init_slog_logger`].
///
/// This is what makes the facade-agnostic macros (`rolling_logger::info!` etc.)
/// work under `slog-backend`: the macros auto-inject this logger, so callers can
/// use the same positional-argument syntax as `log`/`tracing` without passing a
/// logger explicitly.
static GLOBAL_SLOG_LOGGER: OnceLock<slog::Logger> = OnceLock::new();

/// Returns the global slog `Logger` set by [`init_slog_logger`].
///
/// Before initialization (or if initialization failed to register a logger),
/// falls back to a discard logger that drops everything, so the facade macros
/// remain safe to call at any point.
pub fn global_slog_logger() -> slog::Logger {
    GLOBAL_SLOG_LOGGER
        .get()
        .cloned()
        .unwrap_or_else(|| slog::Logger::root(slog::Discard, slog::o!()))
}

/// ANSI reset code.
const RESET: &str = "\x1b[0m";

/// Returns the ANSI color code for a slog level (console output).
fn level_color(level: slog::Level) -> &'static str {
    use slog::Level::*;
    match level {
        Critical | Error => "\x1b[31m", // red
        Warning => "\x1b[33m",          // yellow
        Info => "\x1b[32m",             // green
        Debug => "\x1b[34m",            // blue
        Trace => "\x1b[35m",            // magenta
    }
}

/// A [`Serializer`](slog::Serializer) that renders slog key-values as
/// `key=value` text.
///
/// String values are wrapped in double quotes (e.g. `action="login"`); other
/// types are rendered via `Display`. A leading space is used as a separator.
struct KvSerializer<'a> {
    out: &'a mut String,
}

impl slog::Serializer for KvSerializer<'_> {
    fn emit_arguments(&mut self, key: slog::Key, val: &fmt::Arguments<'_>) -> slog::Result {
        use std::fmt::Write as _;
        let _ = write!(self.out, " {}={}", key, val);
        Ok(())
    }

    /// Wrap string values in double quotes to avoid breaking the log format
    /// when the value contains spaces.
    fn emit_str(&mut self, key: slog::Key, val: &str) -> slog::Result {
        use std::fmt::Write as _;
        let _ = write!(self.out, " {}=\"{}\"", key, val);
        Ok(())
    }
}

/// Rolling file drain for slog.
///
/// Uses `Mutex<RollingFileWriter>` to obtain `Send + Sync`, satisfying
/// `Logger::root`'s `Drain: Send + Sync + 'static` bound. `Err = Never` means
/// writes never fail (IO errors are swallowed and the line is dropped).
struct RollingDrain {
    writer: Mutex<RollingFileWriter>,
    tz: Tz,
    level: slog::Level,
}

impl slog::Drain for RollingDrain {
    type Ok = ();
    type Err = slog::Never;

    /// Runtime level filter: skip logs below the configured level.
    fn is_enabled(&self, level: slog::Level) -> bool {
        level <= self.level
    }

    fn log(
        &self,
        record: &slog::Record<'_>,
        values: &slog::OwnedKVList,
    ) -> Result<Self::Ok, Self::Err> {
        // slog macros call `Logger::log` directly (bypassing `is_enabled`), so
        // the runtime level filter must be applied manually here.
        if !self.is_enabled(record.level()) {
            return Ok(());
        }

        // Serialize structured key-values: inline ones (record.kv()) first, then
        // logger-context ones (values).
        let mut kv_buf = String::new();
        {
            let mut ser = KvSerializer { out: &mut kv_buf };
            let _ = record.kv().serialize(record, &mut ser);
            let _ = values.serialize(record, &mut ser);
        }

        let ts = now_in(self.tz).format("%Y-%m-%d %H:%M:%S%.3f").to_string();

        // Console (colored).
        let color = level_color(record.level());
        println!(
            "{} {}{}{} {} - {}{}",
            ts,
            color,
            record.level(),
            RESET,
            record.module(),
            record.msg(),
            kv_buf
        );

        // File (plain text).
        let mut w = self.writer.lock().unwrap();
        let _ = writeln!(
            w,
            "{} [{}] {} - {}{}",
            ts,
            record.level(),
            record.module(),
            record.msg(),
            kv_buf
        );
        Ok(())
    }
}

/// Parses `config.level` into a slog [`Level`].
///
/// `config.level` follows `EnvFilter` syntax (e.g. "info,my_crate=debug"), but
/// slog only supports a single global level, so this takes the first token and
/// falls back to `Info` on failure.
///
/// [`Level`]: slog::Level
fn parse_level(level: &str) -> slog::Level {
    level
        .split(',')
        .next()
        .and_then(|s| s.trim().parse::<slog::Level>().ok())
        .unwrap_or(slog::Level::Info)
}

/// Initialize the slog facade logging system.
///
/// Returns a [`crate::LoggerGuard`]; obtain the `slog::Logger` via
/// [`LoggerGuard::logger`](crate::LoggerGuard::logger) and pass it to slog macros:
///
/// ```ignore
/// let guard = init_slog_logger(&config)?;
/// let log = guard.logger();
/// slog::info!(log, "hello"; "user_id" => 42);
/// ```
///
/// The guard must stay alive until the program exits; dropping it gracefully
/// shuts down the archiver threads.
pub fn init_slog_logger(config: &LogConfig) -> anyhow::Result<crate::LoggerGuard> {
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

    // Logger::root requires Drain with Err = Never and Ok = (), which our
    // RollingDrain satisfies directly.
    let logger = slog::Logger::root(drain, slog::o!());

    // Register the logger globally so the facade-agnostic macros
    // (`rolling_logger::info!` etc.) can auto-inject it.
    let _ = GLOBAL_SLOG_LOGGER.set(logger.clone());

    Ok(crate::LoggerGuard { slog: logger })
}

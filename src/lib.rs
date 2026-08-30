//! rolling-logger: a reusable rolling file logging library.
//!
//! Provides a production-grade rolling file logging capability that any project
//! can depend on directly:
//!
//! - A rolling file writer supporting both date-based and size-based rotation.
//! - Automatic gzip archival of historical logs into the `history/` directory
//!   (async, concurrency-limited, atomic, crash-safe).
//! - Configurable archive delay in days (negative = archive on rotation).
//! - Configurable log timezone (IANA name, for cross-timezone deployment).
//! - Optional fsync durability, dropped-line monitoring, and graceful shutdown.
//! - Pluggable facades: defaults to the [`tracing`] ecosystem, and can also
//!   target the [`log`] / [`slog`] ecosystems via the `log-backend` /
//!   `slog-backend` features.
//!
//! # Facades (features)
//!
//! The core rolling writer [`RollingFileWriter`] is a facade-agnostic `io::Write`
//! implementation reusable by any logging backend. This crate ships out-of-the-box
//! initialization for three mainstream facades, which are **mutually exclusive**
//! (only one may be enabled):
//!
//! | feature | default | underlying facade |
//! | --- | --- | --- |
//! | `tracing` | ✅ | [`tracing`] (`tracing-subscriber`) |
//! | `log-backend` | ❌ | [`log`] |
//! | `slog-backend` | ❌ | [`slog`] |
//!
//! Regardless of the facade, initialization always uses the single [`init`] entry:
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
//! let guard = init(&config)?;   // keep alive in the main scope
//!
//! // tracing facade (default):
//! tracing::info!("hello via tracing");
//! // log facade (`--no-default-features --features log-backend`):
//! // log::info!("hello via log");
//! // slog facade (`--no-default-features --features slog-backend`):
//! // let log = guard.logger();
//! // slog::info!(log, "hello via slog"; "user_id" => 42);
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! If you need explicit control (bypassing the unified entry), use the low-level
//! primitives [`init_logger`] / [`init_log_logger`] / [`init_slog_logger`].

// Strictly mutually exclusive: tracing / log-backend / slog-backend. Enabling
// more than one fails compilation.
#[cfg(all(feature = "tracing", feature = "log-backend"))]
compile_error!(
    "features `tracing` and `log-backend` are mutually exclusive: \
     enable exactly one of them"
);
#[cfg(all(feature = "tracing", feature = "slog-backend"))]
compile_error!(
    "features `tracing` and `slog-backend` are mutually exclusive: \
     enable exactly one of them"
);
#[cfg(all(feature = "log-backend", feature = "slog-backend"))]
compile_error!(
    "features `log-backend` and `slog-backend` are mutually exclusive: \
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
#[cfg(feature = "slog-backend")]
mod slog_layer;

pub use config::LogConfig;
pub use writer::{parse_timezone, shutdown_archivers, RollingFileWriter};
#[cfg(feature = "tracing")]
pub use tracing_layer::init_logger;
#[cfg(feature = "log-backend")]
pub use log_layer::init_log_logger;
#[cfg(feature = "slog-backend")]
pub use slog_layer::init_slog_logger;
#[cfg(feature = "slog-backend")]
#[doc(hidden)]
pub use slog_layer::global_slog_logger;
// Re-export slog so the facade macros can reference `$crate::slog::info!` etc.
// under macro hygiene (callers do not need a direct `slog` dependency).
#[cfg(feature = "slog-backend")]
pub use slog;

/// Unified logger guard: `init` returns this type regardless of the facade.
///
/// It must stay alive until the program exits (bind it in the `main` scope).
///
/// Dropping it:
/// - flushes the non-blocking write buffers (tracing facade);
/// - gracefully shuts down the archiver threads (all facades).
///
/// Dropping it too early loses buffered logs under the tracing facade.
pub struct LoggerGuard {
    #[cfg(feature = "tracing")]
    tracing: tracing_layer::LoggerGuards,
    #[cfg(feature = "slog-backend")]
    slog: slog::Logger,
}

impl Drop for LoggerGuard {
    fn drop(&mut self) {
        // Gracefully shut down archiver threads (idempotent; under the tracing
        // facade, LoggerGuards' Drop calls this too).
        writer::shutdown_archivers();
    }
}

impl LoggerGuard {
    /// Returns the number of file log lines dropped because the channel was full
    /// (tracing facade only, for monitoring/alerting).
    #[cfg(feature = "tracing")]
    pub fn dropped_file_lines(&self) -> usize {
        self.tracing.dropped_file_lines()
    }

    /// Returns the slog `Logger` instance (slog facade only, to pass to slog macros).
    #[cfg(feature = "slog-backend")]
    pub fn logger(&self) -> slog::Logger {
        self.slog.clone()
    }
}

/// Unified logging system initialization entry.
///
/// The underlying facade is decided at compile time by the feature
/// (`tracing` / `log-backend` / `slog-backend`, mutually exclusive).
/// Returns a [`LoggerGuard`] that must stay alive until the program exits.
pub fn init(config: &LogConfig) -> anyhow::Result<LoggerGuard> {
    // The three facade features are guaranteed mutually exclusive by the
    // compile_error! guards above (at most one enabled), so each branch only
    // needs a single `#[cfg(feature = "...")]` without `not(...)` combinations.
    // All three low-level primitives already return LoggerGuard, so pass through.
    #[cfg(feature = "tracing")]
    {
        init_logger(config)
    }
    #[cfg(feature = "log-backend")]
    {
        init_log_logger(config)
    }
    #[cfg(feature = "slog-backend")]
    {
        init_slog_logger(config)
    }
    #[cfg(not(any(feature = "tracing", feature = "log-backend", feature = "slog-backend")))]
    {
        // No facade feature enabled: the core writer is still usable, but there
        // is no facade initialization capability.
        let _ = config;
        anyhow::bail!(
            "no logging facade feature enabled (enable `tracing`, `log-backend` or `slog-backend`)"
        )
    }
}

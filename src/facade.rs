//! Facade-agnostic logging macro abstraction layer.
//!
//! Provides five level macros: [`trace!`], [`debug!`], [`info!`], [`warn!`],
//! [`error!`], which proxy to the corresponding facade (`tracing` or `log`)
//! based on the feature enabled at compile time.
//!
//! These macros are usable both by downstream callers (`use rolling_logger::info;`)
//! and internally by this crate (e.g. self-diagnostics in `writer.rs`).
//!
//! # Proxy rules
//!
//! - `tracing` enabled (default) → proxies to the corresponding [`tracing`] macros.
//! - `log-backend` enabled → proxies to the corresponding [`log`] macros.
//! - `slog-backend` enabled → proxies to the corresponding [`slog`] macros,
//!   auto-injecting the global logger set by `init_slog_logger`, so the same
//!   positional-argument syntax (`info!("msg {}", x)`) works.
//! - no facade enabled → degrades to no-op (fully lazy: args not evaluated,
//!   zero cost).
//!
//! Only the **common subset** syntax is supported: `info!("msg {}", x)`.
//! Structured field syntax specific to a facade (e.g. `tracing`'s
//! `info!(field = v, "msg")`, `slog`'s `info!(logger, "msg"; "k" => v)`) is out
//! of scope here; use the facade's native macros directly when needed.

// ─────────────────────────────────────────────────────────────────────────────
// Facade macro re-export: re-export the current facade's five level macros into
// this module under the same names, so the public macros below can forward to
// them via `$crate::facade::<name>!` (ensuring macro hygiene).
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "tracing")]
#[doc(hidden)]
pub use tracing::{debug, error, info, trace, warn};

#[cfg(feature = "log-backend")]
#[doc(hidden)]
pub use log::{debug, error, info, trace, warn};

// ─────────────────────────────────────────────────────────────────────────────
// Public macros (facade-agnostic)
//
// When a facade is enabled, forward to the re-exports above; when no facade is
// enabled, degrade to no-op. Note: `#[cfg]` cannot appear inside a `macro_rules!`
// body (cfg is evaluated before macro expansion), so each level defines two
// versions ("facade enabled" / "no facade") selected by `#[cfg]`.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(any(feature = "tracing", feature = "log-backend"))]
#[macro_export]
macro_rules! trace {
    ($($arg:tt)+) => { $crate::facade::trace!($($arg)+) };
}
#[cfg(feature = "slog-backend")]
#[macro_export]
macro_rules! trace {
    ($($arg:tt)+) => { $crate::slog::trace!($crate::global_slog_logger(), $($arg)+) };
}
#[cfg(not(any(feature = "tracing", feature = "log-backend", feature = "slog-backend")))]
#[macro_export]
macro_rules! trace {
    ($($arg:tt)+) => {};
}

#[cfg(any(feature = "tracing", feature = "log-backend"))]
#[macro_export]
macro_rules! debug {
    ($($arg:tt)+) => { $crate::facade::debug!($($arg)+) };
}
#[cfg(feature = "slog-backend")]
#[macro_export]
macro_rules! debug {
    ($($arg:tt)+) => { $crate::slog::debug!($crate::global_slog_logger(), $($arg)+) };
}
#[cfg(not(any(feature = "tracing", feature = "log-backend", feature = "slog-backend")))]
#[macro_export]
macro_rules! debug {
    ($($arg:tt)+) => {};
}

#[cfg(any(feature = "tracing", feature = "log-backend"))]
#[macro_export]
macro_rules! info {
    ($($arg:tt)+) => { $crate::facade::info!($($arg)+) };
}
#[cfg(feature = "slog-backend")]
#[macro_export]
macro_rules! info {
    ($($arg:tt)+) => { $crate::slog::info!($crate::global_slog_logger(), $($arg)+) };
}
#[cfg(not(any(feature = "tracing", feature = "log-backend", feature = "slog-backend")))]
#[macro_export]
macro_rules! info {
    ($($arg:tt)+) => {};
}

#[cfg(any(feature = "tracing", feature = "log-backend"))]
#[macro_export]
macro_rules! warn {
    ($($arg:tt)+) => { $crate::facade::warn!($($arg)+) };
}
#[cfg(feature = "slog-backend")]
#[macro_export]
macro_rules! warn {
    ($($arg:tt)+) => { $crate::slog::warn!($crate::global_slog_logger(), $($arg)+) };
}
#[cfg(not(any(feature = "tracing", feature = "log-backend", feature = "slog-backend")))]
#[macro_export]
macro_rules! warn {
    ($($arg:tt)+) => {};
}

#[cfg(any(feature = "tracing", feature = "log-backend"))]
#[macro_export]
macro_rules! error {
    ($($arg:tt)+) => { $crate::facade::error!($($arg)+) };
}
#[cfg(feature = "slog-backend")]
#[macro_export]
macro_rules! error {
    ($($arg:tt)+) => { $crate::slog::error!($crate::global_slog_logger(), $($arg)+) };
}
#[cfg(not(any(feature = "tracing", feature = "log-backend", feature = "slog-backend")))]
#[macro_export]
macro_rules! error {
    ($($arg:tt)+) => {};
}

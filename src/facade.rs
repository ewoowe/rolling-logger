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
//! - `slog-backend` or no facade enabled → degrades to no-op (fully lazy: args
//!   not evaluated, zero cost), because slog's syntax is incompatible with this
//!   macro layer.
//!
//! Only the **common subset** syntax of the two facades is supported:
//! `info!("msg {}", x)` and `info!(target: "...", "msg {}", x)`. The structured
//! field syntax specific to `tracing` (e.g. `info!(field = v, "msg")`) is out of
//! scope here; use `tracing::info!` directly when needed.

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
#[cfg(not(any(feature = "tracing", feature = "log-backend")))]
#[macro_export]
macro_rules! trace {
    ($($arg:tt)+) => {};
}

#[cfg(any(feature = "tracing", feature = "log-backend"))]
#[macro_export]
macro_rules! debug {
    ($($arg:tt)+) => { $crate::facade::debug!($($arg)+) };
}
#[cfg(not(any(feature = "tracing", feature = "log-backend")))]
#[macro_export]
macro_rules! debug {
    ($($arg:tt)+) => {};
}

#[cfg(any(feature = "tracing", feature = "log-backend"))]
#[macro_export]
macro_rules! info {
    ($($arg:tt)+) => { $crate::facade::info!($($arg)+) };
}
#[cfg(not(any(feature = "tracing", feature = "log-backend")))]
#[macro_export]
macro_rules! info {
    ($($arg:tt)+) => {};
}

#[cfg(any(feature = "tracing", feature = "log-backend"))]
#[macro_export]
macro_rules! warn {
    ($($arg:tt)+) => { $crate::facade::warn!($($arg)+) };
}
#[cfg(not(any(feature = "tracing", feature = "log-backend")))]
#[macro_export]
macro_rules! warn {
    ($($arg:tt)+) => {};
}

#[cfg(any(feature = "tracing", feature = "log-backend"))]
#[macro_export]
macro_rules! error {
    ($($arg:tt)+) => { $crate::facade::error!($($arg)+) };
}
#[cfg(not(any(feature = "tracing", feature = "log-backend")))]
#[macro_export]
macro_rules! error {
    ($($arg:tt)+) => {};
}

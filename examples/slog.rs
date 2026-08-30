//! Example targeting the `slog` facade.
//!
//! Run with (enables slog-backend and disables the default tracing):
//! ```sh
//! cargo run --example slog --no-default-features --features slog-backend
//! ```
//!
//! Note: slog macros require an explicit logger argument and use structured
//! key-value syntax. The facade-agnostic macros (`rolling_logger::info!` etc.)
//! auto-inject the global logger, so they also work under slog with the
//! positional-argument syntax.

use rolling_logger::{init, LogConfig};

fn main() -> anyhow::Result<()> {
    let config = LogConfig {
        dir: "./logs".into(),
        level: "info".into(),
        file_prefix: "app".into(),
        max_file_size_mb: 10,
        max_files: 30,
        archive_delay_days: 0,
        archive_batch_size: 100,
        fsync_on_flush: false,
        timezone: "UTC".into(),
    };

    // Unified init entry: with slog-backend enabled (and others disabled), the
    // underlying facade is `slog`.
    let guard = init(&config)?;

    // Obtain the slog Logger from the guard (cheap Arc clone).
    let log = guard.logger();

    // ── 1. Native slog macros ─────────────────────────────────────────────────
    // Use slog's native API directly for structured key-values (requires an
    // explicit logger argument).
    slog::info!(log, "hello via native slog macro!");
    slog::debug!(log, "this line is hidden by level filter 'info'");
    slog::warn!(log, "native warning via slog"; "code" => 1001);
    slog::error!(log, "native error via slog"; "user_id" => 42, "action" => "login");

    // ── 2. Facade-agnostic macros ─────────────────────────────────────────────
    // Same positional-argument syntax as log/tracing, auto-injecting the global
    // logger registered by `init`.
    rolling_logger::info!("hello via facade-agnostic macro!");
    rolling_logger::warn!("warning via facade-agnostic macro: code {}", 1001);
    rolling_logger::error!("error via facade-agnostic macro: user {}", 42);

    Ok(())
}

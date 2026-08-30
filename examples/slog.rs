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

    // Native slog macros: message + structured key-values.
    slog::info!(log, "hello, slog backend!");
    slog::debug!(log, "this line is hidden by level filter 'info'");
    slog::warn!(log, "warning via slog"; "code" => 1001);
    slog::error!(log, "error via slog"; "user_id" => 42, "action" => "login");

    // Facade-agnostic macros: same positional-argument syntax as log/tracing,
    // auto-injecting the global logger registered by `init`.
    rolling_logger::info!("facade info: hello {}", "world");
    rolling_logger::warn!("facade warn: code {}", 1001);
    rolling_logger::error!("facade error: user {}", 42);

    Ok(())
}

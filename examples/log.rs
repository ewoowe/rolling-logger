//! Example targeting the `log` facade.
//!
//! Run with (enables log-backend and disables the default tracing):
//! ```sh
//! cargo run --example log --no-default-features --features log-backend
//! ```
//!
//! Note: this example uses the **exact same** facade-agnostic macros as
//! `tracing.rs`; only the compiled feature differs. This demonstrates the value
//! of the facade-agnostic macros — business logging code does not depend on a
//! specific facade.

use rolling_logger::{debug, error, info, init, trace, warn, LogConfig};

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

    // Unified init entry: with log-backend enabled (and tracing disabled), the
    // underlying facade is `log`.
    let _guard = init(&config)?;

    // ── 1. Facade-agnostic macros ─────────────────────────────────────────────
    // These proxy to log::*! in this example, without a direct dependency on the
    // `log` crate (the macros locate the facade via $crate).
    trace!("trace level (filtered by info)");
    debug!("debug level (filtered by info)");
    info!("hello via facade-agnostic macro!");
    warn!("warning via facade-agnostic macro");
    error!("error via facade-agnostic macro");

    // ── 2. Native log macros ──────────────────────────────────────────────────
    // Use log's native API directly when you want to bypass the facade-agnostic
    // layer. The `log` crate is available because log-backend enables it.
    log::info!("hello via native log macro!");
    log::warn!("native warning via log crate");
    log::error!("native error via log crate");
    log::info!(target: "my_component", "native log with a target");

    Ok(())
}

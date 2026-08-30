//! Rolling file logger example using the tracing facade (default feature).
//!
//! Run with:
//! ```sh
//! cargo run --example tracing
//! ```

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

    // Unified init entry: the facade is decided at compile time by the feature
    // (tracing by default). The returned guard must stay alive until exit,
    // otherwise buffered logs in the non-blocking writer would be lost on drop.
    let guard = init(&config)?;

    // ── 1. Facade-agnostic macros ─────────────────────────────────────────────
    // These proxy to tracing::*! in this example. Switch the feature to log or
    // slog and the exact same code keeps working (see log.rs / slog.rs).
    trace!("trace level (filtered by info)");
    debug!("debug level (filtered by info)");
    info!("hello via facade-agnostic macro!");
    warn!("warning via facade-agnostic macro");
    error!("error via facade-agnostic macro");

    // ── 2. Native tracing macros ──────────────────────────────────────────────
    // Use tracing's native API directly when you need tracing-specific features
    // such as structured fields or spans.
    tracing::info!("hello via native tracing macro!");
    tracing::warn!(code = 1001, "native warning with a structured field");
    tracing::error!(user_id = 42, action = "login", "native error with fields");
    tracing::info!(answer = 42, "structured fields example");

    // Number of file lines dropped because the channel was full (tracing only).
    let _dropped = guard.dropped_file_lines();

    Ok(())
}

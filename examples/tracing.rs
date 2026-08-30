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

    // Facade-agnostic macros: proxy to tracing::*! in this example. If you switch
    // to the log facade (--no-default-features --features log-backend), they proxy
    // to log::*! with zero code changes (see log.rs).
    trace!("trace level (filtered by info)");
    debug!("debug level (filtered by info)");
    info!("hello, rolling-logger!");
    warn!("warning goes to both console and file");
    error!("error also goes to both");

    // The facade-agnostic macros also support the target syntax (common subset).
    info!(target: "my_component", "a log line with a target");

    // For tracing-specific features (e.g. structured fields), use native macros.
    tracing::info!(answer = 42, "tracing structured fields example");

    // Number of file lines dropped because the channel was full (tracing only).
    let _dropped = guard.dropped_file_lines();

    Ok(())
}

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

    // Facade-agnostic macros: proxy to log::*! in this example, without a direct
    // dependency on the `log` crate (macros locate the facade via $crate).
    trace!("trace level (filtered by info)");
    debug!("debug level (filtered by info)");
    info!("hello, log backend!");
    warn!("warning via log backend");
    error!("error via log backend");

    // The facade-agnostic macros also support the target syntax.
    info!(target: "my_component", "a log line with a target");

    Ok(())
}

use serde::Deserialize;

/// Logging configuration.
#[derive(Clone, Debug, Deserialize)]
pub struct LogConfig {
    /// Directory where log files are stored.
    pub dir: String,
    /// Log level filter rule (e.g. "info,zero_distance=debug").
    pub level: String,
    /// Log file name prefix.
    pub file_prefix: String,
    /// Maximum size of a single log file (in MB).
    pub max_file_size_mb: u64,
    /// Maximum number of archived log files to retain.
    pub max_files: usize,
    /// Archive delay in days: only archive log files whose date is older than
    /// "today - N days".
    ///
    /// - `0` = archive everything before today.
    /// - `1` = archive everything before yesterday, and so on.
    /// - negative = archive immediately upon rotation (don't wait).
    ///
    /// Defaults to `0`.
    #[serde(default)]
    pub archive_delay_days: i64,
    /// Maximum number of files to archive in a single pass, to avoid blocking
    /// too long when compressing a huge backlog. Files beyond this limit are
    /// left for the next rotation. Defaults to `100`.
    #[serde(default = "default_archive_batch_size")]
    pub archive_batch_size: usize,
    /// Whether to force `fsync` on flush (default `false`).
    /// When `true`, logs survive crashes but throughput drops noticeably.
    #[serde(default)]
    pub fsync_on_flush: bool,
    /// Timezone used for log timestamps (IANA name, e.g. "UTC"/"Asia/Shanghai"),
    /// defaults to "UTC".
    #[serde(default = "default_timezone")]
    pub timezone: String,
}

/// Default number of files archived per pass.
fn default_archive_batch_size() -> usize {
    100
}

/// Default log timezone.
fn default_timezone() -> String {
    "UTC".to_string()
}

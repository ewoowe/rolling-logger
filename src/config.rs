use serde::Deserialize;

/// 日志配置
#[derive(Clone, Debug, Deserialize)]
pub struct LogConfig {
    /// 日志文件存储目录
    pub dir: String,
    /// 日志级别过滤规则（如 "info,zero_distance=debug"）
    pub level: String,
    /// 日志文件前缀名
    pub file_prefix: String,
    /// 单个日志文件最大大小（MB）
    pub max_file_size_mb: u64,
    /// 最多保留多少个日志文件
    pub max_files: usize,
    /// 归档等待天数：仅归档日期早于「今天 - N 天」的历史日志
    /// 0 = 今天之前的日志都归档；1 = 昨天之前的日志都归档；依此类推
    /// 负数 = 滚动即归档（历史日志关闭后立即归档，不等天数）
    /// 默认 0
    #[serde(default)]
    pub archive_delay_days: i64,
    /// 单次归档最多处理的文件数量，避免一次性压缩海量文件阻塞过久
    /// 超过该数量的文件留待下次滚动时继续归档，默认 100
    #[serde(default = "default_archive_batch_size")]
    pub archive_batch_size: usize,
    /// flush 时是否强制 fsync 落盘（默认 false）
    /// true 保证崩溃后日志不丢，但会明显降低日志吞吐
    #[serde(default)]
    pub fsync_on_flush: bool,
    /// 日志时间戳使用的时区（IANA 名，如 "UTC"/"Asia/Shanghai"），默认 "UTC"
    #[serde(default = "default_timezone")]
    pub timezone: String,
}

/// 默认单次归档文件数量
fn default_archive_batch_size() -> usize {
    100
}

/// 默认日志时区
fn default_timezone() -> String {
    "UTC".to_string()
}

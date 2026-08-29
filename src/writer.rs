use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use flate2::write::GzEncoder;
use flate2::Compression;

/// 用于生成唯一临时文件名的原子计数器（避免并发归档线程写同一 .tmp 冲突）
static NEXT_TMP_ID: AtomicU64 = AtomicU64::new(0);

/// 最大并发归档线程数，防止滚动频繁时线程爆炸
const MAX_CONCURRENT_ARCHIVERS: usize = 2;

/// 当前活跃归档线程数（用于限并发）
static ACTIVE_ARCHIVERS: AtomicUsize = AtomicUsize::new(0);

/// 归档线程句柄集合，程序退出时用于优雅关闭（join 等待完成）
static ARCHIVE_HANDLES: OnceLock<Mutex<Vec<std::thread::JoinHandle<()>>>> = OnceLock::new();

/// 获取归档线程句柄集合（惰性初始化）
fn archive_handles() -> &'static Mutex<Vec<std::thread::JoinHandle<()>>> {
    ARCHIVE_HANDLES.get_or_init(|| Mutex::new(Vec::new()))
}

/// 归档线程活跃计数守卫：线程退出（含 panic 展开）时自动递减计数
struct ActiveArchiverGuard;

impl Drop for ActiveArchiverGuard {
    fn drop(&mut self) {
        ACTIVE_ARCHIVERS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// 解析时区字符串为 `chrono_tz::Tz`，失败回退 UTC
pub fn parse_timezone(tz: &str) -> Tz {
    tz.parse::<Tz>().unwrap_or(Tz::UTC)
}

/// 获取指定时区的当前时间
pub(crate) fn now_in(tz: Tz) -> DateTime<Tz> {
    Utc::now().with_timezone(&tz)
}

// ─────────────────────────────────────────────────────────────────────────────
// 自定义滚动文件写入器：支持按日期 + 按大小双重滚动
// ─────────────────────────────────────────────────────────────────────────────

/// 支持日期和大小滚动的日志文件写入器
///
/// 文件命名规则：
/// - 默认：`{prefix}.{YYYY-MM-DD}.log`
/// - 大小超限：`{prefix}.{YYYY-MM-DD}.{seq}.log`（seq 从 1 开始递增）
pub struct RollingFileWriter {
    /// 日志文件目录
    dir: PathBuf,
    /// 文件名前缀
    prefix: String,
    /// 单个文件最大字节数（0 表示不限制大小）
    max_file_size: u64,
    /// 最多保留多少个日志文件（0 表示不限制）
    max_files: usize,
    /// 归档等待天数：仅归档日期早于「今天 - N 天」的历史日志
    /// 负数 = 滚动即归档（历史日志关闭后立即归档，不等天数）
    archive_delay_days: i64,
    /// 单次归档最多处理的文件数量
    archive_batch_size: usize,
    /// flush 时是否强制 fsync 落盘
    fsync_on_flush: bool,
    /// 日志时间戳使用的时区
    timezone: Tz,
    /// 当前打开的文件
    current_file: Option<File>,
    /// 当前日期字符串（YYYY-MM-DD）
    current_date: String,
    /// 当前日期内的序号（0 表示第一个文件）
    current_seq: u32,
    /// 当前文件已写入的字节数
    current_size: u64,
    /// 上次检查日期变更的时间点
    last_date_check: Instant,
    /// 日期检查间隔（秒）
    date_check_secs: u64,
}

impl RollingFileWriter {
    /// 创建新的滚动文件写入器
    pub fn new(
        dir: impl Into<PathBuf>,
        prefix: &str,
        max_file_size_mb: u64,
        max_files: usize,
        archive_delay_days: i64,
        archive_batch_size: usize,
        fsync_on_flush: bool,
        timezone: Tz,
    ) -> io::Result<Self> {
        let mut writer = Self {
            dir: dir.into(),
            prefix: prefix.to_string(),
            max_file_size: max_file_size_mb * 1024 * 1024,
            max_files,
            archive_delay_days,
            archive_batch_size,
            fsync_on_flush,
            timezone,
            current_file: None,
            current_date: String::new(),
            current_seq: 0,
            current_size: 0,
            last_date_check: Instant::now() - std::time::Duration::from_secs(5), // 首次强制检查
            date_check_secs: 5, // 每 5 秒检查一次日期变更
        };
        // 首次 rotate_if_needed 会因 current_date 为空而触发滚动，
        // 届时统一完成「异步归档遗留历史日志 + 打开当日文件」。无需在此单独归档。
        writer.rotate_if_needed()?;
        Ok(writer)
    }

    /// 生成当前日志文件名
    fn make_filename(&self) -> String {
        if self.current_seq == 0 {
            format!("{}.{}.log", self.prefix, self.current_date)
        } else {
            format!("{}.{}.{}.log", self.prefix, self.current_date, self.current_seq)
        }
    }

    /// 检查是否需要滚动，如果需要则执行滚动
    ///
    /// 性能优化：
    /// - 大小检查：纯整数比较，每次执行，开销可忽略
    /// - 日期检查：涉及时间格式化（堆分配），每隔 `date_check_secs` 秒才执行一次
    ///   使用 `Instant::now()`（零分配、纳秒级）判断是否到达检查时间点
    fn rotate_if_needed(&mut self) -> io::Result<()> {
        // 大小检查：每次执行，开销极低
        let size_exceeded =
            self.max_file_size > 0 && self.current_size >= self.max_file_size;

        // 日期检查：按时间间隔执行，避免每次 write 都分配 String
        let should_check_date = self.last_date_check.elapsed().as_secs() >= self.date_check_secs;
        let date_changed = if should_check_date {
            self.last_date_check = Instant::now();
            let today = now_in(self.timezone).format("%Y-%m-%d").to_string();
            let changed = self.current_date != today;
            if changed {
                self.current_date = today;
            }
            changed
        } else {
            false
        };

        // 不需要滚动且文件已打开，直接返回
        if !date_changed && !size_exceeded && self.current_file.is_some() {
            return Ok(());
        }

        // 需要滚动：先关闭当前文件（flush 后 drop 关闭句柄）
        if let Some(mut file) = self.current_file.take() {
            let _ = file.flush();
            drop(file);
        }

        // 日期变更：重置序号
        if date_changed {
            self.current_seq = 0;
            crate::debug!("[日志滚动] 检测到日期变更，新日期: {}，序号重置为 0", self.current_date);
        } else if size_exceeded {
            // 仅大小超限：递增序号
            self.current_seq += 1;
            crate::debug!(
                "[日志滚动] 文件大小超限，当前大小: {} bytes，限制: {} bytes，序号递增为 {}",
                self.current_size, self.max_file_size, self.current_seq
            );
        }

        // 确保目录存在
        fs::create_dir_all(&self.dir)?;

        // 将上一次以及遗留的历史日志收集后，交给独立线程异步压缩归档
        // （此时当前文件已关闭、新文件尚未打开，目录下的 .log 均为历史文件）
        // 归档不阻塞日志写入；归档完成后由该线程顺带清理超量归档文件，
        // 从而避免「本次 .gz 尚未生成」导致的清理时机滞后。
        if let Some(files) = self.collect_archive_targets() {
            Self::spawn_archive_worker(
                files,
                self.dir.clone(),
                self.prefix.clone(),
                self.max_files,
            );
        }

        // 打开新文件
        let file_path = self.dir.join(self.make_filename());
        let _filename = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");
        crate::debug!("[日志滚动] 打开新日志文件: {}", file_path.display());
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)?;

        // 获取当前文件大小（如果文件已存在则从末尾追加）
        let metadata = file.metadata()?;
        self.current_size = metadata.len();
        self.current_file = Some(file);
        crate::debug!(
            "[日志滚动] 日志文件已就绪: {}，已有大小: {} bytes",
            _filename, self.current_size
        );

        Ok(())
    }

    /// 收集应归档的历史日志文件列表（不执行压缩，轻量）
    ///
    /// 扫描日志目录下所有匹配前缀且日期早于「今天 - delay 天」的 `.log`
    /// 文件，按修改时间升序（最旧优先）排序，最多返回 `archive_batch_size` 个。
    /// 无待归档文件时返回 `None`。
    fn collect_archive_targets(&self) -> Option<Vec<PathBuf>> {
        // 日志目录尚不存在（首次运行），无需归档
        if !self.dir.exists() {
            return None;
        }

        let prefix_pattern = format!("{}.", self.prefix);
        let today = now_in(self.timezone).date_naive();
        let delay = self.archive_delay_days;

        let mut files: Vec<PathBuf> = fs::read_dir(&self.dir)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| {
                            n.starts_with(&prefix_pattern)
                                && n.ends_with(".log")
                                && Self::should_archive(n, &self.prefix, today, delay)
                        })
                        .unwrap_or(false)
            })
            .collect();

        if files.is_empty() {
            return None;
        }

        // 最旧的文件优先归档（文件名含 YYYY-MM-DD 日期，字典序即时间序，零系统调用）
        files.sort();
        files.truncate(self.archive_batch_size);

        Some(files)
    }

    /// 将一批日志文件 gzip 压缩归档到 history 目录（同步、幂等）
    ///
    /// 每个文件压缩为 `history/{原名}.gz`，成功后删除原文件。单个文件失败
    /// 仅告警不中断。重复调用（源已删、目标已存在）是安全的。
    fn archive_files(files: Vec<PathBuf>, history_dir: PathBuf) {
        if let Err(_e) = fs::create_dir_all(&history_dir) {
            crate::warn!("[日志归档] 创建归档目录 {} 失败: {}", history_dir.display(), _e);
            return;
        }

        for src in files {
            let filename = match src.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let gz_path = history_dir.join(format!("{}.gz", filename));

            // 源已被其他归档任务处理掉
            if !src.exists() {
                continue;
            }
            // 目标已存在：仅清理残留源文件（幂等收尾）
            if gz_path.exists() {
                if let Err(_e) = fs::remove_file(&src) {
                    crate::warn!("[日志归档] 删除原文件 {} 失败: {}", src.display(), _e);
                }
                continue;
            }

            match Self::compress_file(&src, &gz_path) {
                Ok(()) => {
                    if let Err(_e) = fs::remove_file(&src) {
                        crate::warn!("[日志归档] 删除原文件 {} 失败: {}", src.display(), _e);
                    }
                    crate::debug!("[日志归档] 已归档: {} -> {}", src.display(), gz_path.display());
                }
                Err(_e) => {
                    crate::warn!("[日志归档] 压缩 {} 失败: {}", filename, _e);
                }
            }
        }
    }

    /// 在独立线程中异步压缩归档一批日志文件，完成后清理超量归档
    ///
    /// 归档在后台执行，不阻塞日志写入线程。归档完成后立即在同一线程内
    /// 清理超量 `.gz`，保证清理时机正确（此时本次归档文件已全部生成）。
    ///
    /// 线程受 [`MAX_CONCURRENT_ARCHIVERS`] 上限约束：超过上限时跳过本次
    /// 归档（留待下次滚动再试），避免滚动频繁时线程爆炸。句柄保存到全局
    /// [`ARCHIVE_HANDLES`]，供程序退出时优雅关闭（join 等待完成）。
    fn spawn_archive_worker(
        files: Vec<PathBuf>,
        dir: PathBuf,
        prefix: String,
        max_files: usize,
    ) {
        if files.is_empty() {
            return;
        }

        // 限并发：超过上限则跳过本次归档
        if ACTIVE_ARCHIVERS.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT_ARCHIVERS {
            ACTIVE_ARCHIVERS.fetch_sub(1, Ordering::SeqCst);
            crate::debug!("[日志归档] 活跃归档线程已达上限，跳过本次归档");
            return;
        }

        let handle = std::thread::spawn(move || {
            // 线程退出（含 panic）时递减活跃计数
            let _guard = ActiveArchiverGuard;
            let history_dir = dir.join("history");
            Self::archive_files(files, history_dir.clone());
            if max_files > 0 {
                let _ = Self::cleanup_archive_files(&history_dir, &prefix, max_files);
            }
        });

        // 保存句柄用于优雅关闭
        archive_handles().lock().unwrap().push(handle);
    }

    /// 从日志文件名中解析出日期
    ///
    /// 文件名形如 `{prefix}.{YYYY-MM-DD}.log` 或 `{prefix}.{YYYY-MM-DD}.{seq}.log`，
    /// 截取前缀与 `.log` 后缀之间的前 10 个字符（YYYY-MM-DD）解析为日期。
    fn extract_date(filename: &str, prefix: &str) -> Option<NaiveDate> {
        let prefix_dot = format!("{}.", prefix);
        let rest = filename.strip_prefix(&prefix_dot)?;
        let rest = rest.strip_suffix(".log")?;
        let date_str = rest.get(0..10)?;
        NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()
    }

    /// 判断某个日志文件是否应当归档
    ///
    /// 规则：
    /// - delay < 0：滚动即归档，所有历史日志立即归档（不看日期）
    /// - delay >= 0：文件日期早于「今天 - delay 天」（天数差 > delay）时归档；
    ///   delay=0 昨天及更早归档、今天保留；delay=1 前天及更早归档、今天昨天保留
    /// 无法解析出日期的异常命名文件视为应归档，避免无限留存。
    fn should_archive(filename: &str, prefix: &str, today: NaiveDate, delay: i64) -> bool {
        if delay < 0 {
            return true;
        }
        match Self::extract_date(filename, prefix) {
            Some(date) => {
                let diff = (today - date).num_days();
                diff > delay
            }
            None => true,
        }
    }

    /// 将单个文件 gzip 压缩到目标路径（原子写，崩溃安全）
    ///
    /// 先写入唯一临时文件 `{dst}.tmp.{pid}.{seq}`，压缩完成后 `flush` + `sync_all`
    /// 确保数据落盘，再通过 `rename` 原子地替换为目标 `.gz`。这样：
    /// - 目标 `.gz` 一旦存在，就必然是**完整**的（不存在半成品）
    /// - 压缩中途失败/崩溃只会留下 `.tmp`，原始 `.log` 与既有 `.gz` 均不受影响
    fn compress_file(src: &Path, dst: &Path) -> io::Result<()> {
        let tmp = dst.with_extension(format!(
            "gz.tmp.{}.{}",
            std::process::id(),
            NEXT_TMP_ID.fetch_add(1, Ordering::Relaxed)
        ));

        let result = (|| -> io::Result<()> {
            let mut input = File::open(src)?;
            let output = File::create(&tmp)?;
            let mut encoder = GzEncoder::new(output, Compression::default());
            io::copy(&mut input, &mut encoder)?;
            // finish 返回底层 writer，用于 flush + sync_all
            let mut file = encoder.finish()?;
            file.flush()?;
            file.sync_all()?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                // 数据已完整落盘，原子替换为目标 .gz
                fs::rename(&tmp, dst)?;
                Ok(())
            }
            Err(e) => {
                // 失败清理临时文件，避免残留
                let _ = fs::remove_file(&tmp);
                Err(e)
            }
        }
    }

    /// 清理超出数量限制的归档日志文件（history 目录下的 .gz）
    ///
    /// 按文件名升序（最旧优先）删除超出 `max_files` 的归档文件。
    /// 设计为静态方法，供归档线程在归档完成后调用，保证清理时机正确。
    fn cleanup_archive_files(
        history_dir: &Path,
        prefix: &str,
        max_files: usize,
    ) -> io::Result<()> {
        if !history_dir.exists() {
            return Ok(());
        }

        let entries = fs::read_dir(history_dir)?;
        let prefix_pattern = format!("{}.", prefix);

        // 收集所有匹配的归档文件
        let mut archived: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with(&prefix_pattern) && n.ends_with(".gz"))
                        .unwrap_or(false)
            })
            .collect();

        // 清理崩溃遗留的临时文件（`{prefix}.*.gz.tmp.{pid}.{seq}`）
        let mut tmp_removed = 0usize;
        if let Ok(entries) = fs::read_dir(history_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                let is_tmp = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(&prefix_pattern) && n.contains(".tmp."))
                    .unwrap_or(false);
                if is_tmp && path.is_file() {
                    if fs::remove_file(&path).is_ok() {
                        tmp_removed += 1;
                    }
                }
            }
        }
        if tmp_removed > 0 {
            crate::debug!("[日志清理] 清理崩溃遗留临时文件 {} 个", tmp_removed);
        }

        // 按文件名升序排序（最旧的在前）——文件名含日期+序号，字典序即时间序
        archived.sort();

        // 删除超出限制的旧归档
        let total_count = archived.len();
        while archived.len() > max_files {
            if let Some(oldest) = archived.first() {
                let _filename = oldest.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");
                crate::debug!("[日志清理] 删除过期归档文件: {}", _filename);
                let _ = fs::remove_file(oldest);
            }
            archived.remove(0);
        }
        let removed = total_count - archived.len();
        if removed > 0 {
            crate::debug!("[日志清理] 清理完成，共删除 {} 个过期归档文件", removed);
        }

        Ok(())
    }
}

impl Write for RollingFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.rotate_if_needed()?;
        if let Some(ref mut file) = self.current_file {
            let n = file.write(buf)?;
            self.current_size += n as u64;
            Ok(n)
        } else {
            Err(io::Error::new(io::ErrorKind::Other, "日志文件未打开"))
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(ref mut file) = self.current_file {
            file.flush()?;
            if self.fsync_on_flush {
                file.sync_all()?;
            }
            Ok(())
        } else {
            Ok(())
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 日志初始化
// ─────────────────────────────────────────────────────────────────────────────

/// 等待所有活跃归档线程完成（优雅关闭）
///
/// 程序退出前调用，避免归档线程被强制终止而留下半成品文件。
/// 结合 `compress_file` 的原子写，能保证归档要么完整、要么不存在。
pub fn shutdown_archivers() {
    let handles: Vec<_> = archive_handles().lock().unwrap().drain(..).collect();
    for handle in handles {
        let _ = handle.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use std::io::Read;

    /// 创建独立的临时目录用于测试
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("zd_log_{}_{}", tag, uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 生成「N 天前」的日志文件名（形如 `{prefix}.{YYYY-MM-DD}.log`，固定 UTC 时区）
    fn dated_filename(prefix: &str, days_ago: i64) -> String {
        let date = now_in(Tz::UTC).date_naive() - Duration::days(days_ago);
        format!("{}.{}.log", prefix, date.format("%Y-%m-%d"))
    }

    /// 解压 gzip 文件内容为字符串
    fn read_gz(path: &Path) -> String {
        let file = File::open(path).unwrap();
        let mut decoder = flate2::read::GzDecoder::new(file);
        let mut s = String::new();
        decoder.read_to_string(&mut s).unwrap();
        s
    }

    /// 直接构造写入器（绕过 new 的异步归档，便于测试同步控制归档时机）
    fn test_writer(dir: PathBuf, archive_delay_days: i64) -> RollingFileWriter {
        RollingFileWriter {
            dir,
            prefix: "ZeroDistance".to_string(),
            max_file_size: 0,
            max_files: 0,
            archive_delay_days,
            archive_batch_size: 100,
            fsync_on_flush: false,
            timezone: Tz::UTC,
            current_file: None,
            current_date: String::new(),
            current_seq: 0,
            current_size: 0,
            last_date_check: Instant::now(),
            date_check_secs: 5,
        }
    }

    /// 测试：归档遗留的历史日志（压缩 + 删除原文件 + 移动至 history/）
    #[test]
    fn archives_history_logs() {
        let dir = temp_dir("startup");
        let old1 = dated_filename("ZeroDistance", 1); // 昨天
        let old2 = dated_filename("ZeroDistance", 2); // 前天
        fs::write(dir.join(&old1), "old log line\n").unwrap();
        fs::write(dir.join(&old2), "older log line\n").unwrap();

        // delay=0：今天之前的都归档
        let writer = test_writer(dir.clone(), 0);
        let files = writer.collect_archive_targets().expect("应收集到待归档文件");
        RollingFileWriter::archive_files(files, dir.join("history"));

        // 原 .log 已被删除
        assert!(!dir.join(&old1).exists());
        assert!(!dir.join(&old2).exists());
        // history 目录下出现对应的 .gz
        let gz1 = dir.join(format!("history/{}.gz", old1));
        let gz2 = dir.join(format!("history/{}.gz", old2));
        assert!(gz1.exists(), "应生成 {}", gz1.display());
        assert!(gz2.exists(), "应生成 {}", gz2.display());
        // 压缩内容可正确还原
        assert_eq!(read_gz(&gz1), "old log line\n");
        assert_eq!(read_gz(&gz2), "older log line\n");
    }

    /// 测试：归档只处理匹配前缀的 .log，不影响其它文件
    #[test]
    fn archive_ignores_unrelated_files() {
        let dir = temp_dir("ignore");
        let old = dated_filename("ZeroDistance", 1);
        fs::write(dir.join(&old), "log").unwrap();
        fs::write(dir.join("other.log"), "not mine").unwrap(); // 前缀不匹配
        fs::write(dir.join("notes.txt"), "keep me").unwrap(); // 非 .log

        let writer = test_writer(dir.clone(), 0);
        let files = writer.collect_archive_targets().expect("应收集到待归档文件");
        // 只应归档匹配前缀的 .log（1 个），不碰 other.log / notes.txt
        assert_eq!(files.len(), 1);
        RollingFileWriter::archive_files(files, dir.join("history"));

        // 匹配前缀的 .log 被归档
        assert!(!dir.join(&old).exists());
        assert!(dir.join(format!("history/{}.gz", old)).exists());
        // 不匹配前缀的 .log 和非 .log 文件原样保留
        assert!(dir.join("other.log").exists());
        assert!(dir.join("notes.txt").exists());
    }

    /// 测试：首次运行（目录不存在）时启动不报错
    #[test]
    fn startup_without_existing_dir_is_ok() {
        let base = std::env::temp_dir().join(format!("zd_log_missing_{}", uuid::Uuid::new_v4()));
        // 目录尚未创建
        let _writer = RollingFileWriter::new(&base, "ZeroDistance", 0, 0, 0, 100, false, Tz::UTC).unwrap();
        // 归档检查应无异常，且日志目录被创建
        assert!(base.exists());
    }

    /// 测试：归档等待天数生效——delay=1 时昨天保留、前天归档
    #[test]
    fn archive_delay_keeps_recent_logs() {
        let dir = temp_dir("delay");
        let today = dated_filename("ZeroDistance", 0);      // 今天
        let yesterday = dated_filename("ZeroDistance", 1);  // 昨天
        let two_days_ago = dated_filename("ZeroDistance", 2); // 前天
        fs::write(dir.join(&today), "today\n").unwrap();
        fs::write(dir.join(&yesterday), "yesterday\n").unwrap();
        fs::write(dir.join(&two_days_ago), "two days ago\n").unwrap();

        // delay=1：昨天之前的（前天及更早）归档，今天、昨天保留
        let writer = test_writer(dir.clone(), 1);
        let files = writer.collect_archive_targets().expect("应收集到待归档文件");
        RollingFileWriter::archive_files(files, dir.join("history"));

        // 前天被归档
        assert!(!dir.join(&two_days_ago).exists());
        assert!(dir.join(format!("history/{}.gz", two_days_ago)).exists());
        // 今天、昨天保留在原目录
        assert!(dir.join(&today).exists());
        assert!(dir.join(&yesterday).exists());
    }

    /// 测试：单次归档数量受限，超过 archive_batch_size 只取最旧的 N 个
    #[test]
    fn archive_limits_batch_size() {
        let dir = temp_dir("limit");
        let writer = test_writer(dir.clone(), 0);
        let batch = writer.archive_batch_size;

        // 创建超过 batch 数量的历史文件
        let total = batch + 10;
        for i in 0..total {
            let name = dated_filename("ZeroDistance", i as i64 + 1);
            fs::write(dir.join(&name), "x").unwrap();
        }

        let files = writer.collect_archive_targets().expect("应收集到待归档文件");
        // 单次最多返回 archive_batch_size 个
        assert_eq!(files.len(), batch);
    }

    /// 测试：清理崩溃遗留的临时文件（.tmp），且不影响正常 .gz 与无关文件
    #[test]
    fn cleanup_removes_crash_tmp_files() {
        let dir = temp_dir("tmp");
        let history = dir.join("history");
        fs::create_dir_all(&history).unwrap();

        // 模拟崩溃遗留的临时文件
        let tmp1 = history.join("ZeroDistance.2026-08-20.log.gz.tmp.123.0");
        let tmp2 = history.join("ZeroDistance.2026-08-19.log.gz.tmp.123.1");
        fs::write(&tmp1, "partial").unwrap();
        fs::write(&tmp2, "partial").unwrap();
        // 正常归档文件应保留
        let gz = history.join("ZeroDistance.2026-08-18.log.gz");
        fs::write(&gz, "full").unwrap();
        // 无关文件（前缀不匹配）应保留
        let other = history.join("other.gz.tmp.999.0");
        fs::write(&other, "x").unwrap();

        RollingFileWriter::cleanup_archive_files(&history, "ZeroDistance", 10).unwrap();

        // 崩溃残留 .tmp 被清理
        assert!(!tmp1.exists());
        assert!(!tmp2.exists());
        // 正常 .gz 与无关 .tmp 保留
        assert!(gz.exists());
        assert!(other.exists());
    }

    /// 测试：extract_date / should_archive 的日期解析与归档判定
    #[test]
    fn date_parsing_and_archive_decision() {
        let today = now_in(Tz::UTC).date_naive();

        // 提取日期（无序号 / 有序号）
        assert_eq!(
            RollingFileWriter::extract_date("ZeroDistance.2026-08-20.log", "ZeroDistance"),
            NaiveDate::from_ymd_opt(2026, 8, 20)
        );
        assert_eq!(
            RollingFileWriter::extract_date("ZeroDistance.2026-08-20.3.log", "ZeroDistance"),
            NaiveDate::from_ymd_opt(2026, 8, 20)
        );

        // delay=0：昨天(diff=1)归档，今天(diff=0)不归档
        let yesterday = today - Duration::days(1);
        assert!(RollingFileWriter::should_archive(
            &format!("ZeroDistance.{}.log", yesterday.format("%Y-%m-%d")),
            "ZeroDistance",
            today,
            0
        ));
        assert!(!RollingFileWriter::should_archive(
            &format!("ZeroDistance.{}.log", today.format("%Y-%m-%d")),
            "ZeroDistance",
            today,
            0
        ));

        // delay=1：昨天(diff=1)不归档，前天(diff=2)归档
        assert!(!RollingFileWriter::should_archive(
            &format!("ZeroDistance.{}.log", yesterday.format("%Y-%m-%d")),
            "ZeroDistance",
            today,
            1
        ));
        let two_days_ago = today - Duration::days(2);
        assert!(RollingFileWriter::should_archive(
            &format!("ZeroDistance.{}.log", two_days_ago.format("%Y-%m-%d")),
            "ZeroDistance",
            today,
            1
        ));

        // 负数（滚动即归档）：今天的文件也归档
        assert!(RollingFileWriter::should_archive(
            &format!("ZeroDistance.{}.log", today.format("%Y-%m-%d")),
            "ZeroDistance",
            today,
            -1
        ));
    }
}

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

/// Atomic counter for generating unique temp filenames (avoids concurrent
/// archiver threads writing to the same `.tmp` file).
static NEXT_TMP_ID: AtomicU64 = AtomicU64::new(0);

/// Maximum number of concurrent archiver threads, to avoid thread explosion
/// under frequent rotation.
const MAX_CONCURRENT_ARCHIVERS: usize = 2;

/// Number of currently active archiver threads (for concurrency limiting).
static ACTIVE_ARCHIVERS: AtomicUsize = AtomicUsize::new(0);

/// Set of archiver thread handles, used for graceful shutdown (join) on exit.
static ARCHIVE_HANDLES: OnceLock<Mutex<Vec<std::thread::JoinHandle<()>>>> = OnceLock::new();

/// Returns the archiver thread handle set (lazily initialized).
fn archive_handles() -> &'static Mutex<Vec<std::thread::JoinHandle<()>>> {
    ARCHIVE_HANDLES.get_or_init(|| Mutex::new(Vec::new()))
}

/// Guard for the active archiver count: decrements the count when the thread
/// exits (including during panic unwinding).
struct ActiveArchiverGuard;

impl Drop for ActiveArchiverGuard {
    fn drop(&mut self) {
        ACTIVE_ARCHIVERS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Parses a timezone string into `chrono_tz::Tz`, falling back to UTC on failure.
pub fn parse_timezone(tz: &str) -> Tz {
    tz.parse::<Tz>().unwrap_or(Tz::UTC)
}

/// Returns the current time in the given timezone.
pub(crate) fn now_in(tz: Tz) -> DateTime<Tz> {
    Utc::now().with_timezone(&tz)
}

// ─────────────────────────────────────────────────────────────────────────────
// Custom rolling file writer: supports both date-based and size-based rotation.
// ─────────────────────────────────────────────────────────────────────────────

/// A log file writer supporting date- and size-based rolling.
///
/// File naming rules:
/// - default: `{prefix}.{YYYY-MM-DD}.log`
/// - size-exceeded: `{prefix}.{YYYY-MM-DD}.{seq}.log` (seq starts at 1)
pub struct RollingFileWriter {
    /// Directory where log files are stored.
    dir: PathBuf,
    /// Filename prefix.
    prefix: String,
    /// Maximum bytes per file (0 means no size limit).
    max_file_size: u64,
    /// Maximum number of log files to keep (0 means unlimited).
    max_files: usize,
    /// Archive delay in days: only archive logs dated earlier than
    /// "today - N days". Negative = archive immediately on rotation.
    archive_delay_days: i64,
    /// Maximum number of files to archive per pass.
    archive_batch_size: usize,
    /// Whether to force fsync on flush.
    fsync_on_flush: bool,
    /// Timezone used for log timestamps.
    timezone: Tz,
    /// Currently open file.
    current_file: Option<File>,
    /// Current date string (YYYY-MM-DD).
    current_date: String,
    /// Sequence number within the current date (0 = first file).
    current_seq: u32,
    /// Bytes already written to the current file.
    current_size: u64,
    /// Timestamp of the last date-change check.
    last_date_check: Instant,
    /// Date-check interval (seconds).
    date_check_secs: u64,
}

impl RollingFileWriter {
    /// Creates a new rolling file writer.
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
            last_date_check: Instant::now() - std::time::Duration::from_secs(5), // force first check
            date_check_secs: 5, // check date change every 5 seconds
        };
        // The first rotate_if_needed triggers rotation because current_date is
        // empty, at which point it performs "async archive of leftover history +
        // open today's file" in one place. No need to archive separately here.
        writer.rotate_if_needed()?;
        Ok(writer)
    }

    /// Generates the current log filename.
    fn make_filename(&self) -> String {
        if self.current_seq == 0 {
            format!("{}.{}.log", self.prefix, self.current_date)
        } else {
            format!("{}.{}.{}.log", self.prefix, self.current_date, self.current_seq)
        }
    }

    /// Checks whether rotation is needed and performs it if so.
    ///
    /// Performance optimizations:
    /// - size check: pure integer comparison, negligible cost
    /// - date check: involves time formatting (heap allocation), only performed
    ///   every `date_check_secs` seconds, using `Instant::now()` (zero-allocation,
    ///   nanosecond-level) to decide whether the check point is reached.
    fn rotate_if_needed(&mut self) -> io::Result<()> {
        // Size check: runs every time, extremely cheap.
        let size_exceeded =
            self.max_file_size > 0 && self.current_size >= self.max_file_size;

        // Date check: run at intervals to avoid allocating a String on every write.
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

        // No rotation needed and file already open: return early.
        if !date_changed && !size_exceeded && self.current_file.is_some() {
            return Ok(());
        }

        // Rotation needed: close the current file first (flush then drop the handle).
        if let Some(mut file) = self.current_file.take() {
            let _ = file.flush();
            drop(file);
        }

        // Date changed: reset the sequence number.
        if date_changed {
            self.current_seq = 0;
            crate::debug!("[rolling] date changed, new date: {}, seq reset to 0", self.current_date);
        } else if size_exceeded {
            // Size exceeded only: increment the sequence number.
            self.current_seq += 1;
            crate::debug!(
                "[rolling] size exceeded, current: {} bytes, limit: {} bytes, seq -> {}",
                self.current_size, self.max_file_size, self.current_seq
            );
        }

        // Ensure the directory exists.
        fs::create_dir_all(&self.dir)?;

        // Collect previous and leftover historical logs, then hand them to a
        // dedicated thread for async compression/archival (at this point the
        // current file is closed and the new one not yet opened, so all `.log`
        // files in the directory are historical). Archival does not block log
        // writes; after archiving, the same thread cleans up excess archives,
        // avoiding the cleanup-timing lag caused by "this pass's .gz not yet
        // generated".
        if let Some(files) = self.collect_archive_targets() {
            Self::spawn_archive_worker(
                files,
                self.dir.clone(),
                self.prefix.clone(),
                self.max_files,
            );
        }

        // Open the new file.
        let file_path = self.dir.join(self.make_filename());
        let _filename = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");
        crate::debug!("[rolling] opening new log file: {}", file_path.display());
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)?;

        // Get current file size (append from the end if the file already exists).
        let metadata = file.metadata()?;
        self.current_size = metadata.len();
        self.current_file = Some(file);
        crate::debug!(
            "[rolling] log file ready: {}, existing size: {} bytes",
            _filename, self.current_size
        );

        Ok(())
    }

    /// Collects the list of historical log files to archive (no compression,
    /// lightweight).
    ///
    /// Scans the log directory for `.log` files matching the prefix and dated
    /// earlier than "today - delay days", sorted by name ascending (oldest
    /// first), returning at most `archive_batch_size` entries. Returns `None`
    /// when there is nothing to archive.
    fn collect_archive_targets(&self) -> Option<Vec<PathBuf>> {
        // Log directory doesn't exist yet (first run): nothing to archive.
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

        // Archive oldest first (filenames contain YYYY-MM-DD dates, so
        // lexicographic order is chronological order — zero syscalls).
        files.sort();
        files.truncate(self.archive_batch_size);

        Some(files)
    }

    /// Gzip-compresses a batch of log files into the history directory
    /// (synchronous, idempotent).
    ///
    /// Each file is compressed to `history/{original-name}.gz`, then the original
    /// is deleted on success. A single file failing only warns and does not
    /// abort. Re-calling (source already deleted, target already exists) is safe.
    fn archive_files(files: Vec<PathBuf>, history_dir: PathBuf) {
        if let Err(_e) = fs::create_dir_all(&history_dir) {
            crate::warn!("[archive] failed to create archive dir {}: {}", history_dir.display(), _e);
            return;
        }

        for src in files {
            let filename = match src.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let gz_path = history_dir.join(format!("{}.gz", filename));

            // Source already handled by another archive task.
            if !src.exists() {
                continue;
            }
            // Target already exists: just clean up the leftover source (idempotent).
            if gz_path.exists() {
                if let Err(_e) = fs::remove_file(&src) {
                    crate::warn!("[archive] failed to remove source {}: {}", src.display(), _e);
                }
                continue;
            }

            match Self::compress_file(&src, &gz_path) {
                Ok(()) => {
                    if let Err(_e) = fs::remove_file(&src) {
                        crate::warn!("[archive] failed to remove source {}: {}", src.display(), _e);
                    }
                    crate::debug!("[archive] archived: {} -> {}", src.display(), gz_path.display());
                }
                Err(_e) => {
                    crate::warn!("[archive] failed to compress {}: {}", filename, _e);
                }
            }
        }
    }

    /// Asynchronously compresses and archives a batch of log files on a separate
    /// thread, then cleans up excess archives.
    ///
    /// Archival runs in the background and does not block the log-writing thread.
    /// After archiving, the same thread immediately cleans up excess `.gz` files,
    /// ensuring correct cleanup timing (at this point all archives from this pass
    /// have been generated).
    ///
    /// The thread is bounded by [`MAX_CONCURRENT_ARCHIVERS`]: exceeding the limit
    /// skips this pass (left for the next rotation) to avoid thread explosion
    /// under frequent rotation. Handles are stored in the global
    /// [`ARCHIVE_HANDLES`] for graceful shutdown (join) on exit.
    fn spawn_archive_worker(
        files: Vec<PathBuf>,
        dir: PathBuf,
        prefix: String,
        max_files: usize,
    ) {
        if files.is_empty() {
            return;
        }

        // Concurrency limit: skip this pass if the limit is reached.
        if ACTIVE_ARCHIVERS.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT_ARCHIVERS {
            ACTIVE_ARCHIVERS.fetch_sub(1, Ordering::SeqCst);
            crate::debug!("[archive] archiver threads at limit, skipping this pass");
            return;
        }

        let handle = std::thread::spawn(move || {
            // Decrement the active count when the thread exits (including panic).
            let _guard = ActiveArchiverGuard;
            let history_dir = dir.join("history");
            Self::archive_files(files, history_dir.clone());
            if max_files > 0 {
                let _ = Self::cleanup_archive_files(&history_dir, &prefix, max_files);
            }
        });

        // Save the handle for graceful shutdown.
        archive_handles().lock().unwrap().push(handle);
    }

    /// Parses a date out of a log filename.
    ///
    /// Filenames look like `{prefix}.{YYYY-MM-DD}.log` or
    /// `{prefix}.{YYYY-MM-DD}.{seq}.log`; takes the first 10 characters
    /// (YYYY-MM-DD) between the prefix and the `.log` suffix.
    fn extract_date(filename: &str, prefix: &str) -> Option<NaiveDate> {
        let prefix_dot = format!("{}.", prefix);
        let rest = filename.strip_prefix(&prefix_dot)?;
        let rest = rest.strip_suffix(".log")?;
        let date_str = rest.get(0..10)?;
        NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()
    }

    /// Decides whether a log file should be archived.
    ///
    /// Rules:
    /// - delay < 0: archive immediately on rotation (ignore date).
    /// - delay >= 0: archive when the file's date is earlier than
    ///   "today - delay days" (day difference > delay). delay=0 archives
    ///   yesterday and earlier, keeps today; delay=1 archives the day before
    ///   yesterday and earlier, keeps today and yesterday.
    /// Abnormally-named files that can't be parsed are treated as archivable to
    /// avoid indefinite retention.
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

    /// Gzip-compresses a single file to the target path (atomic write, crash-safe).
    ///
    /// Writes to a unique temp file `{dst}.tmp.{pid}.{seq}` first, then after
    /// compression does `flush` + `sync_all` to ensure data is durable, and
    /// finally `rename`s atomically to the target `.gz`. This way:
    /// - once the target `.gz` exists, it is necessarily **complete** (no partial);
    /// - a mid-compression failure/crash only leaves a `.tmp`, leaving both the
    ///   original `.log` and any existing `.gz` unaffected.
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
            // finish returns the underlying writer, used for flush + sync_all.
            let mut file = encoder.finish()?;
            file.flush()?;
            file.sync_all()?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                // Data is fully durable; atomically replace the target .gz.
                fs::rename(&tmp, dst)?;
                Ok(())
            }
            Err(e) => {
                // Clean up the temp file on failure to avoid leftovers.
                let _ = fs::remove_file(&tmp);
                Err(e)
            }
        }
    }

    /// Cleans up archive files exceeding the retention limit (`.gz` under history).
    ///
    /// Deletes archives exceeding `max_files`, oldest first (filename ascending).
    /// Designed as a static method so the archiver thread can call it right after
    /// archiving, ensuring correct cleanup timing.
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

        // Collect all matching archive files.
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

        // Clean up leftover temp files from crashes (`{prefix}.*.gz.tmp.{pid}.{seq}`).
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
            crate::debug!("[cleanup] removed {} leftover temp files", tmp_removed);
        }

        // Sort by filename ascending (oldest first) — filenames contain
        // date+seq, so lexicographic order is chronological order.
        archived.sort();

        // Delete old archives beyond the limit.
        let total_count = archived.len();
        while archived.len() > max_files {
            if let Some(oldest) = archived.first() {
                let _filename = oldest.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");
                crate::debug!("[cleanup] removing expired archive: {}", _filename);
                let _ = fs::remove_file(oldest);
            }
            archived.remove(0);
        }
        let removed = total_count - archived.len();
        if removed > 0 {
            crate::debug!("[cleanup] done, removed {} expired archives", removed);
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
            Err(io::Error::new(io::ErrorKind::Other, "log file not open"))
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
// Logging initialization
// ─────────────────────────────────────────────────────────────────────────────

/// Waits for all active archiver threads to finish (graceful shutdown).
///
/// Call before program exit to avoid archiver threads being forcibly terminated
/// and leaving half-written files. Combined with `compress_file`'s atomic write,
/// it guarantees archives are either complete or absent.
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

    /// Creates an isolated temp directory for tests.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("zd_log_{}_{}", tag, uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Generates a log filename "N days ago" (form `{prefix}.{YYYY-MM-DD}.log`,
    /// fixed UTC timezone).
    fn dated_filename(prefix: &str, days_ago: i64) -> String {
        let date = now_in(Tz::UTC).date_naive() - Duration::days(days_ago);
        format!("{}.{}.log", prefix, date.format("%Y-%m-%d"))
    }

    /// Decompresses a gzip file's contents into a string.
    fn read_gz(path: &Path) -> String {
        let file = File::open(path).unwrap();
        let mut decoder = flate2::read::GzDecoder::new(file);
        let mut s = String::new();
        decoder.read_to_string(&mut s).unwrap();
        s
    }

    /// Constructs a writer directly (bypassing `new`'s async archival, for
    /// synchronous control over archival timing in tests).
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

    /// Test: archive leftover historical logs (compress + delete source + move to history/).
    #[test]
    fn archives_history_logs() {
        let dir = temp_dir("startup");
        let old1 = dated_filename("ZeroDistance", 1); // yesterday
        let old2 = dated_filename("ZeroDistance", 2); // two days ago
        fs::write(dir.join(&old1), "old log line\n").unwrap();
        fs::write(dir.join(&old2), "older log line\n").unwrap();

        // delay=0: archive everything before today.
        let writer = test_writer(dir.clone(), 0);
        let files = writer.collect_archive_targets().expect("should collect archive targets");
        RollingFileWriter::archive_files(files, dir.join("history"));

        // Original .log files deleted.
        assert!(!dir.join(&old1).exists());
        assert!(!dir.join(&old2).exists());
        // Corresponding .gz files appear under history/.
        let gz1 = dir.join(format!("history/{}.gz", old1));
        let gz2 = dir.join(format!("history/{}.gz", old2));
        assert!(gz1.exists(), "should generate {}", gz1.display());
        assert!(gz2.exists(), "should generate {}", gz2.display());
        // Compressed contents decompress correctly.
        assert_eq!(read_gz(&gz1), "old log line\n");
        assert_eq!(read_gz(&gz2), "older log line\n");
    }

    /// Test: archiving only handles prefix-matching .log files, leaving others alone.
    #[test]
    fn archive_ignores_unrelated_files() {
        let dir = temp_dir("ignore");
        let old = dated_filename("ZeroDistance", 1);
        fs::write(dir.join(&old), "log").unwrap();
        fs::write(dir.join("other.log"), "not mine").unwrap(); // prefix mismatch
        fs::write(dir.join("notes.txt"), "keep me").unwrap(); // not .log

        let writer = test_writer(dir.clone(), 0);
        let files = writer.collect_archive_targets().expect("should collect archive targets");
        // Only the prefix-matching .log (1 file) should be archived; leave other.log / notes.txt.
        assert_eq!(files.len(), 1);
        RollingFileWriter::archive_files(files, dir.join("history"));

        // Prefix-matching .log archived.
        assert!(!dir.join(&old).exists());
        assert!(dir.join(format!("history/{}.gz", old)).exists());
        // Non-matching .log and non-.log files preserved.
        assert!(dir.join("other.log").exists());
        assert!(dir.join("notes.txt").exists());
    }

    /// Test: startup with a non-existent directory does not error.
    #[test]
    fn startup_without_existing_dir_is_ok() {
        let base = std::env::temp_dir().join(format!("zd_log_missing_{}", uuid::Uuid::new_v4()));
        // Directory not yet created.
        let _writer = RollingFileWriter::new(&base, "ZeroDistance", 0, 0, 0, 100, false, Tz::UTC).unwrap();
        // Archive check should not error, and the log directory should be created.
        assert!(base.exists());
    }

    /// Test: archive delay works — delay=1 keeps yesterday, archives two-days-ago.
    #[test]
    fn archive_delay_keeps_recent_logs() {
        let dir = temp_dir("delay");
        let today = dated_filename("ZeroDistance", 0);      // today
        let yesterday = dated_filename("ZeroDistance", 1);  // yesterday
        let two_days_ago = dated_filename("ZeroDistance", 2); // two days ago
        fs::write(dir.join(&today), "today\n").unwrap();
        fs::write(dir.join(&yesterday), "yesterday\n").unwrap();
        fs::write(dir.join(&two_days_ago), "two days ago\n").unwrap();

        // delay=1: archive before yesterday (two-days-ago and earlier), keep today and yesterday.
        let writer = test_writer(dir.clone(), 1);
        let files = writer.collect_archive_targets().expect("should collect archive targets");
        RollingFileWriter::archive_files(files, dir.join("history"));

        // Two-days-ago archived.
        assert!(!dir.join(&two_days_ago).exists());
        assert!(dir.join(format!("history/{}.gz", two_days_ago)).exists());
        // Today and yesterday kept in the original directory.
        assert!(dir.join(&today).exists());
        assert!(dir.join(&yesterday).exists());
    }

    /// Test: single-pass archive count is limited; exceeding archive_batch_size
    /// takes only the oldest N.
    #[test]
    fn archive_limits_batch_size() {
        let dir = temp_dir("limit");
        let writer = test_writer(dir.clone(), 0);
        let batch = writer.archive_batch_size;

        // Create more historical files than the batch size.
        let total = batch + 10;
        for i in 0..total {
            let name = dated_filename("ZeroDistance", i as i64 + 1);
            fs::write(dir.join(&name), "x").unwrap();
        }

        let files = writer.collect_archive_targets().expect("should collect archive targets");
        // At most archive_batch_size files per pass.
        assert_eq!(files.len(), batch);
    }

    /// Test: cleanup removes crash-leftover temp files (.tmp) without touching
    /// valid .gz and unrelated files.
    #[test]
    fn cleanup_removes_crash_tmp_files() {
        let dir = temp_dir("tmp");
        let history = dir.join("history");
        fs::create_dir_all(&history).unwrap();

        // Simulate crash-leftover temp files.
        let tmp1 = history.join("ZeroDistance.2026-08-20.log.gz.tmp.123.0");
        let tmp2 = history.join("ZeroDistance.2026-08-19.log.gz.tmp.123.1");
        fs::write(&tmp1, "partial").unwrap();
        fs::write(&tmp2, "partial").unwrap();
        // Valid archive files should be preserved.
        let gz = history.join("ZeroDistance.2026-08-18.log.gz");
        fs::write(&gz, "full").unwrap();
        // Unrelated files (prefix mismatch) should be preserved.
        let other = history.join("other.gz.tmp.999.0");
        fs::write(&other, "x").unwrap();

        RollingFileWriter::cleanup_archive_files(&history, "ZeroDistance", 10).unwrap();

        // Crash-leftover .tmp files cleaned up.
        assert!(!tmp1.exists());
        assert!(!tmp2.exists());
        // Valid .gz and unrelated .tmp preserved.
        assert!(gz.exists());
        assert!(other.exists());
    }

    /// Test: date parsing and archive decision of extract_date / should_archive.
    #[test]
    fn date_parsing_and_archive_decision() {
        let today = now_in(Tz::UTC).date_naive();

        // Extract date (with and without sequence number).
        assert_eq!(
            RollingFileWriter::extract_date("ZeroDistance.2026-08-20.log", "ZeroDistance"),
            NaiveDate::from_ymd_opt(2026, 8, 20)
        );
        assert_eq!(
            RollingFileWriter::extract_date("ZeroDistance.2026-08-20.3.log", "ZeroDistance"),
            NaiveDate::from_ymd_opt(2026, 8, 20)
        );

        // delay=0: yesterday (diff=1) archived, today (diff=0) not.
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

        // delay=1: yesterday (diff=1) not archived, two-days-ago (diff=2) archived.
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

        // Negative (archive on rotation): today's file is also archived.
        assert!(RollingFileWriter::should_archive(
            &format!("ZeroDistance.{}.log", today.format("%Y-%m-%d")),
            "ZeroDistance",
            today,
            -1
        ));
    }
}

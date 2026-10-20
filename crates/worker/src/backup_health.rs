//! worker/backup_health — age of the last successful backup.
//!
//! Why a file stamp and not a database row: backups are taken by a DIFFERENT
//! container (`deploy/backup/backupd.sh`, the `backup` compose profile), which
//! is a shell script with no OTLP exporter and no Rust dependency. The one
//! thing it can leave behind that a core process can read is a file. So the
//! runner writes one epoch-second stamp after a pass that did not fail, and
//! this module turns it into `marula.backup.age_seconds`.
//!
//! The hook is OPT-IN by path, deliberately. There is no sensible default: a
//! missing stamp must not read as "fresh backup" and must not read as "very
//! old backup" either. With `MARULA_BACKUP_STAMP_PATH` unset — the shipped
//! compose file does not mount `backupdumps` into the worker, so this is the
//! default today — the gauge is simply not emitted, and the report says why.
//! A series that is absent is a question an operator can ask; a series pinned
//! at zero is a lie they cannot see through.
//!
//! The stamp's format is one line: a UNIX epoch second count, decimal. That is
//! what `date +%s` produces, which keeps the producer a one-liner in a POSIX
//! shell with no JSON, no timezone handling and no parsing bugs.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Environment variable naming the stamp file the backup runner writes.
pub const STAMP_VAR: &str = "MARULA_BACKUP_STAMP_PATH";

/// The file path this process is configured to read, if any.
#[must_use]
pub fn stamp_path() -> Option<PathBuf> {
    std::env::var(STAMP_VAR)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Age in seconds of the timestamp in `contents`, relative to `now_secs`.
///
/// Returns `None` for anything that is not a post-1970 epoch second — an
/// empty file, a truncated write, a JSON blob someone put there by mistake.
/// A future timestamp clamps to zero rather than wrapping: a backup stamped
/// by a clock that is ahead of the reader is "just now", not 58 billion years
/// away.
#[must_use]
pub fn age_from_stamp(contents: &str, now_secs: u64) -> Option<u64> {
    let stamp: u64 = contents.trim().parse().ok()?;
    Some(now_secs.saturating_sub(stamp))
}

/// Reads the stamp at `path` and returns its age, or `None` when the file is
/// missing, unreadable or malformed.
///
/// A missing file is the normal state before the first backup and after a
/// volume loss; both deserve absence, not an error.
#[must_use]
pub fn read_age(path: &Path, now_secs: u64) -> Option<u64> {
    let contents = std::fs::read_to_string(path).ok()?;
    age_from_stamp(&contents, now_secs)
}

/// Samples the configured stamp and emits the gauge. No-op when the path is
/// unset or the stamp cannot be read.
///
/// The read is a single small local file. It runs at most once per queue tick
/// (>= the queue's idle interval), so it is not on a hot path; the timestamp
/// arithmetic happens in [`age_from_stamp`], which is where the tests are.
pub fn sample() {
    let Some(path) = stamp_path() else {
        return;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match read_age(&path, now) {
        Some(age) => core_otel::metrics::backup_age(age),
        None => {
            tracing::debug!(
                path = %path.display(),
                "backup stamp unreadable or absent; backup age not reported"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stamp_yields_the_age_since_that_backup() {
        assert_eq!(age_from_stamp("1700000000\n", 1_700_003_600), Some(3600));
        assert_eq!(age_from_stamp("  1700000000  ", 1_700_000_000), Some(0));
    }

    #[test]
    fn garbage_and_empty_stamps_are_absent_not_zero() {
        for bad in ["", "\n", "not-a-number", "{\"at\":1700000000}", "-5"] {
            assert_eq!(
                age_from_stamp(bad, 1_700_000_000),
                None,
                "{bad:?} must not become an age"
            );
        }
    }

    #[test]
    fn a_future_stamp_clamps_to_zero_instead_of_wrapping() {
        assert_eq!(age_from_stamp("1700009999", 1_700_000_000), Some(0));
    }

    #[test]
    fn a_missing_stamp_is_absent() {
        let dir = std::env::temp_dir().join(format!("marula-backup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let missing = dir.join("never-written");
        let _ = std::fs::remove_file(&missing);
        assert_eq!(read_age(&missing, 1_700_000_000), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_written_stamp_is_read_back() {
        let dir = std::env::temp_dir().join(format!("marula-backup-rw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("last-success");
        std::fs::write(&path, "1700000000\n").expect("write stamp");
        assert_eq!(read_age(&path, 1_700_007_200), Some(7200));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The path is opt-in: an unset or blank variable means this build does
    /// not report backup age at all. (Serial: env is process-global.)
    #[test]
    #[serial_test::serial]
    fn an_unset_or_blank_path_disables_the_probe() {
        unsafe { std::env::remove_var(STAMP_VAR) };
        assert_eq!(stamp_path(), None);
        unsafe { std::env::set_var(STAMP_VAR, "   ") };
        assert_eq!(stamp_path(), None);
        unsafe { std::env::set_var(STAMP_VAR, "/var/lib/marula/.last-success") };
        assert_eq!(
            stamp_path(),
            Some(PathBuf::from("/var/lib/marula/.last-success"))
        );
        unsafe { std::env::remove_var(STAMP_VAR) };
    }
}

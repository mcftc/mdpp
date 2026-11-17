//! Crash reports (PLAN §4.4): a panic writes a plain-text report to `<data>/logs/`; the next start
//! mentions it once. Nothing is uploaded — the user decides what to share (PLAN principle 5).
//! Unsaved work is covered separately by hot exit, which backs up every few seconds.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Reports kept; older ones are deleted when a new one is written.
pub const MAX_REPORTS: usize = 10;
const PREFIX: &str = "crash-";
/// Name of the newest report the user has already been told about.
const SEEN_FILE: &str = ".seen";

/// Install the panic hook. The default hook still runs (it prints to stderr).
pub fn install(log_dir: PathBuf, version: &'static str) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()));
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".to_string());
        let report = format_report(
            version,
            thread.name().unwrap_or("unnamed"),
            &message,
            location.as_deref(),
            &std::backtrace::Backtrace::force_capture().to_string(),
        );
        // Best effort: a failing write must not hide the original panic.
        let _ = write_report(&log_dir, &report);
        default_hook(info);
    }));
}

fn format_report(
    version: &str,
    thread: &str,
    message: &str,
    location: Option<&str>,
    backtrace: &str,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "MD++ {version} crashed");
    let _ = writeln!(
        out,
        "os: {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let _ = writeln!(out, "time: {} (seconds since 1970, UTC)", unix_seconds());
    let _ = writeln!(out, "thread: {thread}");
    let _ = writeln!(out, "location: {}", location.unwrap_or("unknown"));
    let _ = writeln!(out, "message: {message}");
    let _ = writeln!(out, "\nbacktrace:\n{backtrace}");
    out
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Write a report and prune old ones. Returns the report's path.
pub fn write_report(log_dir: &Path, report: &str) -> std::io::Result<PathBuf> {
    fs::create_dir_all(log_dir)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let path = log_dir.join(format!("{PREFIX}{nanos:024}-{}.log", std::process::id()));
    fs::write(&path, report)?;
    for old in reports(log_dir).into_iter().rev().skip(MAX_REPORTS) {
        let _ = fs::remove_file(old);
    }
    Ok(path)
}

/// Reports, oldest first (names sort by time).
fn reports(log_dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(log_dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(PREFIX))
        })
        .collect();
    found.sort();
    found
}

/// The newest report the user has not been told about yet; marks it as told.
pub fn take_unseen(log_dir: &Path) -> Option<PathBuf> {
    let newest = reports(log_dir).pop()?;
    let name = newest.file_name()?.to_string_lossy().into_owned();
    let seen = log_dir.join(SEEN_FILE);
    if fs::read_to_string(&seen).ok().as_deref() == Some(name.as_str()) {
        return None;
    }
    let _ = fs::write(seen, name);
    Some(newest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_are_written_pruned_and_announced_once() {
        let dir = std::env::temp_dir().join(format!("mdpp-crash-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(take_unseen(&dir), None);
        let report = format_report("0.0.1", "main", "boom", Some("src/x.rs:3"), "frames");
        assert!(report.contains("message: boom") && report.contains("location: src/x.rs:3"));
        let mut last = PathBuf::new();
        for _ in 0..MAX_REPORTS + 3 {
            last = write_report(&dir, &report).unwrap();
        }
        assert_eq!(reports(&dir).len(), MAX_REPORTS);
        assert_eq!(take_unseen(&dir), Some(last));
        assert_eq!(take_unseen(&dir), None); // told once
        let _ = fs::remove_dir_all(dir);
    }
}

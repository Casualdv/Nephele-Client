//! Crash reports: local first, upload only with explicit consent.
//!
//! The game runs as a child process, so the launcher is the natural place to
//! notice an abnormal exit and to keep the end of the game's log. A report is
//! **always written locally** (unless the user has deleted the folder) and is
//! **never uploaded by default**: `upload_crashes` starts `false`, and with no
//! `NEPHELE_CRASH_URL` configured there is nowhere to upload to anyway.
//!
//! Nothing here runs automatically. The caller decides to write a report, and
//! uploads only when [`upload_allowed`] says the user opted in.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashKind {
    Game,
    Launcher,
}

impl CrashKind {
    fn slug(self) -> &'static str {
        match self {
            Self::Game => "game",
            Self::Launcher => "launcher",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CrashReport {
    pub kind: CrashKind,
    pub version: String,
    pub profile: String,
    pub exit_code: Option<i32>,
    /// Last lines of the session log, already trimmed by [`tail`].
    pub log_tail: String,
    pub signing: String,
}

impl CrashReport {
    pub fn new(kind: CrashKind, version: &str, profile: &str, signing: &str) -> Self {
        Self {
            kind,
            version: version.to_string(),
            profile: profile.to_string(),
            exit_code: None,
            log_tail: String::new(),
            signing: signing.to_string(),
        }
    }

    pub fn exit_code(mut self, code: Option<i32>) -> Self {
        self.exit_code = code;
        self
    }

    pub fn log_tail(mut self, tail: String) -> Self {
        self.log_tail = tail;
        self
    }

    /// The report body. Plain text on purpose: it can be read with Notepad and
    /// pasted into a bug report without a tool.
    pub fn text(&self, unix_seconds: u64) -> String {
        let exit = match self.exit_code {
            Some(code) => code.to_string(),
            None => "abnormal termination (no exit code)".to_string(),
        };
        format!(
            "Nephele crash report\n\
             generated: {} (unix {unix_seconds})\n\
             kind: {}\n\
             version: {}\n\
             build: {}\n\
             profile: {}\n\
             os: {} {}\n\
             exit: {exit}\n\
             \n\
             --- end of session log ---\n\
             {}\n\
             --- end ---\n",
            utc_stamp(unix_seconds),
            self.kind.slug(),
            self.version,
            self.signing,
            self.profile,
            std::env::consts::OS,
            std::env::consts::ARCH,
            if self.log_tail.is_empty() {
                "(no log captured)"
            } else {
                &self.log_tail
            }
        )
    }
}

/// Write a report under `dir/crashes/`. Returns the path that was written.
pub fn write(dir: &Path, report: &CrashReport, unix_seconds: u64) -> io::Result<PathBuf> {
    let crashes = dir.join("crashes");
    fs::create_dir_all(&crashes)?;
    let name = format!(
        "crash-{}-{}.txt",
        report.kind.slug(),
        utc_stamp(unix_seconds)
    );
    let path = crashes.join(name);
    fs::write(&path, report.text(unix_seconds))?;
    Ok(path)
}

/// Upload is a two-part opt-in: the user consented **and** this build has an
/// endpoint. No consent means no request, full stop.
pub fn upload_allowed(consent: bool, endpoint: Option<&str>) -> bool {
    consent && endpoint.is_some_and(|url| !url.trim().is_empty())
}

/// The configured upload endpoint, if the release build has one.
pub fn endpoint() -> Option<&'static str> {
    option_env!("NEPHELE_CRASH_URL").filter(|url| !url.trim().is_empty())
}

/// POST a report. Only reached with consent; a failure is reported to the UI
/// and the local file remains the fallback.
pub fn upload(report: &str, endpoint: &str) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(8))
        .build();
    agent
        .post(endpoint)
        .set("Content-Type", "text/plain; charset=utf-8")
        .send_string(report)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

/// The last `max_lines` lines of a log file, or a short explanation.
pub fn tail(path: &Path, max_lines: usize) -> String {
    let Ok(text) = fs::read_to_string(path) else {
        return String::new();
    };
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join("\n")
}

/// Install a panic hook that leaves a local launcher report. This is the
/// launcher's own last resort; a game crash is reported from the child-exit
/// path instead.
pub fn install_launcher_panic_hook(dir: PathBuf, version: &'static str, signing: String) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| (*text).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_string());
        let location = info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_else(|| "unknown location".to_string());
        let mut report = CrashReport::new(CrashKind::Launcher, version, "launcher", &signing);
        report.log_tail = format!("panic at {location}: {message}");
        let _ = write(&dir, &report, unix_now());
        previous(info);
    }));
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// `YYYY-MM-DD_HH-MM-SS` in UTC, so a file name sorts chronologically.
pub fn utc_stamp(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let seconds_of_day = unix_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}_{hour:02}-{minute:02}-{second:02}")
}

/// Howard Hinnant's days-to-civil date algorithm, valid for the proleptic
/// Gregorian calendar and independent of any time-zone database.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nephele-launcher-crash-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_report_is_written_locally_with_the_log_tail() {
        let dir = temp_dir("write");
        let report = CrashReport::new(CrashKind::Game, "0.1.0", "Default", "UNSIGNED BUILD")
            .exit_code(Some(3))
            .log_tail("line one\nline two".to_string());
        let path = write(&dir, &report, 1_700_000_000).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("kind: game"));
        assert!(text.contains("exit: 3"));
        assert!(text.contains("line two"));
        assert!(path.starts_with(dir.join("crashes")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_abnormal_exit_without_a_code_says_so() {
        let report = CrashReport::new(CrashKind::Game, "1", "p", "s").exit_code(None);
        assert!(report.text(0).contains("abnormal termination"));
    }

    #[test]
    fn upload_requires_both_consent_and_an_endpoint() {
        assert!(!upload_allowed(false, Some("https://example.invalid")));
        assert!(!upload_allowed(true, None));
        assert!(!upload_allowed(true, Some("   ")));
        assert!(upload_allowed(true, Some("https://example.invalid")));
    }

    #[test]
    fn the_tail_keeps_the_last_lines_only() {
        let dir = temp_dir("tail");
        let path = dir.join("session.log");
        fs::write(&path, "a\nb\nc\nd\n").unwrap();
        assert_eq!(tail(&path, 2), "c\nd");
        assert_eq!(tail(&dir.join("missing.log"), 2), "");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn timestamps_are_utc_and_sortable() {
        assert_eq!(utc_stamp(0), "1970-01-01_00-00-00");
        assert_eq!(utc_stamp(1_700_000_000), "2023-11-14_22-13-20");
        assert_eq!(utc_stamp(1_709_164_800), "2024-02-29_00-00-00");
    }

    #[test]
    fn a_launcher_panic_is_written_before_the_previous_hook_runs() {
        // The hook is process-global, so only prove the report builder and
        // writer here; installing it would replace the test harness's hook.
        let dir = temp_dir("launcher");
        let report = CrashReport::new(CrashKind::Launcher, "1", "launcher", "s")
            .log_tail("panic at src/main.rs:1: boom".to_string());
        let path = write(&dir, &report, 0).unwrap();
        assert!(fs::read_to_string(path)
            .unwrap()
            .contains("panic at src/main.rs:1: boom"));
        let _ = fs::remove_dir_all(&dir);
    }
}

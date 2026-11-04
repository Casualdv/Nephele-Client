use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const MAX_LOG_LINES: usize = 80;
const MAX_REPORT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashReport {
    pub report_id: String,
    pub launcher_version: String,
    pub client_version: String,
    pub platform: String,
    pub exit_code: Option<i32>,
    pub occurred_at_unix: u64,
    pub log_excerpt: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CrashIncident {
    pub report: CrashReport,
    pub local_path: PathBuf,
    pub upload_status: Option<String>,
}

#[derive(Debug, Default)]
pub struct LogBuffer {
    lines: VecDeque<String>,
}

impl LogBuffer {
    pub fn push(&mut self, line: impl AsRef<str>) {
        let sanitized = redact_sensitive(line.as_ref());
        if sanitized.trim().is_empty() {
            return;
        }
        self.lines.push_back(sanitized.chars().take(500).collect());
        while self.lines.len() > MAX_LOG_LINES {
            self.lines.pop_front();
        }
    }

    pub fn snapshot(&self) -> Vec<String> {
        self.lines.iter().cloned().collect()
    }
}

impl CrashReport {
    pub fn new(client_version: &str, exit_code: Option<i32>, logs: Vec<String>) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            report_id: format!("{}-{now}", std::process::id()),
            launcher_version: env!("CARGO_PKG_VERSION").to_string(),
            client_version: client_version.to_string(),
            platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            exit_code,
            occurred_at_unix: now,
            log_excerpt: logs
                .into_iter()
                .rev()
                .take(MAX_LOG_LINES)
                .map(|line| redact_sensitive(&line))
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect(),
        }
    }

    pub fn save(&self, data_dir: &Path) -> Result<PathBuf, String> {
        let directory = data_dir.join("crash-reports");
        std::fs::create_dir_all(&directory)
            .map_err(|err| format!("Could not create local crash report folder: {err}"))?;
        let path = directory.join(format!("{}.json", self.report_id));
        let json = serde_json::to_vec_pretty(self)
            .map_err(|err| format!("Could not encode crash report: {err}"))?;
        if json.len() > MAX_REPORT_BYTES {
            return Err("Crash report exceeded the local size limit".into());
        }
        std::fs::write(&path, json)
            .map_err(|err| format!("Could not write local crash report: {err}"))?;
        Ok(path)
    }
}

pub fn upload(report: &CrashReport) -> Result<String, String> {
    let endpoint = option_env!("NEPHELE_CRASH_REPORT_URL")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Crash uploads are not configured for this build".to_string())?;
    if !endpoint.starts_with("https://") {
        return Err("Crash report endpoint must use HTTPS".into());
    }
    let body = serde_json::to_vec(report)
        .map_err(|err| format!("Could not encode crash report: {err}"))?;
    if body.len() > MAX_REPORT_BYTES {
        return Err("Crash report exceeded the upload size limit".into());
    }
    let response = ureq::post(endpoint)
        .set("Content-Type", "application/json")
        .set(
            "User-Agent",
            &format!("NepheleLauncher/{}", env!("CARGO_PKG_VERSION")),
        )
        .send_bytes(&body)
        .map_err(|err| format!("Crash report upload failed: {err}"))?;
    if !(200..300).contains(&response.status()) {
        return Err(format!(
            "Crash report service returned HTTP {}",
            response.status()
        ));
    }
    Ok("Crash report sent. Thank you for helping improve Nephele.".into())
}

fn redact_sensitive(input: &str) -> String {
    let mut output = input.to_string();
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let home = home.to_string_lossy();
        if !home.is_empty() {
            output = output.replace(home.as_ref(), "<USER_HOME>");
            output = output.replace(&home.to_lowercase(), "<USER_HOME>");
        }
    }
    for marker in ["access_token", "refresh_token", "Authorization", "Bearer "] {
        output = redact_value_after_marker(&output, marker);
    }
    output
}

fn redact_value_after_marker(input: &str, marker: &str) -> String {
    let Some(index) = input
        .to_ascii_lowercase()
        .find(&marker.to_ascii_lowercase())
    else {
        return input.to_string();
    };
    let start = index + marker.len();
    let tail = &input[start..];
    let value_start = tail
        .char_indices()
        .find(|(_, ch)| !matches!(ch, ' ' | '\t' | ':' | '=' | '"' | '\''))
        .map(|(index, _)| index);
    let Some(value_start) = value_start else {
        return input.to_string();
    };
    let value = &tail[value_start..];
    if marker.eq_ignore_ascii_case("Authorization") {
        return format!("{}<REDACTED>", &input[..start + value_start]);
    }
    let value_end = value
        .find([' ', '\t', ',', '"', '\'', ';'])
        .unwrap_or(value.len());
    if value_end == 0 {
        return input.to_string();
    }
    format!(
        "{}<REDACTED>{}",
        &input[..start + value_start],
        &value[value_end..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_buffer_is_bounded_and_redacts_credentials() {
        let mut logs = LogBuffer::default();
        logs.push("access_token=super-secret");
        for index in 0..200 {
            logs.push(format!("line {index}"));
        }
        let snapshot = logs.snapshot();
        assert_eq!(snapshot.len(), MAX_LOG_LINES);
        assert!(!snapshot.iter().any(|line| line.contains("super-secret")));
        assert!(snapshot.last().unwrap().contains("line 199"));
    }

    #[test]
    fn report_is_local_and_has_no_upload_side_effect() {
        let report = CrashReport::new("1.2.3", Some(1), vec!["client stopped".into()]);
        let root = std::env::temp_dir().join(format!("nephele-crash-test-{}", report.report_id));
        let path = report.save(&root).unwrap();
        let saved: CrashReport = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(saved.client_version, "1.2.3");
        assert_eq!(saved.exit_code, Some(1));
        assert_eq!(saved.log_excerpt, vec!["client stopped"]);
        assert_eq!(
            report.platform,
            format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sensitive_markers_redact_the_following_value() {
        assert_eq!(
            redact_sensitive("Authorization: Bearer abc123"),
            "Authorization: <REDACTED>"
        );
        assert_eq!(
            redact_sensitive("refresh_token=long-token"),
            "refresh_token=<REDACTED>"
        );
    }
}

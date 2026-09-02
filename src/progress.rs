use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{Deserialize, Serialize};

const EVENT_LIMIT: usize = 20;
const WRITE_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ProgressEvent {
    pub at: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct ProgressSnapshot {
    pub job_id: Option<i64>,
    pub job_kind: Option<String>,
    pub state: String,
    pub message: String,
    pub current: Option<u64>,
    pub total: Option<u64>,
    pub total_is_estimate: bool,
    pub unit: Option<String>,
    pub updated_at: String,
    #[serde(default)]
    pub events: Vec<ProgressEvent>,
}

pub(crate) struct ProgressTracker {
    path: Option<PathBuf>,
    snapshot: ProgressSnapshot,
    last_write: Instant,
    write_error_reported: bool,
}

impl ProgressTracker {
    pub(crate) fn disabled() -> Self {
        Self {
            path: None,
            snapshot: ProgressSnapshot::default(),
            last_write: Instant::now(),
            write_error_reported: false,
        }
    }

    pub(crate) fn start(config_path: &Path, job_id: i64, job_kind: &str) -> Self {
        let path = progress_path(config_path);
        let mut snapshot = read_path(&path).unwrap_or_default();
        snapshot.job_id = Some(job_id);
        snapshot.job_kind = Some(job_kind.to_owned());
        snapshot.state = "running".into();
        snapshot.message = format!("Starting {job_kind}");
        snapshot.current = None;
        snapshot.total = None;
        snapshot.total_is_estimate = false;
        snapshot.unit = None;
        append_event(&mut snapshot, format!("Started {job_kind}"));
        let mut tracker = Self {
            path: Some(path),
            snapshot,
            last_write: Instant::now(),
            write_error_reported: false,
        };
        tracker.write(true);
        tracker
    }

    pub(crate) fn phase(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.snapshot.message.clone_from(&message);
        self.snapshot.current = None;
        self.snapshot.total = None;
        self.snapshot.total_is_estimate = false;
        self.snapshot.unit = None;
        append_event(&mut self.snapshot, message);
        self.write(true);
    }

    pub(crate) fn progress(
        &mut self,
        message: impl Into<String>,
        current: u64,
        total: Option<u64>,
        total_is_estimate: bool,
        unit: &str,
    ) {
        self.snapshot.message = message.into();
        self.snapshot.current = Some(current);
        self.snapshot.total = total;
        self.snapshot.total_is_estimate = total_is_estimate;
        self.snapshot.unit = Some(unit.into());
        self.write(false);
    }

    pub(crate) fn event(&mut self, message: impl Into<String>) {
        append_event(&mut self.snapshot, message.into());
        self.write(true);
    }

    pub(crate) fn idle(&mut self, message: impl Into<String>) {
        append_event(&mut self.snapshot, message.into());
        self.snapshot.job_id = None;
        self.snapshot.job_kind = None;
        self.snapshot.state = "idle".into();
        self.snapshot.message = "Waiting for work".into();
        self.snapshot.current = None;
        self.snapshot.total = None;
        self.snapshot.total_is_estimate = false;
        self.snapshot.unit = None;
        self.write(true);
    }

    fn write(&mut self, force: bool) {
        if !force && self.last_write.elapsed() < WRITE_INTERVAL {
            return;
        }
        self.snapshot.updated_at = Utc::now().to_rfc3339();
        if let Some(path) = &self.path {
            match write_path(path, &self.snapshot) {
                Ok(()) => self.write_error_reported = false,
                Err(error) if !self.write_error_reported => {
                    eprintln!("CRM progress telemetry error: {error}");
                    self.write_error_reported = true;
                }
                Err(_) => {}
            }
        }
        self.last_write = Instant::now();
    }
}

pub(crate) fn read(config_path: &Path) -> Option<ProgressSnapshot> {
    read_path(&progress_path(config_path))
}

fn progress_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("live-status.json")
}

fn read_path(path: &Path) -> Option<ProgressSnapshot> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn append_event(snapshot: &mut ProgressSnapshot, message: String) {
    snapshot.events.push(ProgressEvent {
        at: Utc::now().to_rfc3339(),
        message,
    });
    let excess = snapshot.events.len().saturating_sub(EVENT_LIMIT);
    if excess > 0 {
        snapshot.events.drain(..excess);
    }
}

fn write_path(path: &Path, snapshot: &ProgressSnapshot) -> std::io::Result<()> {
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(snapshot).map_err(std::io::Error::other)?;
    fs::write(&temporary, bytes)?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_progress_and_bounds_the_activity_log() {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("config.toml");
        let mut tracker = ProgressTracker::start(&config_path, 42, "gmail");
        tracker.progress(
            "Reading emails from inbox@example.com",
            10,
            Some(100),
            true,
            "emails",
        );
        for index in 0..25 {
            tracker.event(format!("Event {index}"));
        }

        let snapshot = read(&config_path).unwrap();
        assert_eq!(snapshot.job_id, Some(42));
        assert_eq!(snapshot.current, Some(10));
        assert_eq!(snapshot.total, Some(100));
        assert_eq!(snapshot.events.len(), EVENT_LIMIT);
        assert_eq!(snapshot.events.last().unwrap().message, "Event 24");

        tracker.idle("Completed gmail");
        let snapshot = read(&config_path).unwrap();
        assert_eq!(snapshot.state, "idle");
        assert_eq!(snapshot.message, "Waiting for work");
    }
}

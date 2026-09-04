use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{Deserialize, Serialize};

const EVENT_LIMIT: usize = 20;
const FOCUS_ITEM_LIMIT: usize = 120;
const WRITE_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy)]
pub(crate) struct ProgressStage {
    pub current: u64,
    pub total: u64,
}

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
    #[serde(default)]
    pub stage_current: u64,
    #[serde(default)]
    pub stage_total: u64,
    #[serde(default)]
    pub current: u64,
    #[serde(default)]
    pub total: u64,
    pub total_is_estimate: bool,
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub focus: Vec<String>,
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
        let path = progress_path(config_path, job_kind);
        let mut snapshot = read_path(&path).unwrap_or_default();
        snapshot.job_id = Some(job_id);
        snapshot.job_kind = Some(job_kind.to_owned());
        snapshot.state = "running".into();
        snapshot.message = format!("Starting {job_kind}");
        snapshot.stage_current = 1;
        snapshot.stage_total = 1;
        snapshot.current = 0;
        snapshot.total = 1;
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn stage(
        &mut self,
        message: impl Into<String>,
        stage_current: u64,
        stage_total: u64,
        total: u64,
        total_is_estimate: bool,
        unit: &str,
    ) {
        let message = message.into();
        self.snapshot.message.clone_from(&message);
        self.snapshot.stage_current = stage_current;
        self.snapshot.stage_total = stage_total;
        self.snapshot.current = 0;
        self.snapshot.total = total;
        self.snapshot.total_is_estimate = total_is_estimate;
        self.snapshot.unit = Some(unit.into());
        self.snapshot.focus.clear();
        append_event(&mut self.snapshot, message);
        self.write(true);
    }

    pub(crate) fn focus(&mut self, items: impl IntoIterator<Item = String>) {
        self.snapshot.focus = items.into_iter().map(sanitize_focus).collect();
        self.write(false);
    }

    pub(crate) fn focus_now(&mut self, items: impl IntoIterator<Item = String>) {
        self.snapshot.focus = items.into_iter().map(sanitize_focus).collect();
        self.write(true);
    }

    pub(crate) fn progress(
        &mut self,
        message: impl Into<String>,
        current: u64,
        total: u64,
        total_is_estimate: bool,
        unit: &str,
    ) {
        self.snapshot.message = message.into();
        self.snapshot.current = current;
        self.snapshot.total = total;
        self.snapshot.total_is_estimate = total_is_estimate;
        self.snapshot.unit = Some(unit.into());
        self.write(false);
    }

    pub(crate) fn finish_stage(
        &mut self,
        message: impl Into<String>,
        current: u64,
        total: u64,
        total_is_estimate: bool,
        unit: &str,
    ) {
        let message = message.into();
        self.progress_now(&message, current, total, total_is_estimate, unit);
        self.snapshot.focus.clear();
        append_event(&mut self.snapshot, message);
        self.write(true);
    }

    pub(crate) fn progress_now(
        &mut self,
        message: impl Into<String>,
        current: u64,
        total: u64,
        total_is_estimate: bool,
        unit: &str,
    ) {
        self.snapshot.message = message.into();
        self.snapshot.current = current;
        self.snapshot.total = total;
        self.snapshot.total_is_estimate = total_is_estimate;
        self.snapshot.unit = Some(unit.into());
        self.write(true);
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
        self.snapshot.stage_current = 0;
        self.snapshot.stage_total = 0;
        self.snapshot.current = 0;
        self.snapshot.total = 0;
        self.snapshot.total_is_estimate = false;
        self.snapshot.unit = None;
        self.snapshot.focus.clear();
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

pub(crate) fn read(config_path: &Path, job_kind: &str) -> Option<ProgressSnapshot> {
    read_path(&progress_path(config_path, job_kind))
}

pub(crate) fn record_interrupted(config_path: &Path, count: usize) {
    if count == 0 {
        return;
    }
    let directory = progress_directory(config_path);
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.extension().is_some_and(|value| value == "json") {
            continue;
        }
        let Some(mut snapshot) = read_path(&path).filter(|snapshot| snapshot.state == "running")
        else {
            continue;
        };
        let message = format!(
            "Interrupted at {} of {} {}: {}",
            snapshot.current,
            snapshot.total,
            snapshot.unit.as_deref().unwrap_or("items"),
            snapshot.message,
        );
        snapshot.state = "interrupted".into();
        snapshot.message.clone_from(&message);
        snapshot.focus.clear();
        snapshot.updated_at = Utc::now().to_rfc3339();
        append_event(&mut snapshot, message);
        let _ = write_path(&path, &snapshot);
    }
}

fn progress_directory(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("live-status")
}

fn progress_path(config_path: &Path, job_kind: &str) -> PathBuf {
    progress_directory(config_path).join(format!("{job_kind}.json"))
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

fn sanitize_focus(value: String) -> String {
    let normalized: String = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    let mut characters = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    if characters.chars().count() > FOCUS_ITEM_LIMIT {
        characters = characters.chars().take(FOCUS_ITEM_LIMIT - 1).collect();
        characters.push('…');
    }
    characters
}

fn write_path(path: &Path, snapshot: &ProgressSnapshot) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
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
        tracker.stage("Reading inbox", 1, 2, 100, false, "emails");
        tracker.progress(
            "Reading emails from inbox@example.com",
            10,
            100,
            true,
            "emails",
        );
        tracker.focus(["Alex\nSmith — a very long message".repeat(8)]);
        for index in 0..25 {
            tracker.event(format!("Event {index}"));
        }

        let snapshot = read(&config_path, "gmail").unwrap();
        assert_eq!(snapshot.job_id, Some(42));
        assert_eq!(snapshot.stage_current, 1);
        assert_eq!(snapshot.stage_total, 2);
        assert_eq!(snapshot.current, 10);
        assert_eq!(snapshot.focus.len(), 1);
        assert!(!snapshot.focus[0].contains('\n'));
        assert_eq!(snapshot.focus[0].chars().count(), FOCUS_ITEM_LIMIT);
        assert_eq!(snapshot.total, 100);
        assert_eq!(snapshot.events.len(), EVENT_LIMIT);
        assert_eq!(snapshot.events.last().unwrap().message, "Event 24");

        tracker.idle("Completed gmail");
        let snapshot = read(&config_path, "gmail").unwrap();
        assert_eq!(snapshot.state, "idle");
        assert_eq!(snapshot.message, "Waiting for work");
        assert!(snapshot.focus.is_empty());
    }

    #[test]
    fn preserves_interrupted_job_progress_for_the_next_run() {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("config.toml");
        let mut tracker = ProgressTracker::start(&config_path, 7, "analysis");
        tracker.progress_now(
            "Analyzing interactions 97-104 of 915",
            96,
            915,
            false,
            "interactions",
        );

        record_interrupted(&config_path, 1);

        let snapshot = read(&config_path, "analysis").unwrap();
        assert_eq!(snapshot.state, "interrupted");
        assert_eq!(snapshot.current, 96);
        assert!(snapshot.focus.is_empty());
        assert_eq!(snapshot.total, 915);
        assert_eq!(
            snapshot.message,
            "Interrupted at 96 of 915 interactions: Analyzing interactions 97-104 of 915"
        );

        let tracker = ProgressTracker::start(&config_path, 7, "analysis");
        assert_eq!(
            tracker.snapshot.events[tracker.snapshot.events.len() - 2].message,
            "Interrupted at 96 of 915 interactions: Analyzing interactions 97-104 of 915"
        );
    }

    #[test]
    fn leaves_idle_progress_unchanged_when_another_job_was_interrupted() {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("config.toml");
        let mut tracker = ProgressTracker::start(&config_path, 8, "gmail");
        tracker.idle("Completed gmail");

        record_interrupted(&config_path, 1);

        assert_eq!(read(&config_path, "gmail").unwrap().state, "idle");
    }

    #[test]
    fn keeps_concurrent_job_progress_separate() {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("config.toml");
        let mut gmail = ProgressTracker::start(&config_path, 1, "gmail");
        let mut photos = ProgressTracker::start(&config_path, 2, "photos");
        gmail.progress_now("Reading Gmail", 1, 10, false, "emails");
        photos.progress_now("Reading Photos", 2, 5, false, "people");

        assert_eq!(
            read(&config_path, "gmail").unwrap().message,
            "Reading Gmail"
        );
        assert_eq!(
            read(&config_path, "photos").unwrap().message,
            "Reading Photos"
        );
    }
}

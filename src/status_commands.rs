use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Local};
use rusqlite::Connection;
use serde::Serialize;

use crate::cli::StatusArgs;
use crate::error::{CrmError, Result};
use crate::output::{self, Format};
use crate::progress::ProgressSnapshot;

#[derive(Serialize)]
pub(crate) struct Status {
    config_path: PathBuf,
    database_path: PathBuf,
    schema_version: i64,
    source_count: i64,
    daemon_running: bool,
    daemon_pid: Option<i64>,
    queued_jobs: i64,
    running_jobs: i64,
    rerun_jobs: i64,
    running_activity: Vec<ProgressSnapshot>,
    failed_jobs: i64,
    pending_reviews: i64,
    active_people: i64,
    retired_people: i64,
    migration_people: i64,
    pending_analysis: i64,
    sources: Vec<SourceStatus>,
}

#[derive(Serialize)]
struct SourceStatus {
    id: String,
    status: String,
    cursor: Option<String>,
    last_sync_at: Option<String>,
    last_reconcile_at: Option<String>,
    error: Option<String>,
}

pub(crate) fn initialized(
    config_path: PathBuf,
    database_path: PathBuf,
    schema_version: i64,
) -> Status {
    Status {
        config_path,
        database_path,
        schema_version,
        source_count: 0,
        daemon_running: false,
        daemon_pid: None,
        queued_jobs: 0,
        running_jobs: 0,
        rerun_jobs: 0,
        running_activity: Vec::new(),
        failed_jobs: 0,
        pending_reviews: 0,
        active_people: 0,
        retired_people: 0,
        migration_people: 1,
        pending_analysis: 0,
        sources: Vec::new(),
    }
}

pub(crate) fn run(format: Format, config_path: PathBuf, args: StatusArgs) -> Result<()> {
    let connection = crate::commands::open_database(&config_path)?;
    if args.live {
        live(format, &config_path, &connection)
    } else {
        let status = collect(&config_path, &connection)?;
        let table = summary(&status);
        output::emit(format, "status", &status, table)
    }
}

fn live(format: Format, config_path: &Path, connection: &Connection) -> Result<()> {
    if !matches!(format, Format::Table) {
        return Err(CrmError::InvalidConfig(
            "`crm status --live` only supports table output".into(),
        ));
    }
    if !std::io::stdout().is_terminal() {
        return Err(CrmError::InvalidConfig(
            "`crm status --live` requires an interactive terminal".into(),
        ));
    }
    loop {
        let status = collect(config_path, connection)?;
        let screen = live_screen(&status);
        print!("\x1b[H\x1b[2J{screen}");
        std::io::stdout().flush().map_err(|source| CrmError::Io {
            path: PathBuf::from("stdout"),
            source,
        })?;
        thread::sleep(Duration::from_millis(500));
    }
}

fn collect(config_path: &Path, connection: &Connection) -> Result<Status> {
    let database_path = config_path.parent().unwrap().join("crm.sqlite3");
    let source_count =
        connection.query_row("SELECT COUNT(*) FROM sources", [], |row| row.get(0))?;
    let daemon_pid: Option<i64> = connection.query_row(
        "SELECT pid FROM daemon_state WHERE id=1 UNION ALL SELECT NULL LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    let daemon_running = daemon_pid.is_some_and(crate::daemon::process_is_running);
    let count = |sql| connection.query_row(sql, [], |row| row.get::<_, i64>(0));
    let mut statement =
        connection.prepare("SELECT id, kind FROM jobs WHERE state='running' ORDER BY kind")?;
    let running: Vec<(i64, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let running_activity = running
        .into_iter()
        .map(|(id, kind)| {
            crate::progress::read(config_path, &kind)
                .filter(|progress| progress.job_id == Some(id) && progress.state == "running")
                .unwrap_or_else(|| ProgressSnapshot {
                    job_id: Some(id),
                    job_kind: Some(kind.clone()),
                    state: "running".into(),
                    message: format!("Starting {kind}"),
                    ..ProgressSnapshot::default()
                })
        })
        .collect();
    let sources = connection
        .prepare(
            "SELECT id, status, cursor, last_sync_at, last_reconcile_at, error
             FROM sources ORDER BY id",
        )?
        .query_map([], |row| {
            Ok(SourceStatus {
                id: row.get(0)?,
                status: row.get(1)?,
                cursor: row.get(2)?,
                last_sync_at: row.get(3)?,
                last_reconcile_at: row.get(4)?,
                error: row.get(5)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok(Status {
        config_path: config_path.to_owned(),
        database_path,
        schema_version: crate::db::schema_version(connection)?,
        source_count,
        daemon_running,
        daemon_pid,
        queued_jobs: count("SELECT COUNT(*) FROM jobs WHERE state='queued'")?,
        running_jobs: count("SELECT COUNT(*) FROM jobs WHERE state='running'")?,
        rerun_jobs: count("SELECT COUNT(*) FROM jobs WHERE state='running' AND rerun_requested=1")?,
        running_activity,
        failed_jobs: crate::jobs::unresolved_failed_count(connection)?,
        pending_reviews: count("SELECT COUNT(*) FROM review_items WHERE status='pending'")?,
        active_people: count("SELECT COUNT(*) FROM people WHERE lifecycle_state='active'")?,
        retired_people: count("SELECT COUNT(*) FROM people WHERE lifecycle_state='retired'")?,
        migration_people: count(
            "SELECT COUNT(*) FROM people p WHERE lifecycle_state='migration_pending'
             AND NOT EXISTS (SELECT 1 FROM person_merges m WHERE m.source_person_id=p.id)",
        )?,
        pending_analysis: count(
            "SELECT COUNT(*) FROM interactions WHERE analysis_state='pending'
             AND deleted_at IS NULL AND body IS NOT NULL AND trim(body) != ''",
        )?,
        sources,
    })
}

fn summary(status: &Status) -> String {
    let mut output = format!(
        "daemon         {}{}\nschema version  {}\nsources         {}\npeople          {} active, {} retired, {} migration\njobs            {} queued, {} running, {} rerun, {} failed\nanalysis        {} pending\nreview          {} pending",
        if status.daemon_running {
            "running"
        } else {
            "stopped"
        },
        status
            .daemon_pid
            .map(|pid| format!(" (PID {pid})"))
            .unwrap_or_default(),
        status.schema_version,
        status.source_count,
        status.active_people,
        status.retired_people,
        status.migration_people,
        status.queued_jobs,
        status.running_jobs,
        status.rerun_jobs,
        status.failed_jobs,
        status.pending_analysis,
        status.pending_reviews
    );
    if !status.sources.is_empty() {
        output.push_str("\n\nSource status\n");
        for source in &status.sources {
            output.push_str(&format!(
                "{:<18} {:<8} sync {}  audit {}  cursor {}{}\n",
                source.id,
                source.status,
                source.last_sync_at.as_deref().unwrap_or("-"),
                source.last_reconcile_at.as_deref().unwrap_or("-"),
                source.cursor.as_deref().unwrap_or("-"),
                source
                    .error
                    .as_deref()
                    .map(|error| format!("  error {error}"))
                    .unwrap_or_default(),
            ));
        }
        output.pop();
    }
    output
}

fn live_screen(status: &Status) -> String {
    let mut output = format!("CRM live status — Ctrl-C to exit\n\n{}", summary(status));
    output.push_str("\n\nCurrent activity\n");
    if !status.daemon_running {
        output.push_str("Daemon is stopped; no live activity.");
    } else if !status.running_activity.is_empty() {
        for progress in &status.running_activity {
            output.push_str(&format!(
                "{}  {}",
                progress.job_kind.as_deref().unwrap_or("job"),
                progress.message
            ));
            if progress.state == "running" {
                output.push_str(&format!(
                    "\nStage {} / {}\n{}",
                    progress.stage_current,
                    progress.stage_total,
                    progress_bar(progress)
                ));
            }
            output.push_str("\n\n");
        }
        output.pop();
        output.pop();
    } else {
        output.push_str("Waiting for work.");
    }
    let events: Vec<_> = status
        .running_activity
        .iter()
        .flat_map(|progress| progress.events.iter())
        .collect();
    if !events.is_empty() {
        output.push_str("\n\nRecent activity\n");
        for event in events {
            output.push_str(&format!("{}  {}\n", local_time(&event.at), event.message));
        }
        output.pop();
    }
    output.push('\n');
    output
}

fn progress_bar(progress: &ProgressSnapshot) -> String {
    const WIDTH: usize = 36;
    let current = progress.current;
    let total = progress.total.max(current);
    let (filled, percent) = if total == 0 {
        (WIDTH, 100)
    } else {
        let filled = ((current as f64 / total as f64) * WIDTH as f64).round() as usize;
        let percent = (current.saturating_mul(100) / total).min(100);
        (filled, percent)
    };
    let estimate = if progress.total_is_estimate { "~" } else { "" };
    let unit = progress.unit.as_deref().unwrap_or("items");
    format!(
        "[{}{}] {} / {}{} {} ({}%)",
        "█".repeat(filled),
        "░".repeat(WIDTH - filled),
        grouped(current),
        estimate,
        grouped(total),
        unit,
        percent
    )
}

fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut result = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(character);
    }
    result
}

fn local_time(value: &str) -> String {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| {
            timestamp
                .with_timezone(&Local)
                .format("%H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|_| "--:--:--".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_known_progress() {
        let progress = ProgressSnapshot {
            job_kind: Some("whatsapp".into()),
            current: 1234,
            total: 5000,
            total_is_estimate: true,
            unit: Some("emails".into()),
            ..ProgressSnapshot::default()
        };
        let bar = progress_bar(&progress);
        assert!(bar.contains("1,234 / ~5,000 emails (24%)"));
    }

    #[test]
    fn zero_work_is_complete_without_an_indeterminate_bar() {
        let progress = ProgressSnapshot {
            current: 0,
            total: 0,
            unit: Some("messages".into()),
            ..ProgressSnapshot::default()
        };
        assert!(progress_bar(&progress).contains("0 / 0 messages (100%)"));
    }

    #[test]
    fn live_screen_shows_stage_and_exact_item_progress() {
        let mut status = initialized("config.toml".into(), "crm.sqlite3".into(), 10);
        status.daemon_running = true;
        let progress = ProgressSnapshot {
            state: "running".into(),
            message: "Reading WhatsApp conversations".into(),
            stage_current: 2,
            stage_total: 4,
            current: 25,
            total: 100,
            unit: Some("messages".into()),
            ..ProgressSnapshot::default()
        };
        status.running_activity.push(progress);
        let screen = live_screen(&status);
        assert!(screen.contains("Stage 2 / 4"));
        assert!(screen.contains("25 / 100 messages (25%)"));
        assert!(!screen.contains("working"));
    }

    #[test]
    fn groups_large_counts() {
        assert_eq!(grouped(20_280), "20,280");
    }
}

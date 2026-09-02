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
    failed_jobs: i64,
    pending_reviews: i64,
    active_people: i64,
    retired_people: i64,
    migration_people: i64,
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
        failed_jobs: 0,
        pending_reviews: 0,
        active_people: 0,
        retired_people: 0,
        migration_people: 1,
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
        let progress = crate::progress::read(config_path);
        let screen = live_screen(&status, progress.as_ref());
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
    Ok(Status {
        config_path: config_path.to_owned(),
        database_path,
        schema_version: crate::db::schema_version(connection)?,
        source_count,
        daemon_running,
        daemon_pid,
        queued_jobs: count("SELECT COUNT(*) FROM jobs WHERE state='queued'")?,
        running_jobs: count("SELECT COUNT(*) FROM jobs WHERE state='running'")?,
        failed_jobs: crate::jobs::unresolved_failed_count(connection)?,
        pending_reviews: count("SELECT COUNT(*) FROM review_items WHERE status='pending'")?,
        active_people: count("SELECT COUNT(*) FROM people WHERE lifecycle_state='active'")?,
        retired_people: count("SELECT COUNT(*) FROM people WHERE lifecycle_state='retired'")?,
        migration_people: count(
            "SELECT COUNT(*) FROM people p WHERE lifecycle_state='migration_pending'
             AND NOT EXISTS (SELECT 1 FROM person_merges m WHERE m.source_person_id=p.id)",
        )?,
    })
}

fn summary(status: &Status) -> String {
    format!(
        "daemon         {}{}\nschema version  {}\nsources         {}\npeople          {} active, {} retired, {} migration\njobs            {} queued, {} running, {} failed\nreview          {} pending",
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
        status.failed_jobs,
        status.pending_reviews
    )
}

fn live_screen(status: &Status, progress: Option<&ProgressSnapshot>) -> String {
    let mut output = format!("CRM live status — Ctrl-C to exit\n\n{}", summary(status));
    output.push_str("\n\nCurrent activity\n");
    if !status.daemon_running {
        output.push_str("Daemon is stopped; no live activity.");
    } else if let Some(progress) = progress {
        output.push_str(&progress.message);
        if progress.state == "running" {
            output.push_str(&format!(
                "\nStage {} / {}",
                progress.stage_current, progress.stage_total
            ));
            output.push('\n');
            output.push_str(&progress_bar(progress));
        }
    } else {
        output.push_str("Waiting for the daemon's first progress update.");
    }
    if let Some(progress) = progress
        && !progress.events.is_empty()
    {
        output.push_str("\n\nRecent activity\n");
        for event in &progress.events {
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
        let screen = live_screen(&status, Some(&progress));
        assert!(screen.contains("Stage 2 / 4"));
        assert!(screen.contains("25 / 100 messages (25%)"));
        assert!(!screen.contains("working"));
    }

    #[test]
    fn groups_large_counts() {
        assert_eq!(grouped(20_280), "20,280");
    }
}

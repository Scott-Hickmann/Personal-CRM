use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::Serialize;

use crate::cli::StatusArgs;
use crate::error::{CrmError, Result};
use crate::output::{self, Format};
use crate::progress::ProgressSnapshot;

mod live;

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
    total_contacts: i64,
    total_analyzable_interactions: i64,
    analyzed_interactions: i64,
    sources: Vec<SourceStatus>,
}

#[derive(Serialize)]
pub(super) struct SourceStatus {
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
        total_contacts: 0,
        total_analyzable_interactions: 0,
        analyzed_interactions: 0,
        sources: Vec::new(),
    }
}

pub(crate) fn run(format: Format, config_path: PathBuf, args: StatusArgs) -> Result<()> {
    let connection = crate::commands::open_database(&config_path)?;
    if args.live {
        live(format, &config_path, &connection)
    } else {
        let mut status = collect(&config_path, &connection)?;
        for activity in &mut status.running_activity {
            activity.focus.clear();
        }
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
    live::run(|| collect(config_path, connection))
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
    let interaction_counts = crate::analysis::counts(connection)?;
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
        total_contacts: count("SELECT COUNT(*) FROM people WHERE lifecycle_state='active'")?,
        total_analyzable_interactions: interaction_counts.total,
        analyzed_interactions: interaction_counts.analyzed,
        sources,
    })
}

fn summary(status: &Status) -> String {
    let mut output = format!(
        "daemon                    {}{}\nschema version             {}\nsources                    {}\ncontacts                   {}\nanalyzable interactions    {}\nanalyzed interactions      {}\njobs                       {} queued, {} running, {} rerun, {} failed\nreview                     {} pending",
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
        grouped(status.total_contacts as u64),
        grouped(status.total_analyzable_interactions as u64),
        grouped(status.analyzed_interactions as u64),
        status.queued_jobs,
        status.running_jobs,
        status.rerun_jobs,
        status.failed_jobs,
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

pub(super) fn grouped(value: u64) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_large_counts() {
        assert_eq!(grouped(20_280), "20,280");
    }
}

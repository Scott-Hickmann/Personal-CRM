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
    pending_work: i64,
    running_work: i64,
    running_activity: Vec<ProgressSnapshot>,
    failed_work: i64,
    dirty_people: i64,
    dirty_conversations: i64,
    pending_reviews: i64,
    total_contacts: i64,
    total_interactions: i64,
    work: Vec<WorkStatus>,
    sources: Vec<SourceStatus>,
}

#[derive(Serialize)]
struct WorkStatus {
    kind: String,
    state: String,
    step: Option<String>,
    requested_generation: i64,
    completed_generation: i64,
    error: Option<String>,
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
        pending_work: 0,
        running_work: 0,
        running_activity: Vec::new(),
        failed_work: 0,
        dirty_people: 0,
        dirty_conversations: 0,
        pending_reviews: 0,
        total_contacts: 0,
        total_interactions: 0,
        work: Vec::new(),
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
    let running = crate::coordinator::running(connection)?;
    let running_activity = running
        .into_iter()
        .map(|kind| {
            crate::progress::read(config_path, &kind)
                .filter(|progress| progress.state == "running")
                .unwrap_or_else(|| ProgressSnapshot {
                    work_kind: Some(kind.clone()),
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
    let work = connection
        .prepare(
            "SELECT kind, state, step, requested_generation, completed_generation, error
             FROM source_sync_state
             UNION ALL
             SELECT kind, state, NULL, requested_generation, completed_generation, error
             FROM maintenance_state
             ORDER BY kind",
        )?
        .query_map([], |row| {
            Ok(WorkStatus {
                kind: row.get(0)?,
                state: row.get(1)?,
                step: row.get(2)?,
                requested_generation: row.get(3)?,
                completed_generation: row.get(4)?,
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
        pending_work: crate::coordinator::pending_count(connection)?,
        running_work: crate::coordinator::running(connection)?.len() as i64,
        running_activity,
        failed_work: crate::coordinator::failed_count(connection)?,
        dirty_people: count("SELECT COUNT(*) FROM dirty_people")?,
        dirty_conversations: count("SELECT COUNT(*) FROM dirty_conversations")?,
        pending_reviews: count("SELECT COUNT(*) FROM review_items WHERE status='pending'")?,
        total_contacts: count("SELECT COUNT(*) FROM people WHERE lifecycle_state='active'")?,
        total_interactions: count("SELECT COUNT(*) FROM interactions WHERE deleted_at IS NULL")?,
        work,
        sources,
    })
}

fn summary(status: &Status) -> String {
    let mut output = format!(
        "daemon                    {}{}\nschema version             {}\nsources                    {}\ncontacts                   {}\ninteractions               {}\nwork                       {} pending, {} running, {} failed\ndirty conversations        {}\ndirty people               {}\nreview                     {} pending",
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
        grouped(status.total_interactions as u64),
        status.pending_work,
        status.running_work,
        status.failed_work,
        status.dirty_conversations,
        status.dirty_people,
        status.pending_reviews
    );
    let active_work = status
        .work
        .iter()
        .filter(|work| work.state != "idle" || work.error.is_some())
        .collect::<Vec<_>>();
    if !active_work.is_empty() {
        output.push_str("\n\nPersisted work\n");
        for work in active_work {
            output.push_str(&format!(
                "{:<18} {:<8} {:<14} generation {}/{}{}\n",
                work.kind,
                work.state,
                work.step.as_deref().unwrap_or("-"),
                work.completed_generation,
                work.requested_generation,
                work.error
                    .as_deref()
                    .map(|error| format!("  error {error}"))
                    .unwrap_or_default(),
            ));
        }
        output.pop();
    }
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

use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDateTime, Utc};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::error::Result;
use crate::progress::ProgressSnapshot;

use super::catalog;

#[derive(Serialize)]
pub(crate) struct Status {
    pub config_path: PathBuf,
    pub database_path: PathBuf,
    pub generated_at: String,
    pub schema_version: i64,
    pub source_count: i64,
    pub daemon_running: bool,
    pub daemon_pid: Option<i64>,
    pub daemon_started_at: Option<String>,
    pub daemon_heartbeat_at: Option<String>,
    pub daemon_last_error: Option<String>,
    pub pending_work: i64,
    pub running_work: i64,
    pub failed_work: i64,
    pub dirty_people: i64,
    pub dirty_conversations: i64,
    pub pending_reviews: i64,
    pub total_contacts: i64,
    pub total_interactions: i64,
    pub work: Vec<WorkStatus>,
    pub sources: Vec<SourceStatus>,
}

#[derive(Serialize)]
pub(crate) struct WorkStatus {
    pub kind: String,
    pub state: String,
    pub step: Option<String>,
    pub reason: Option<String>,
    pub run_after: String,
    pub requested_generation: i64,
    pub running_generation: Option<i64>,
    pub completed_generation: i64,
    pub attempts: i64,
    pub changed: bool,
    pub error: Option<String>,
    pub updated_at: String,
    pub pending_position: Option<usize>,
    pub downstream: Vec<&'static str>,
    pub progress: Option<ProgressSnapshot>,
}

#[derive(Serialize)]
pub(crate) struct SourceStatus {
    pub id: String,
    pub status: String,
    pub cursor: Option<String>,
    pub last_sync_at: Option<String>,
    pub last_reconcile_at: Option<String>,
    pub error: Option<String>,
}

impl Status {
    pub(super) fn initialized(
        config_path: PathBuf,
        database_path: PathBuf,
        schema_version: i64,
    ) -> Self {
        Self {
            config_path,
            database_path,
            generated_at: Utc::now().to_rfc3339(),
            schema_version,
            source_count: 0,
            daemon_running: false,
            daemon_pid: None,
            daemon_started_at: None,
            daemon_heartbeat_at: None,
            daemon_last_error: None,
            pending_work: 0,
            running_work: 0,
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

    pub(super) fn active_or_next(&self) -> Option<&WorkStatus> {
        self.work
            .iter()
            .find(|work| work.state == "running")
            .or_else(|| {
                self.work
                    .iter()
                    .find(|work| work.pending_position == Some(1))
            })
            .or_else(|| self.work.iter().find(|work| work.state == "failed"))
    }
}

impl WorkStatus {
    pub(super) fn label(&self) -> &'static str {
        catalog::label(&self.kind)
    }

    pub(super) fn is_source(&self) -> bool {
        catalog::priority(&self.kind) < 6
    }

    pub(super) fn rerun_queued(&self) -> bool {
        self.state == "running"
            && self
                .running_generation
                .is_some_and(|running| self.requested_generation > running)
    }

    pub(super) fn ready(&self) -> bool {
        self.state == "pending" && ready_at(&self.run_after)
    }
}

pub(super) fn collect(config_path: &Path, connection: &Connection) -> Result<Status> {
    let database_path = config_path.parent().unwrap().join("crm.sqlite3");
    let source_count =
        connection.query_row("SELECT COUNT(*) FROM sources", [], |row| row.get(0))?;
    let daemon = connection
        .query_row(
            "SELECT pid, started_at, heartbeat_at, last_error FROM daemon_state WHERE id=1",
            [],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()?;
    let (daemon_pid, daemon_started_at, daemon_heartbeat_at, daemon_last_error) =
        daemon.unwrap_or_default();
    let daemon_running = daemon_pid.is_some_and(crate::daemon::process_is_running);
    let count = |sql| connection.query_row(sql, [], |row| row.get::<_, i64>(0));
    let sources = collect_sources(connection)?;
    let mut work = collect_work(config_path, connection)?;
    assign_pending_positions(&mut work);
    let pending_work = work.iter().filter(|item| item.state == "pending").count() as i64;
    let running_work = work.iter().filter(|item| item.state == "running").count() as i64;
    let failed_work = work.iter().filter(|item| item.state == "failed").count() as i64;
    Ok(Status {
        config_path: config_path.to_owned(),
        database_path,
        generated_at: Utc::now().to_rfc3339(),
        schema_version: crate::db::schema_version(connection)?,
        source_count,
        daemon_running,
        daemon_pid,
        daemon_started_at,
        daemon_heartbeat_at,
        daemon_last_error,
        pending_work,
        running_work,
        failed_work,
        dirty_people: count("SELECT COUNT(*) FROM dirty_people")?,
        dirty_conversations: count("SELECT COUNT(*) FROM dirty_conversations")?,
        pending_reviews: count("SELECT COUNT(*) FROM review_items WHERE status='pending'")?,
        total_contacts: count("SELECT COUNT(*) FROM people WHERE lifecycle_state='active'")?,
        total_interactions: count("SELECT COUNT(*) FROM interactions WHERE deleted_at IS NULL")?,
        work,
        sources,
    })
}

fn collect_sources(connection: &Connection) -> Result<Vec<SourceStatus>> {
    let mut statement = connection.prepare(
        "SELECT id, status, cursor, last_sync_at, last_reconcile_at, error
         FROM sources ORDER BY id",
    )?;
    Ok(statement
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
        .collect::<std::result::Result<_, _>>()?)
}

fn collect_work(config_path: &Path, connection: &Connection) -> Result<Vec<WorkStatus>> {
    let mut statement = connection.prepare(
        "SELECT kind, state, step, reason, run_after, requested_generation,
                running_generation, completed_generation, attempts, changed, error, updated_at
         FROM source_sync_state
         UNION ALL
         SELECT kind, state, NULL, reason, run_after, requested_generation,
                running_generation, completed_generation, attempts, 0, error, updated_at
         FROM maintenance_state",
    )?;
    let mut work = statement
        .query_map([], |row| {
            let kind: String = row.get(0)?;
            let state: String = row.get(1)?;
            let running_generation = row.get(6)?;
            let reason: Option<String> = row.get(3)?;
            let mut progress = crate::progress::read(config_path, &kind);
            if state == "running"
                && !progress.as_ref().is_some_and(|snapshot| {
                    snapshot.state == "running" && snapshot.generation == running_generation
                })
            {
                progress = Some(ProgressSnapshot {
                    work_kind: Some(kind.clone()),
                    generation: running_generation,
                    reason: reason.clone(),
                    state: "running".into(),
                    message: format!("Starting {kind}"),
                    ..ProgressSnapshot::default()
                });
            }
            Ok(WorkStatus {
                downstream: catalog::downstream(&kind),
                kind,
                state,
                step: row.get(2)?,
                reason,
                run_after: row.get(4)?,
                requested_generation: row.get(5)?,
                running_generation,
                completed_generation: row.get(7)?,
                attempts: row.get(8)?,
                changed: row.get(9)?,
                error: row.get(10)?,
                updated_at: row.get(11)?,
                pending_position: None,
                progress,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    work.sort_by_key(|item| catalog::priority(&item.kind));
    Ok(work)
}

fn assign_pending_positions(work: &mut [WorkStatus]) {
    let mut position = 0;
    for item in work {
        if item.ready() {
            position += 1;
            item.pending_position = Some(position);
        }
    }
}

fn ready_at(value: &str) -> bool {
    let timestamp = DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .or_else(|_| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").map(|value| value.and_utc())
        });
    timestamp.map_or(true, |value| value <= Utc::now())
}

#[cfg(test)]
#[path = "model/tests.rs"]
mod tests;

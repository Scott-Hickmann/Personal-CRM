use std::path::Path;

use chrono::{Duration, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::error::{CrmError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Contacts,
    Communications,
    Gmail,
    Analysis,
    Scoring,
    Photos,
    GooglePublish,
    Suggestions,
}

impl JobKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Contacts => "contacts",
            Self::Communications => "communications",
            Self::Gmail => "gmail",
            Self::Analysis => "analysis",
            Self::Scoring => "scoring",
            Self::Photos => "photos",
            Self::GooglePublish => "google_publish",
            Self::Suggestions => "suggestions",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "contacts" => Ok(Self::Contacts),
            "communications" => Ok(Self::Communications),
            "gmail" => Ok(Self::Gmail),
            "analysis" => Ok(Self::Analysis),
            "scoring" => Ok(Self::Scoring),
            "photos" => Ok(Self::Photos),
            "google_publish" => Ok(Self::GooglePublish),
            "suggestions" => Ok(Self::Suggestions),
            _ => Err(CrmError::InvalidConfig(format!("unknown job kind {value}"))),
        }
    }
}

pub fn enqueue(
    connection: &Connection,
    kind: JobKind,
    reason: &str,
    delay: Duration,
) -> Result<()> {
    let run_after = (Utc::now() + delay).to_rfc3339();
    connection.execute(
        "INSERT INTO jobs(kind, reason, run_after) VALUES (?1, ?2, ?3)
         ON CONFLICT(kind) WHERE state IN ('queued', 'running') DO UPDATE SET
           reason=excluded.reason,
           run_after=CASE WHEN jobs.state='running' THEN jobs.run_after
                          ELSE MIN(jobs.run_after, excluded.run_after) END,
           updated_at=CURRENT_TIMESTAMP",
        params![kind.as_str(), reason, run_after],
    )?;
    Ok(())
}

pub fn recover_running(connection: &Connection) -> Result<usize> {
    connection
        .execute(
            "UPDATE jobs SET state='queued', run_after=?1,
             error='daemon restarted while job was running', updated_at=CURRENT_TIMESTAMP
             WHERE state='running'",
            [Utc::now().to_rfc3339()],
        )
        .map_err(Into::into)
}

pub fn unresolved_failed_count(connection: &Connection) -> Result<i64> {
    connection
        .query_row(
            "SELECT COUNT(DISTINCT failed.kind) FROM jobs failed
             WHERE failed.state='failed'
             AND NOT EXISTS (
                SELECT 1 FROM jobs succeeded
                WHERE succeeded.kind=failed.kind AND succeeded.state='complete'
                AND succeeded.id>failed.id
             )",
            [],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

pub fn process_one(config_path: &Path, connection: &Connection) -> Result<bool> {
    let job: Option<(i64, String, i64)> = connection
        .query_row(
            "SELECT id, kind, attempts FROM jobs
             WHERE state='queued' AND run_after<=?1 ORDER BY run_after, id LIMIT 1",
            [Utc::now().to_rfc3339()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((id, kind, attempts)) = job else {
        return Ok(false);
    };
    if connection.execute(
        "UPDATE jobs SET state='running', attempts=attempts+1, updated_at=CURRENT_TIMESTAMP
         WHERE id=?1 AND state='queued'",
        [id],
    )? == 0
    {
        return Ok(true);
    }
    let kind = JobKind::parse(&kind)?;
    let mut progress = crate::progress::ProgressTracker::start(config_path, id, kind.as_str());
    match crate::job_runner::run_with_progress(config_path, kind, &mut progress) {
        Ok(()) => {
            connection.execute(
                "UPDATE jobs SET state='complete', completed_at=CURRENT_TIMESTAMP,
                 updated_at=CURRENT_TIMESTAMP, error=NULL WHERE id=?1",
                [id],
            )?;
            enqueue_downstream(connection, kind)?;
            progress.idle(format!("Completed {}", kind.as_str()));
        }
        Err(error) => {
            let retry = attempts < 4;
            connection.execute(
                "UPDATE jobs SET state=?2, run_after=?3, error=?4, updated_at=CURRENT_TIMESTAMP
                 WHERE id=?1",
                params![
                    id,
                    if retry { "queued" } else { "failed" },
                    (Utc::now() + Duration::seconds(30 * (attempts + 1))).to_rfc3339(),
                    error.to_string()
                ],
            )?;
            progress.idle(if retry {
                format!("{} failed; retry scheduled: {error}", kind.as_str())
            } else {
                format!("{} failed: {error}", kind.as_str())
            });
        }
    }
    Ok(true)
}

pub fn run(config_path: &Path, kind: JobKind) -> Result<()> {
    crate::job_runner::run(config_path, kind)
}

fn enqueue_downstream(connection: &Connection, kind: JobKind) -> Result<()> {
    match kind {
        JobKind::Contacts => {
            enqueue(
                connection,
                JobKind::GooglePublish,
                "contacts reconciled",
                Duration::zero(),
            )?;
            enqueue(
                connection,
                JobKind::Suggestions,
                "contacts reconciled",
                Duration::zero(),
            )?;
        }
        JobKind::Communications | JobKind::Gmail => {
            enqueue(
                connection,
                JobKind::Analysis,
                "new interactions",
                Duration::seconds(30),
            )?;
            enqueue(
                connection,
                JobKind::Suggestions,
                "new participants",
                Duration::zero(),
            )?;
        }
        JobKind::Analysis => enqueue(
            connection,
            JobKind::Scoring,
            "analysis complete",
            Duration::zero(),
        )?,
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn coalesces_open_jobs_by_kind() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        enqueue(
            &connection,
            JobKind::Contacts,
            "first",
            Duration::seconds(5),
        )
        .unwrap();
        enqueue(&connection, JobKind::Contacts, "second", Duration::zero()).unwrap();
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn recovers_jobs_interrupted_by_a_daemon_restart() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        enqueue(
            &connection,
            JobKind::GooglePublish,
            "test",
            Duration::zero(),
        )
        .unwrap();
        connection
            .execute("UPDATE jobs SET state='running'", [])
            .unwrap();

        assert_eq!(recover_running(&connection).unwrap(), 1);
        let (state, error): (String, Option<String>) = connection
            .query_row("SELECT state, error FROM jobs", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(state, "queued");
        assert_eq!(
            error.as_deref(),
            Some("daemon restarted while job was running")
        );
    }

    #[test]
    fn counts_unresolved_failure_kinds_instead_of_historical_rows() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute_batch(
                "INSERT INTO jobs(kind, state, reason) VALUES
                    ('gmail', 'failed', 'first'),
                    ('gmail', 'failed', 'second'),
                    ('analysis', 'failed', 'third');",
            )
            .unwrap();
        assert_eq!(unresolved_failed_count(&connection).unwrap(), 2);

        connection
            .execute(
                "INSERT INTO jobs(kind, state, reason) VALUES ('gmail', 'complete', 'recovered')",
                [],
            )
            .unwrap();
        assert_eq!(unresolved_failed_count(&connection).unwrap(), 1);
    }
}

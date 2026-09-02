use std::path::Path;

use chrono::{Duration, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::error::{CrmError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, clap::ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Contacts,
    Imessage,
    Whatsapp,
    AppleCalls,
    WhatsappCalls,
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
            Self::Imessage => "imessage",
            Self::Whatsapp => "whatsapp",
            Self::AppleCalls => "apple_calls",
            Self::WhatsappCalls => "whatsapp_calls",
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
            "imessage" => Ok(Self::Imessage),
            "whatsapp" => Ok(Self::Whatsapp),
            "apple_calls" => Ok(Self::AppleCalls),
            "whatsapp_calls" => Ok(Self::WhatsappCalls),
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
           rerun_requested=CASE WHEN jobs.state='running' THEN 1
                                ELSE jobs.rerun_requested END,
           rerun_after=CASE WHEN jobs.state='running'
                            THEN MIN(COALESCE(jobs.rerun_after, excluded.run_after), excluded.run_after)
                            ELSE jobs.rerun_after END,
           updated_at=CURRENT_TIMESTAMP",
        params![kind.as_str(), reason, run_after],
    )?;
    Ok(())
}

pub fn recover_running(connection: &Connection) -> Result<usize> {
    connection
        .execute(
            "UPDATE jobs SET state='queued', run_after=?1,
             error='daemon restarted while job was running', rerun_requested=0,
             rerun_after=NULL, updated_at=CURRENT_TIMESTAMP
             WHERE state='running'",
            [Utc::now().to_rfc3339()],
        )
        .map_err(Into::into)
}

pub fn recover_job(connection: &Connection, id: i64, error: &str) -> Result<()> {
    connection.execute(
        "UPDATE jobs SET state='queued', run_after=?2, error=?3,
         rerun_requested=0, rerun_after=NULL,
         updated_at=CURRENT_TIMESTAMP WHERE id=?1 AND state='running'",
        params![id, (Utc::now() + Duration::seconds(30)).to_rfc3339(), error],
    )?;
    Ok(())
}

pub fn recover_kind(connection: &Connection, kind: JobKind, error: &str) -> Result<()> {
    connection.execute(
        "UPDATE jobs SET state='queued', run_after=?2, error=?3,
         rerun_requested=0, rerun_after=NULL,
         updated_at=CURRENT_TIMESTAMP WHERE kind=?1 AND state='running'",
        params![
            kind.as_str(),
            (Utc::now() + Duration::seconds(30)).to_rfc3339(),
            error
        ],
    )?;
    Ok(())
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

pub fn ready(connection: &Connection) -> Result<Vec<(i64, JobKind)>> {
    let mut statement = connection.prepare(
        "SELECT id, kind FROM jobs
         WHERE state='queued' AND run_after<=?1 ORDER BY run_after, id",
    )?;
    statement
        .query_map([Utc::now().to_rfc3339()], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .map(|row| {
            let (id, kind) = row?;
            Ok((id, JobKind::parse(&kind)?))
        })
        .collect()
}

pub fn process(config_path: &Path, id: i64) -> Result<bool> {
    let mut connection = crate::commands::open_database(config_path)?;
    let job: Option<(String, i64)> = connection
        .query_row(
            "SELECT kind, attempts FROM jobs
             WHERE id=?1 AND state='queued' AND run_after<=?2",
            params![id, Utc::now().to_rfc3339()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((kind, attempts)) = job else {
        return Ok(false);
    };
    if connection.execute(
        "UPDATE jobs SET state='running', attempts=attempts+1, updated_at=CURRENT_TIMESTAMP
         WHERE id=?1 AND state='queued'",
        [id],
    )? == 0
    {
        return Ok(false);
    }
    let kind = JobKind::parse(&kind)?;
    let mut progress = crate::progress::ProgressTracker::start(config_path, id, kind.as_str());
    match crate::job_runner::run_with_progress(config_path, kind, &mut progress) {
        Ok(()) => {
            let transaction =
                connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let rerun_after: Option<String> = transaction
                .query_row(
                    "SELECT rerun_after FROM jobs WHERE id=?1 AND rerun_requested=1",
                    [id],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            transaction.execute(
                "UPDATE jobs SET state='complete', completed_at=CURRENT_TIMESTAMP,
                 updated_at=CURRENT_TIMESTAMP, error=NULL, rerun_requested=0,
                 rerun_after=NULL WHERE id=?1",
                [id],
            )?;
            let rerun = rerun_after.is_some();
            if let Some(run_after) = rerun_after {
                transaction.execute(
                    "INSERT INTO jobs(kind, reason, run_after)
                     VALUES (?1, 'changes arrived while job was running', ?2)",
                    params![kind.as_str(), run_after],
                )?;
            }
            enqueue_downstream(&transaction, kind, rerun)?;
            transaction.commit()?;
            progress.idle(format!("Completed {}", kind.as_str()));
        }
        Err(error) => {
            let retry = attempts < 4;
            let transaction =
                connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let rerun_after: Option<String> = if retry {
                None
            } else {
                transaction
                    .query_row(
                        "SELECT rerun_after FROM jobs WHERE id=?1 AND rerun_requested=1",
                        [id],
                        |row| row.get(0),
                    )
                    .optional()?
                    .flatten()
            };
            transaction.execute(
                "UPDATE jobs SET state=?2, run_after=?3, error=?4,
                 rerun_requested=0, rerun_after=NULL, updated_at=CURRENT_TIMESTAMP
                 WHERE id=?1",
                params![
                    id,
                    if retry { "queued" } else { "failed" },
                    (Utc::now() + Duration::seconds(30 * (attempts + 1))).to_rfc3339(),
                    error.to_string()
                ],
            )?;
            if let Some(run_after) = rerun_after {
                transaction.execute(
                    "INSERT INTO jobs(kind, reason, run_after)
                     VALUES (?1, 'changes arrived during failed job', ?2)",
                    params![kind.as_str(), run_after],
                )?;
            }
            transaction.commit()?;
            record_source_failure(&connection, kind, &error.to_string())?;
            progress.idle(if retry {
                format!("{} failed; retry scheduled: {error}", kind.as_str())
            } else {
                format!("{} failed: {error}", kind.as_str())
            });
        }
    }
    Ok(true)
}

fn record_source_failure(connection: &Connection, kind: JobKind, error: &str) -> Result<()> {
    let source = match kind {
        JobKind::Contacts => Some(("id", "contacts")),
        JobKind::Imessage => Some(("id", "imessage")),
        JobKind::Whatsapp => Some(("id", "whatsapp")),
        JobKind::AppleCalls => Some(("id", "apple_calls")),
        JobKind::WhatsappCalls => Some(("id", "whatsapp_calls")),
        JobKind::Gmail => None,
        _ => None,
    };
    if let Some((column, value)) = source {
        connection.execute(
            &format!("UPDATE sources SET status='failed', error=?2 WHERE {column}=?1"),
            params![value, error],
        )?;
    }
    Ok(())
}

pub fn run(config_path: &Path, kind: JobKind) -> Result<()> {
    crate::job_runner::run(config_path, kind)
}

fn enqueue_downstream(connection: &Connection, kind: JobKind, rerun: bool) -> Result<()> {
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
            enqueue(
                connection,
                JobKind::Gmail,
                "contacts reconciled",
                Duration::zero(),
            )?;
        }
        JobKind::Imessage | JobKind::Whatsapp | JobKind::AppleCalls | JobKind::WhatsappCalls => {
            enqueue(
                connection,
                JobKind::Analysis,
                "new interactions",
                Duration::zero(),
            )?;
            enqueue(
                connection,
                JobKind::Suggestions,
                "new participants",
                Duration::zero(),
            )?;
        }
        JobKind::Gmail => {
            enqueue(
                connection,
                JobKind::Analysis,
                "new interactions",
                Duration::zero(),
            )?;
            enqueue(
                connection,
                JobKind::Suggestions,
                "new participants",
                Duration::zero(),
            )?;
            if crate::sync::gmail_backfill_pending(connection)? {
                enqueue(
                    connection,
                    JobKind::Gmail,
                    "people-focused backfill pending",
                    Duration::seconds(2),
                )?;
            }
        }
        JobKind::Analysis => {
            let pending = crate::analysis::has_pending(connection)?;
            if pending {
                enqueue(
                    connection,
                    JobKind::Analysis,
                    "new interactions arrived during analysis",
                    Duration::zero(),
                )?;
            }
            let importing: bool = connection.query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM jobs WHERE state IN ('queued', 'running')
                     AND kind IN ('imessage', 'whatsapp', 'apple_calls',
                                  'whatsapp_calls', 'gmail')
                 )",
                [],
                |row| row.get(0),
            )?;
            if !rerun && !pending && !importing {
                enqueue(
                    connection,
                    JobKind::Scoring,
                    "analysis queue drained",
                    Duration::zero(),
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
#[path = "jobs/tests.rs"]
mod tests;

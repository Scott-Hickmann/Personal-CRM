use std::path::Path;

use chrono::{Duration, Utc};
use rusqlite::{Connection, params};

use super::{WorkKind, request, table};
use crate::error::Result;
use crate::progress::ProgressTracker;
use crate::sync::SyncTarget;

pub(super) fn process(config_path: &Path, connection: &Connection, kind: WorkKind) -> Result<bool> {
    let (generation, reason): (i64, Option<String>) = connection.query_row(
        &format!(
            "SELECT requested_generation, reason FROM {} WHERE kind=?1",
            table(kind)
        ),
        [kind.as_str()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if connection.execute(
        &format!(
            "UPDATE {} SET state='running', running_generation=?2, attempts=attempts+1,
             updated_at=CURRENT_TIMESTAMP WHERE kind=?1 AND state='pending'",
            table(kind)
        ),
        params![kind.as_str(), generation],
    )? == 0
    {
        return Ok(false);
    }
    let mut progress =
        ProgressTracker::start(config_path, kind.as_str(), generation, reason.as_deref());
    let result = if kind.is_source() {
        process_source(
            config_path,
            connection,
            kind,
            generation,
            reason.as_deref(),
            &mut progress,
        )
    } else {
        process_maintenance(config_path, connection, kind, generation, &mut progress)
    };
    match result {
        Ok(()) => {
            progress.idle(format!("Completed {}", kind.as_str()));
            Ok(true)
        }
        Err(error) => {
            fail(connection, kind, &error.to_string())?;
            progress.idle(format!("{} failed: {error}", kind.as_str()));
            Ok(true)
        }
    }
}

fn process_source(
    config_path: &Path,
    connection: &Connection,
    kind: WorkKind,
    generation: i64,
    reason: Option<&str>,
    progress: &mut ProgressTracker,
) -> Result<()> {
    let config = crate::config::Config::load(config_path)?;
    let target = sync_target(kind);
    let step: String = connection.query_row(
        "SELECT step FROM source_sync_state WHERE kind=?1",
        [kind.as_str()],
        |row| row.get(0),
    )?;
    if step == "sync" {
        progress.phase("import", "Import source data", 1, 3);
        let reports = crate::sync::import_with_progress(target, &config, connection, progress)?;
        let sources = reports
            .iter()
            .map(|report| report.source.clone())
            .collect::<Vec<_>>();
        let changed = reports.iter().any(|report| report.changed);
        if changed {
            crate::relationships::mark_dirty(connection, &sources, kind == WorkKind::Contacts)?;
        }
        let sources_json = serde_json::to_string(&sources)
            .map_err(|error| crate::error::CrmError::Serialization(error.to_string()))?;
        connection.execute(
            "UPDATE source_sync_state SET step='relationships', changed=?2,
             affected_sources_json=?3, updated_at=CURRENT_TIMESTAMP WHERE kind=?1",
            params![kind.as_str(), changed, sources_json],
        )?;
    }
    let (step, changed, sources_json): (String, bool, String) = connection.query_row(
        "SELECT step, changed, affected_sources_json FROM source_sync_state WHERE kind=?1",
        [kind.as_str()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let sources: Vec<String> = serde_json::from_str(&sources_json)
        .map_err(|error| crate::error::CrmError::Serialization(error.to_string()))?;
    if step == "relationships" {
        progress.phase("relationships", "Reconcile relationships", 2, 3);
        let reports = sources
            .iter()
            .map(|source| crate::sync::SyncReport {
                source: source.clone(),
                imported: 0,
                deleted: 0,
                schema_fingerprint: String::new(),
                changed,
            })
            .collect::<Vec<_>>();
        crate::sync::reconcile_with_progress(target, connection, &reports, progress)?;
        crate::relationships::clear_dirty(connection, &sources, kind == WorkKind::Contacts)?;
        connection.execute(
            "UPDATE source_sync_state SET step='dirty_people', updated_at=CURRENT_TIMESTAMP
             WHERE kind=?1",
            [kind.as_str()],
        )?;
    }
    progress.phase("dirty_people", "Mark affected people", 3, 3);
    progress.stage("Marking affected people", 1, 1, 1, false, "step");
    if changed {
        crate::scoring::mark_dirty_for_sources(connection, kind == WorkKind::Contacts, &sources)?;
    }
    progress.finish_stage("Marked affected people", 1, 1, false, "step");
    let transaction = crate::db::immediate_transaction(connection)?;
    finish(&transaction, kind, generation)?;
    let downstream = request_downstream(
        &transaction,
        kind,
        changed,
        reason == Some("daemon startup"),
    )?;
    transaction.commit()?;
    for requested in downstream {
        progress.event(format!("Queued {}", requested.as_str()));
    }
    Ok(())
}

fn process_maintenance(
    config_path: &Path,
    connection: &Connection,
    kind: WorkKind,
    generation: i64,
    progress: &mut ProgressTracker,
) -> Result<()> {
    progress.phase(
        kind.as_str(),
        format!("Run {} maintenance", kind.as_str().replace('_', " ")),
        1,
        1,
    );
    match kind {
        WorkKind::Scoring => {
            crate::scoring::recalculate_dirty(connection, progress)?;
        }
        WorkKind::Photos => {
            crate::photos_commands::reconcile_automatic(config_path, progress)?;
        }
        WorkKind::GooglePublish => {
            crate::contact_commands::publish_automatic(config_path, progress)?;
        }
        WorkKind::Suggestions => {
            crate::review::enqueue_unresolved_candidates_with_progress(connection, progress)?;
        }
        _ => unreachable!(),
    };
    finish(connection, kind, generation)
}

fn finish(connection: &Connection, kind: WorkKind, generation: i64) -> Result<()> {
    let state = if kind.is_source() {
        "step='sync', changed=0, affected_sources_json='[]',"
    } else {
        ""
    };
    connection.execute(
        &format!(
            "UPDATE {} SET {state} completed_generation=?2, running_generation=NULL,
             state=CASE WHEN requested_generation>?2 THEN 'pending' ELSE 'idle' END,
             attempts=0, error=NULL, updated_at=CURRENT_TIMESTAMP WHERE kind=?1",
            table(kind)
        ),
        params![kind.as_str(), generation],
    )?;
    Ok(())
}

fn fail(connection: &Connection, kind: WorkKind, error: &str) -> Result<()> {
    let attempts: i64 = connection.query_row(
        &format!("SELECT attempts FROM {} WHERE kind=?1", table(kind)),
        [kind.as_str()],
        |row| row.get(0),
    )?;
    connection.execute(
        &format!(
            "UPDATE {} SET state=?2, run_after=?3, running_generation=NULL, error=?4,
             updated_at=CURRENT_TIMESTAMP WHERE kind=?1",
            table(kind)
        ),
        params![
            kind.as_str(),
            if attempts < 5 { "pending" } else { "failed" },
            (Utc::now() + Duration::seconds(30 * attempts.max(1))).to_rfc3339(),
            error
        ],
    )?;
    Ok(())
}

fn request_downstream(
    connection: &Connection,
    kind: WorkKind,
    changed: bool,
    daemon_startup: bool,
) -> Result<Vec<WorkKind>> {
    let mut requested = Vec::new();
    if changed {
        request(
            connection,
            WorkKind::Scoring,
            "source data changed",
            Duration::zero(),
        )?;
        requested.push(WorkKind::Scoring);
        request(
            connection,
            WorkKind::Suggestions,
            "source data changed",
            Duration::zero(),
        )?;
        requested.push(WorkKind::Suggestions);
    }
    if kind == WorkKind::Contacts && (changed || daemon_startup) {
        request(
            connection,
            WorkKind::GooglePublish,
            if changed {
                "contacts changed"
            } else {
                "daemon startup"
            },
            Duration::zero(),
        )?;
        requested.push(WorkKind::GooglePublish);
    }
    if kind == WorkKind::Contacts && changed {
        request(
            connection,
            WorkKind::Gmail,
            "contact identities changed",
            Duration::zero(),
        )?;
        requested.push(WorkKind::Gmail);
    }
    if kind == WorkKind::Gmail && crate::sync::gmail_backfill_pending(connection)? {
        request(
            connection,
            WorkKind::Gmail,
            "Gmail backfill pending",
            Duration::seconds(2),
        )?;
        requested.push(WorkKind::Gmail);
    }
    Ok(requested)
}

fn sync_target(kind: WorkKind) -> SyncTarget {
    match kind {
        WorkKind::Contacts => SyncTarget::Contacts,
        WorkKind::Imessage => SyncTarget::Imessage,
        WorkKind::Whatsapp => SyncTarget::Whatsapp,
        WorkKind::AppleCalls => SyncTarget::AppleCalls,
        WorkKind::WhatsappCalls => SyncTarget::WhatsappCalls,
        WorkKind::Gmail => SyncTarget::Gmail,
        _ => unreachable!(),
    }
}

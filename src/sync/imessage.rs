use rusqlite::Connection;
use std::collections::HashSet;

use super::{SyncReport, add_participant, finish_source, open_source, upsert_interaction};
use crate::error::{CrmError, Result};
use crate::progress::ProgressTracker;

pub fn sync(
    config: &crate::config::Config,
    crm: &Connection,
    progress: &mut ProgressTracker,
    stage_current: u64,
    stage_total: u64,
) -> Result<SyncReport> {
    let path = config
        .paths
        .imessage
        .as_ref()
        .ok_or_else(|| CrmError::InvalidConfig("iMessage path is not configured".into()))?;
    let (source, fingerprint, run_at) = open_source(
        crm,
        "imessage",
        "imessage",
        path,
        "message",
        &["guid", "date", "is_from_me", "text"],
    )?;
    let mut statement = source.connection().prepare(
        "SELECT m.guid, COALESCE(c.guid, c.chat_identifier), COALESCE(m.service, 'iMessage'),
                datetime((m.date / 1000000000) + 978307200, 'unixepoch'), m.is_from_me,
                m.subject, m.text, h.id, m.cache_has_attachments,
                CASE WHEN participants.handle_count = 1 THEN NULLIF(c.display_name, '') END,
                COUNT(*) OVER()
         FROM message m
         LEFT JOIN chat_message_join cmj ON cmj.message_id = m.ROWID
         LEFT JOIN chat c ON c.ROWID = cmj.chat_id
         LEFT JOIN handle h ON h.ROWID = m.handle_id
         LEFT JOIN (
             SELECT chat_id, COUNT(*) AS handle_count FROM chat_handle_join GROUP BY chat_id
         ) participants ON participants.chat_id = c.ROWID
         WHERE m.guid IS NOT NULL AND m.date IS NOT NULL AND m.is_system_message = 0",
    )?;
    let mut imported = HashSet::new();
    let mut rows = statement.query([])?;
    let mut processed = 0_u64;
    let mut total = 0_u64;
    progress.stage(
        "Reading iMessage conversations",
        stage_current,
        stage_total,
        1,
        false,
        "query",
    );
    while let Some(row) = rows.next()? {
        total = u64::try_from(row.get::<_, i64>(10)?).unwrap_or_default();
        let native_id: String = row.get(0)?;
        imported.insert(native_id.clone());
        let from_me = row.get::<_, i64>(4)? != 0;
        let identity: Option<String> = row.get(7)?;
        let interaction_id = upsert_interaction(
            crm,
            "imessage",
            &native_id,
            row.get::<_, Option<String>>(1)?.as_deref(),
            &row.get::<_, String>(2)?,
            "message",
            &row.get::<_, String>(3)?,
            Some(if from_me { "outgoing" } else { "incoming" }),
            row.get::<_, Option<String>>(5)?.as_deref(),
            row.get::<_, Option<String>>(6)?.as_deref(),
            &serde_json::json!({"has_attachments": row.get::<_, i64>(8)? != 0}),
            &run_at,
        )?;
        if let Some(identity) = identity {
            add_participant(
                crm,
                &interaction_id,
                &identity,
                row.get::<_, Option<String>>(9)?.as_deref(),
                if from_me { "recipient" } else { "sender" },
            )?;
        }
        processed += 1;
        progress.progress(
            "Reading iMessage conversations",
            processed,
            total,
            false,
            "messages",
        );
    }
    progress.finish_stage(
        "Read iMessage conversations",
        processed,
        total,
        false,
        "messages",
    );
    progress.progress_now("Finalizing iMessage sync", 0, 1, false, "step");
    let deleted = finish_source(crm, "imessage", &run_at)?;
    progress.finish_stage("Finalized iMessage sync", 1, 1, false, "step");
    Ok(SyncReport {
        source: "imessage".into(),
        imported: imported.len(),
        deleted,
        schema_fingerprint: fingerprint,
    })
}

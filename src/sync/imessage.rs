use rusqlite::Connection;
use std::collections::HashSet;

use super::incremental::{finish_incremental_source, incremental_floor, open_incremental_source};
use super::{SyncReport, add_participant, upsert_interaction};
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
    let source = open_incremental_source(
        crm,
        "imessage",
        "imessage",
        path,
        "message",
        &["guid", "date", "is_from_me", "text"],
    )?;
    source.require_columns("chat_handle_join", &["chat_id", "handle_id"])?;
    source.require_columns("handle", &["ROWID", "id"])?;
    let mut statement = source.connection().prepare(
        "SELECT m.guid, COALESCE(c.guid, c.chat_identifier), COALESCE(m.service, 'iMessage'),
                datetime((m.date / 1000000000) + 978307200, 'unixepoch'), m.is_from_me,
                m.subject, m.text, h.id, m.cache_has_attachments,
                CASE WHEN participants.handle_count = 1 THEN NULLIF(c.display_name, '') END, m.ROWID,
                COUNT(*) OVER()
         FROM message m
         LEFT JOIN chat_message_join cmj ON cmj.message_id = m.ROWID
         LEFT JOIN chat c ON c.ROWID = cmj.chat_id
         LEFT JOIN handle h ON h.ROWID = m.handle_id
         LEFT JOIN (
             SELECT chat_id, COUNT(*) AS handle_count FROM chat_handle_join GROUP BY chat_id
         ) participants ON participants.chat_id = c.ROWID
         WHERE m.guid IS NOT NULL AND m.date IS NOT NULL AND m.is_system_message = 0
           AND m.ROWID > ?1",
    )?;
    let mut imported = HashSet::new();
    let mut processed = 0_u64;
    let mut total = 0_u64;
    let mut cursor = if source.audit { 0 } else { source.cursor };
    progress.stage(
        "Reading iMessage conversations",
        stage_current,
        stage_total,
        1,
        false,
        "query",
    );
    let mut rows = statement.query([incremental_floor(&source)])?;
    while let Some(row) = rows.next()? {
        total = u64::try_from(row.get::<_, i64>(11)?).unwrap_or_default();
        cursor = cursor.max(row.get(10)?);
        let native_id: String = row.get(0)?;
        imported.insert(native_id.clone());
        let from_me = row.get::<_, i64>(4)? != 0;
        let identity: Option<String> = row.get(7)?;
        let display_name: Option<String> = row.get(9)?;
        let occurred_at: String = row.get(3)?;
        let subject: Option<String> = row.get(5)?;
        progress.focus([format!(
            "{} · iMessage · {} · {}",
            display_name
                .as_deref()
                .or(identity.as_deref())
                .unwrap_or("Unknown participant"),
            subject
                .as_deref()
                .unwrap_or(if from_me { "outgoing" } else { "incoming" }),
            occurred_at.get(..10).unwrap_or(&occurred_at),
        )]);
        let thread_native_id: Option<String> = row.get(1)?;
        let interaction_id = upsert_interaction(
            crm,
            "imessage",
            &native_id,
            thread_native_id.as_deref(),
            &row.get::<_, String>(2)?,
            "message",
            &occurred_at,
            Some(if from_me { "outgoing" } else { "incoming" }),
            subject.as_deref(),
            row.get::<_, Option<String>>(6)?.as_deref(),
            &serde_json::json!({"has_attachments": row.get::<_, i64>(8)? != 0}),
            &source.run_at,
        )?;
        if let Some(identity) = identity {
            add_participant(
                crm,
                &interaction_id,
                &identity,
                display_name.as_deref(),
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
    drop(rows);
    drop(statement);
    refresh_memberships(source.connection(), crm)?;
    progress.finish_stage(
        "Read iMessage conversations",
        processed,
        total,
        false,
        "messages",
    );
    progress.progress_now("Finalizing iMessage sync", 0, 1, false, "step");
    let deleted = finish_incremental_source(crm, "imessage", &source, cursor)?;
    progress.finish_stage("Finalized iMessage sync", 1, 1, false, "step");
    Ok(SyncReport {
        source: "imessage".into(),
        imported: imported.len(),
        deleted,
        schema_fingerprint: source.fingerprint.clone(),
        changed: !imported.is_empty() || deleted > 0,
    })
}

fn refresh_memberships(source: &Connection, crm: &Connection) -> Result<()> {
    let mut chats = source.prepare(
        "SELECT ROWID, COALESCE(guid, chat_identifier), NULLIF(display_name, '') FROM chat
         WHERE COALESCE(guid, chat_identifier) IS NOT NULL",
    )?;
    let rows = chats
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut roster = source.prepare(
        "SELECT h.id FROM chat_handle_join chj JOIN handle h ON h.ROWID=chj.handle_id
         WHERE chj.chat_id=?1 ORDER BY h.id",
    )?;
    for (chat_id, thread_native_id, title) in rows {
        let identities = roster
            .query_map([chat_id], |member| member.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let members = identities
            .iter()
            .map(|identity| crate::relationships::ConversationMember {
                identity,
                display_name: None,
            })
            .collect::<Vec<_>>();
        crate::relationships::replace_members(
            crm,
            "imessage",
            &thread_native_id,
            title.as_deref(),
            &members,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use rusqlite::params;

    #[test]
    fn group_roster_includes_silent_members() {
        let source = Connection::open_in_memory().unwrap();
        source
            .execute_batch(
                "CREATE TABLE chat(ROWID INTEGER PRIMARY KEY, guid TEXT, chat_identifier TEXT, display_name TEXT);
             CREATE TABLE handle(ROWID INTEGER PRIMARY KEY, id TEXT NOT NULL);
             CREATE TABLE chat_handle_join(chat_id INTEGER, handle_id INTEGER);
             INSERT INTO chat VALUES (1, 'group', 'group', 'Weekend crew');
             INSERT INTO handle VALUES (1, '+15550100'), (2, '+15550200');
             INSERT INTO chat_handle_join VALUES (1, 1), (1, 2);",
            )
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let crm = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        crm.execute(
            "INSERT INTO sources(id, kind) VALUES ('imessage', 'imessage')",
            [],
        )
        .unwrap();
        for (id, name, identity) in [("a", "Alex", "15550100"), ("b", "Blair", "15550200")] {
            crm.execute(
                "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
                 VALUES (?1, ?2, ?1, 'active')",
                params![id, name],
            )
            .unwrap();
            crm.execute(
                "INSERT INTO identities(id, person_id, kind, value, normalized_value)
                 VALUES (?1, ?1, 'phone', ?2, ?2)",
                params![id, identity],
            )
            .unwrap();
        }

        refresh_memberships(&source, &crm).unwrap();

        let count: i64 = crm
            .query_row(
                "SELECT COUNT(DISTINCT person_id) FROM conversation_memberships
             WHERE source_id='imessage' AND thread_native_id='group'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }
}

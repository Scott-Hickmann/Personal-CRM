mod calls;
mod contacts;
mod gmail;
mod gmail_message;
mod imessage;
mod whatsapp;

use std::path::Path;

use base64::Engine as _;
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use uuid::Uuid;

use crate::error::Result;
use crate::source::ReadOnlySource;

#[derive(Debug, Clone, Copy)]
pub enum SyncTarget {
    Contacts,
    Imessage,
    Whatsapp,
    Calls,
    Gmail,
}

#[derive(Debug, Serialize)]
pub struct SyncReport {
    pub source: String,
    pub imported: usize,
    pub deleted: usize,
    pub schema_fingerprint: String,
}

pub fn run(
    target: SyncTarget,
    config: &crate::config::Config,
    crm: &Connection,
) -> Result<Vec<SyncReport>> {
    let mut reports = Vec::new();
    if matches!(target, SyncTarget::Contacts) {
        let transaction = crm.unchecked_transaction()?;
        reports.push(contacts::sync(config, &transaction)?);
        transaction.commit()?;
    }
    if matches!(target, SyncTarget::Imessage) {
        let transaction = crm.unchecked_transaction()?;
        reports.push(imessage::sync(config, &transaction)?);
        transaction.commit()?;
    }
    if matches!(target, SyncTarget::Whatsapp) {
        let transaction = crm.unchecked_transaction()?;
        reports.push(whatsapp::sync(config, &transaction)?);
        transaction.commit()?;
    }
    if matches!(target, SyncTarget::Calls) {
        let transaction = crm.unchecked_transaction()?;
        reports.extend(calls::sync(config, &transaction)?);
        transaction.commit()?;
    }
    if matches!(target, SyncTarget::Gmail) && !config.gmail.accounts.is_empty() {
        let transaction = crm.unchecked_transaction()?;
        reports.extend(gmail::sync(config, &transaction)?);
        transaction.commit()?;
    }
    Ok(reports)
}

fn open_source(
    crm: &Connection,
    id: &str,
    kind: &str,
    path: &Path,
    table: &str,
    columns: &[&str],
) -> Result<(ReadOnlySource, String, String)> {
    let source = ReadOnlySource::open(path)?;
    source.require_columns(table, columns)?;
    let fingerprint = source.schema_fingerprint()?;
    let run_at = Utc::now().to_rfc3339();
    crm.execute(
        "INSERT INTO sources(id, kind, schema_fingerprint, status) VALUES (?1, ?2, ?3, 'syncing')
         ON CONFLICT(id) DO UPDATE SET schema_fingerprint = excluded.schema_fingerprint, status = 'syncing', error = NULL",
        params![id, kind, fingerprint],
    )?;
    Ok((source, fingerprint, run_at))
}

fn finish_source(crm: &Connection, id: &str, run_at: &str) -> Result<usize> {
    let mut statement = crm.prepare(
        "SELECT native_id FROM interactions WHERE source_id = ?1 AND deleted_at IS NULL AND last_seen_at < ?2",
    )?;
    let deleted_ids: Vec<String> = statement
        .query_map(params![id, run_at], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    for native_id in &deleted_ids {
        crm.execute(
            "INSERT OR IGNORE INTO tombstones(source_id, native_id) VALUES (?1, ?2)",
            params![id, native_id],
        )?;
    }
    crm.execute(
        "UPDATE interactions SET body = NULL, subject = NULL, deleted_at = CURRENT_TIMESTAMP
         WHERE source_id = ?1 AND deleted_at IS NULL AND last_seen_at < ?2",
        params![id, run_at],
    )?;
    crm.execute(
        "UPDATE sources SET status = 'ok', last_sync_at = CURRENT_TIMESTAMP, last_reconcile_at = CURRENT_TIMESTAMP WHERE id = ?1",
        [id],
    )?;
    Ok(deleted_ids.len())
}

#[allow(clippy::too_many_arguments)]
fn upsert_interaction(
    crm: &Connection,
    source_id: &str,
    native_id: &str,
    thread_id: Option<&str>,
    channel: &str,
    kind: &str,
    occurred_at: &str,
    direction: Option<&str>,
    subject: Option<&str>,
    body: Option<&str>,
    metadata: &serde_json::Value,
    run_at: &str,
) -> Result<String> {
    let id = crm
        .query_row(
            "SELECT id FROM interactions WHERE source_id = ?1 AND native_id = ?2",
            params![source_id, native_id],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    crm.execute(
        "INSERT INTO interactions(id, source_id, native_id, thread_native_id, channel, kind, occurred_at, direction, subject, body, metadata_json, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(source_id, native_id) DO UPDATE SET thread_native_id=excluded.thread_native_id, channel=excluded.channel, kind=excluded.kind, occurred_at=excluded.occurred_at, direction=excluded.direction, subject=excluded.subject, body=excluded.body, metadata_json=excluded.metadata_json, deleted_at=NULL, last_seen_at=excluded.last_seen_at",
        params![id, source_id, native_id, thread_id, channel, kind, occurred_at, direction, subject, body, metadata.to_string(), run_at],
    )?;
    Ok(id)
}

fn add_participant(
    crm: &Connection,
    interaction_id: &str,
    identity: &str,
    display_name: Option<&str>,
    role: &str,
) -> Result<()> {
    let normalized = crate::repository::normalize_observed_identity(identity);
    let display_name = usable_display_name(identity, display_name);
    let person_id: Option<String> = crm
        .query_row(
            "SELECT i.person_id FROM identities i JOIN people p ON p.id=i.person_id
             WHERE i.normalized_value=?1 AND i.active=1 AND p.lifecycle_state='active' LIMIT 1",
            [normalized],
            |row| row.get(0),
        )
        .optional()?;
    crm.execute(
        "INSERT OR REPLACE INTO interaction_participants(
             interaction_id, person_id, identity_value, display_name, role
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![interaction_id, person_id, identity, display_name, role],
    )?;
    Ok(())
}

fn usable_display_name<'a>(identity: &str, name: Option<&'a str>) -> Option<&'a str> {
    let name = name.map(str::trim).filter(|name| !name.is_empty())?;
    if !name.chars().any(char::is_alphabetic)
        || crate::repository::normalize_observed_identity(name)
            == crate::repository::normalize_observed_identity(identity)
        || looks_encoded_name(name)
    {
        None
    } else {
        Some(name)
    }
}

fn looks_encoded_name(value: &str) -> bool {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .is_ok_and(|decoded| {
            decoded
                .iter()
                .any(|byte| *byte == 0 || (*byte < b' ' && !matches!(*byte, b'\t' | b'\n' | b'\r')))
        })
}

pub(crate) fn rebind_unresolved_participants(crm: &Connection) -> Result<usize> {
    let mut statement = crm.prepare(
        "SELECT interaction_id, identity_value, role FROM interaction_participants
         WHERE person_id IS NULL AND identity_value IS NOT NULL",
    )?;
    let rows: Vec<(String, String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    let mut rebound = 0;
    for (interaction_id, identity, role) in rows {
        let normalized = crate::repository::normalize_observed_identity(&identity);
        let person_id: Option<String> = crm
            .query_row(
                "SELECT i.person_id FROM identities i JOIN people p ON p.id=i.person_id
                 WHERE i.normalized_value=?1 AND i.active=1 AND p.lifecycle_state='active' LIMIT 1",
                [normalized],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(person_id) = person_id {
            crm.execute(
                "UPDATE interaction_participants SET person_id=?4
                 WHERE interaction_id=?1 AND identity_value=?2 AND role=?3",
                params![interaction_id, identity, role, person_id],
            )?;
            rebound += 1;
        }
    }
    Ok(rebound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_human_source_names() {
        assert_eq!(
            usable_display_name("+15550100", Some(" Alex Example ")),
            Some("Alex Example")
        );
    }

    #[test]
    fn rejects_encoded_or_numeric_source_names() {
        assert_eq!(usable_display_name("+15550100", Some("IAA=")), None);
        assert_eq!(usable_display_name("+15550100", Some("IABoAXAB")), None);
        assert_eq!(usable_display_name("+15550100", Some("15550100")), None);
    }
}

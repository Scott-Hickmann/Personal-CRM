mod calls;
mod contacts;
mod gmail;
mod gmail_backfill;
mod gmail_import;
pub(crate) mod gmail_message;
mod gmail_store;
mod imessage;
mod incremental;
mod whatsapp;
mod whatsapp_identity;

use base64::Engine as _;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use uuid::Uuid;

use crate::error::Result;
use crate::progress::ProgressTracker;

#[derive(Debug, Clone, Copy)]
pub enum SyncTarget {
    Contacts,
    Imessage,
    Whatsapp,
    AppleCalls,
    WhatsappCalls,
    Gmail,
}

#[derive(Debug, Serialize)]
pub struct SyncReport {
    pub source: String,
    pub imported: usize,
    pub deleted: usize,
    pub schema_fingerprint: String,
    pub changed: bool,
}

pub fn run(
    target: SyncTarget,
    config: &crate::config::Config,
    crm: &Connection,
) -> Result<Vec<SyncReport>> {
    let mut progress = ProgressTracker::disabled();
    run_with_progress(target, config, crm, &mut progress)
}

pub(crate) fn run_with_progress(
    target: SyncTarget,
    config: &crate::config::Config,
    crm: &Connection,
    progress: &mut ProgressTracker,
) -> Result<Vec<SyncReport>> {
    let reports = import_with_progress(target, config, crm, progress)?;
    reconcile_with_progress(target, crm, &reports, progress)?;
    Ok(reports)
}

pub(crate) fn import_with_progress(
    target: SyncTarget,
    config: &crate::config::Config,
    crm: &Connection,
    progress: &mut ProgressTracker,
) -> Result<Vec<SyncReport>> {
    let mut reports = Vec::new();
    if matches!(target, SyncTarget::Contacts) {
        reports.push(contacts::sync(config, crm, progress)?);
    }
    if matches!(target, SyncTarget::Imessage) {
        reports.push(imessage::sync(config, crm, progress, 1, 1)?);
    }
    if matches!(target, SyncTarget::Whatsapp) {
        reports.push(whatsapp::sync(config, crm, progress, 1, 1)?);
    }
    if matches!(target, SyncTarget::AppleCalls) {
        reports.push(calls::sync_apple(config, crm, progress, 1, 1)?);
    }
    if matches!(target, SyncTarget::WhatsappCalls) {
        reports.push(calls::sync_whatsapp(config, crm, progress, 1, 1)?);
    }
    if matches!(target, SyncTarget::Gmail) && !config.gmail.accounts.is_empty() {
        reports.extend(gmail::sync(config, crm, progress)?);
    }
    Ok(reports)
}

pub(crate) fn reconcile_with_progress(
    target: SyncTarget,
    crm: &Connection,
    reports: &[SyncReport],
    progress: &mut ProgressTracker,
) -> Result<()> {
    if matches!(target, SyncTarget::Contacts) {
        progress.stage("Rebinding contact identities", 1, 2, 1, false, "query");
        crate::relationships::rebind_unresolved_members(crm)?;
        progress.finish_stage("Rebound contact identities", 1, 1, false, "query");
        progress.stage("Rebuilding relationship contexts", 2, 2, 1, false, "query");
        crate::relationships::reconcile_all(crm)?;
        progress.finish_stage("Rebuilt relationship contexts", 1, 1, false, "query");
    } else {
        let total = reports.len() as u64;
        progress.stage(
            "Rebuilding relationship contexts",
            1,
            1,
            total,
            false,
            "sources",
        );
        for (index, source) in reports.iter().enumerate() {
            progress.focus_now([source.source.clone()]);
            if matches!(target, SyncTarget::Gmail) {
                crate::relationships::rebuild_members_from_interactions(crm, &source.source)?;
            }
            crate::relationships::reconcile_source(crm, &source.source)?;
            progress.progress_now(
                "Rebuilding relationship contexts",
                (index + 1) as u64,
                total,
                false,
                "sources",
            );
        }
        progress.finish_stage(
            "Rebuilt relationship contexts",
            total,
            total,
            false,
            "sources",
        );
    }
    Ok(())
}

pub(crate) fn gmail_backfill_pending(crm: &Connection) -> Result<bool> {
    gmail_backfill::has_pending(crm)
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
         ON CONFLICT(source_id, native_id) DO UPDATE SET
           thread_native_id=excluded.thread_native_id, channel=excluded.channel,
           kind=excluded.kind, occurred_at=excluded.occurred_at,
           direction=excluded.direction, subject=excluded.subject, body=excluded.body,
           metadata_json=excluded.metadata_json, deleted_at=NULL,
           last_seen_at=excluded.last_seen_at",
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

fn replace_participant(
    crm: &Connection,
    interaction_id: &str,
    identity: &str,
    display_name: Option<&str>,
    role: &str,
) -> Result<()> {
    crm.execute(
        "DELETE FROM interaction_participants WHERE interaction_id=?1 AND role=?2",
        params![interaction_id, role],
    )?;
    add_participant(crm, interaction_id, identity, display_name, role)
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

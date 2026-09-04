use std::collections::HashSet;

use chrono::{TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use super::gmail_message::{Mailbox, addresses, header, import_attachments, message_text};
use super::{add_participant, upsert_interaction};
use crate::error::{CrmError, Result};
use crate::gmail::GmailMessage;

pub(super) fn known_emails(crm: &Connection) -> Result<HashSet<String>> {
    let mut statement = crm.prepare(
        "SELECT DISTINCT lower(trim(i.normalized_value))
         FROM identities i JOIN people p ON p.id=i.person_id
         WHERE i.kind='email' AND i.active=1 AND i.is_self=0
           AND p.lifecycle_state='active' AND p.apple_contact_id IS NOT NULL",
    )?;
    Ok(statement
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?)
}

pub(super) fn persist_message(
    crm: &Connection,
    source_id: &str,
    message: &GmailMessage,
    outgoing: bool,
    candidate_eligible: bool,
    participants: &[Mailbox],
) -> Result<()> {
    let millis = message.internal_date.parse::<i64>().map_err(|_| {
        CrmError::Network(format!(
            "Gmail message {} has invalid internalDate",
            message.id
        ))
    })?;
    let occurred_at = Utc
        .timestamp_millis_opt(millis)
        .single()
        .ok_or_else(|| {
            CrmError::Network(format!(
                "Gmail message {} has out-of-range internalDate",
                message.id
            ))
        })?
        .to_rfc3339();
    let run_at = Utc::now().to_rfc3339();
    let interaction_id = upsert_interaction(
        crm,
        source_id,
        &message.id,
        Some(&message.thread_id),
        "gmail",
        "email",
        &occurred_at,
        Some(if outgoing { "outgoing" } else { "incoming" }),
        header(message, "Subject").as_deref(),
        message_text(message).as_deref(),
        &serde_json::json!({
            "classification": "human",
            "candidate_eligible": candidate_eligible,
            "labels": message.label_ids,
        }),
        &run_at,
    )?;
    for table in ["interaction_participants", "attachments"] {
        crm.execute(
            &format!("DELETE FROM {table} WHERE interaction_id=?1"),
            [&interaction_id],
        )?;
    }
    let from_addresses: HashSet<_> = addresses(&header(message, "From").unwrap_or_default())
        .into_iter()
        .collect();
    for participant in participants {
        let role = if !outgoing && from_addresses.contains(&participant.email) {
            "sender"
        } else {
            "recipient"
        };
        add_participant(
            crm,
            &interaction_id,
            &participant.email,
            participant.name.as_deref(),
            role,
        )?;
        crate::relationships::observe_member(
            crm,
            source_id,
            &message.thread_id,
            crate::relationships::ConversationMember {
                identity: &participant.email,
                display_name: participant.name.as_deref(),
            },
        )?;
    }
    import_attachments(crm, &interaction_id, &message.id, &message.payload)?;
    Ok(())
}

pub(super) fn discard_message(crm: &Connection, source_id: &str, native_id: &str) -> Result<bool> {
    let interaction_id: Option<String> = crm
        .query_row(
            "SELECT id FROM interactions WHERE source_id=?1 AND native_id=?2
             AND deleted_at IS NULL",
            params![source_id, native_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(interaction_id) = interaction_id else {
        return Ok(false);
    };
    crm.execute(
        "DELETE FROM attachments WHERE interaction_id=?1",
        [&interaction_id],
    )?;
    crm.execute(
        "UPDATE interactions SET body=NULL, subject=NULL, deleted_at=CURRENT_TIMESTAMP WHERE id=?1",
        [&interaction_id],
    )?;
    crm.execute(
        "INSERT OR IGNORE INTO tombstones(source_id, native_id) VALUES (?1, ?2)",
        params![source_id, native_id],
    )?;
    Ok(true)
}

pub(super) fn prune_legacy_noise(crm: &Connection, source_id: &str) -> Result<usize> {
    let mut statement = crm.prepare(
        "SELECT i.native_id FROM interactions i
         WHERE i.source_id=?1 AND i.deleted_at IS NULL AND (
           json_extract(i.metadata_json, '$.classification')='automated'
           OR i.metadata_json LIKE '%CATEGORY_PROMOTIONS%'
           OR i.metadata_json LIKE '%CATEGORY_SOCIAL%'
           OR i.metadata_json LIKE '%CATEGORY_FORUMS%'
           OR (
             json_extract(i.metadata_json, '$.candidate_eligible') IS NOT 1
             AND NOT EXISTS (
               SELECT 1 FROM interaction_participants ip
               JOIN people p ON p.id=ip.person_id
               WHERE ip.interaction_id=i.id AND p.lifecycle_state='active'
                 AND p.apple_contact_id IS NOT NULL
             )
           )
         )",
    )?;
    let native_ids: Vec<String> = statement
        .query_map([source_id], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    for native_id in &native_ids {
        discard_message(crm, source_id, native_id)?;
    }
    Ok(native_ids.len())
}

pub(super) fn prune_ignored_domains(
    crm: &Connection,
    source_id: &str,
    ignored_domains: &[String],
) -> Result<usize> {
    if ignored_domains.is_empty() {
        return Ok(0);
    }
    let mut statement = crm.prepare(
        "SELECT DISTINCT i.native_id, ip.identity_value
         FROM interactions i
         JOIN interaction_participants ip ON ip.interaction_id=i.id
         WHERE i.source_id=?1 AND i.deleted_at IS NULL AND ip.identity_value IS NOT NULL",
    )?;
    let native_ids = statement
        .query_map([source_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|(_, email)| crate::config::email_domain_is_ignored(email, ignored_domains))
        .map(|(native_id, _)| native_id)
        .collect::<HashSet<_>>();
    drop(statement);
    let count = native_ids.len();
    for native_id in &native_ids {
        discard_message(crm, source_id, native_id)?;
    }
    Ok(count)
}

#[cfg(test)]
#[path = "gmail_store_tests.rs"]
mod tests;

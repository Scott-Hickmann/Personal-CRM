use rusqlite::Connection;
use std::collections::HashSet;

use super::incremental::{
    delete_interactions, finish_incremental_source, incremental_floor, open_incremental_source,
};
use super::whatsapp_identity::LidResolver;
use super::{SyncReport, replace_participant, upsert_interaction};
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
        .whatsapp
        .as_ref()
        .ok_or_else(|| CrmError::InvalidConfig("WhatsApp path is not configured".into()))?;
    let identities = LidResolver::load(path)?;
    let source = open_incremental_source(
        crm,
        "whatsapp",
        "whatsapp",
        path,
        "ZWAMESSAGE",
        &["Z_PK", "ZMESSAGEDATE", "ZISFROMME", "ZTEXT"],
    )?;
    source.require_columns("ZWAPROFILEPUSHNAME", &["ZJID", "ZPUSHNAME"])?;
    source.require_columns(
        "ZWACHATSESSION",
        &["Z_PK", "ZCONTACTJID", "ZPARTNERNAME", "ZREMOVED"],
    )?;
    source.require_columns(
        "ZWAGROUPMEMBER",
        &[
            "ZCHATSESSION",
            "ZISACTIVE",
            "ZMEMBERJID",
            "ZCONTACTNAME",
            "ZFIRSTNAME",
        ],
    )?;
    let mut statement = source.connection().prepare(
        "SELECT m.Z_PK, m.ZSTANZAID, s.ZCONTACTJID, datetime(m.ZMESSAGEDATE + 978307200, 'unixepoch'),
                m.ZISFROMME, m.ZTEXT, m.ZFROMJID, m.ZTOJID, m.ZMESSAGETYPE,
                profile.ZPUSHNAME, m.ZPUSHNAME, s.ZPARTNERNAME, COUNT(*) OVER()
         FROM ZWAMESSAGE m JOIN ZWACHATSESSION s ON s.Z_PK = m.ZCHATSESSION
         LEFT JOIN ZWAPROFILEPUSHNAME profile
           ON profile.ZJID = CASE WHEN m.ZISFROMME = 1 THEN m.ZTOJID ELSE m.ZFROMJID END
         WHERE m.ZMESSAGEDATE IS NOT NULL AND COALESCE(s.ZREMOVED, 0) = 0
           AND m.Z_PK > ?1",
    )?;
    let mut imported = HashSet::new();
    let mut processed = 0_u64;
    let mut total = 0_u64;
    let mut cursor = if source.audit { 0 } else { source.cursor };
    progress.stage(
        "Reading WhatsApp conversations",
        stage_current,
        stage_total,
        1,
        false,
        "query",
    );
    let mut rows = statement.query([incremental_floor(&source)])?;
    while let Some(row) = rows.next()? {
        total = u64::try_from(row.get::<_, i64>(12)?).unwrap_or_default();
        let pk: i64 = row.get(0)?;
        cursor = cursor.max(pk);
        let native_id = row
            .get::<_, Option<String>>(1)?
            .unwrap_or_else(|| format!("pk:{pk}"));
        imported.insert(native_id.clone());
        let from_me = row.get::<_, i64>(4)? != 0;
        let sender: Option<String> = row.get(6)?;
        let recipient: Option<String> = row.get(7)?;
        let profile_name: Option<String> = row.get(9)?;
        let push_name: Option<String> = row.get(10)?;
        let partner_name: Option<String> = row.get(11)?;
        let display_name = if from_me {
            profile_name.or(partner_name)
        } else {
            profile_name.or(push_name).or(partner_name)
        };
        let occurred_at: String = row.get(3)?;
        let identity = if from_me {
            recipient.as_deref()
        } else {
            sender.as_deref()
        };
        progress.focus([format!(
            "{} · WhatsApp · {} · {}",
            display_name
                .as_deref()
                .or(identity)
                .unwrap_or("Unknown participant"),
            if from_me { "outgoing" } else { "incoming" },
            occurred_at.get(..10).unwrap_or(&occurred_at),
        )]);
        let thread_native_id: Option<String> = row.get(2)?;
        let interaction_id = upsert_interaction(
            crm,
            "whatsapp",
            &native_id,
            thread_native_id.as_deref(),
            "whatsapp",
            "message",
            &occurred_at,
            Some(if from_me { "outgoing" } else { "incoming" }),
            None,
            row.get::<_, Option<String>>(5)?.as_deref(),
            &serde_json::json!({"message_type": row.get::<_, i64>(8)?}),
            &source.run_at,
        )?;
        if let Some(identity) = identity {
            replace_participant(
                crm,
                &interaction_id,
                identities.resolve(identity),
                display_name.as_deref(),
                if from_me { "recipient" } else { "sender" },
            )?;
        }
        processed += 1;
        progress.progress(
            "Reading WhatsApp conversations",
            processed,
            total,
            false,
            "messages",
        );
    }
    drop(rows);
    drop(statement);
    refresh_memberships(source.connection(), crm, &identities)?;
    progress.finish_stage(
        "Read WhatsApp conversations",
        processed,
        total,
        false,
        "messages",
    );
    progress.progress_now("Finalizing WhatsApp sync", 0, 1, false, "step");
    let removed = removed_message_ids(source.connection())?;
    let deleted = delete_interactions(crm, "whatsapp", removed)?
        + finish_incremental_source(crm, "whatsapp", &source, cursor)?;
    progress.finish_stage("Finalized WhatsApp sync", 1, 1, false, "step");
    Ok(SyncReport {
        source: "whatsapp".into(),
        imported: imported.len(),
        deleted,
        schema_fingerprint: source.fingerprint.clone(),
        changed: !imported.is_empty() || deleted > 0,
    })
}

fn refresh_memberships(
    source: &Connection,
    crm: &Connection,
    identities: &LidResolver,
) -> Result<()> {
    let mut chats = source.prepare(
        "SELECT Z_PK, ZCONTACTJID, NULLIF(ZPARTNERNAME, '') FROM ZWACHATSESSION
         WHERE COALESCE(ZREMOVED, 0)=0 AND ZCONTACTJID IS NOT NULL",
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
        "SELECT ZMEMBERJID, COALESCE(NULLIF(ZCONTACTNAME, ''), NULLIF(ZFIRSTNAME, ''))
         FROM ZWAGROUPMEMBER
         WHERE ZCHATSESSION=?1 AND COALESCE(ZISACTIVE, 1)=1 AND ZMEMBERJID IS NOT NULL
         ORDER BY ZMEMBERJID",
    )?;
    for (chat_id, thread_native_id, partner_name) in rows {
        let group = roster
            .query_map([chat_id], |member| {
                Ok((
                    member.get::<_, String>(0)?,
                    member.get::<_, Option<String>>(1)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let resolved = if group.is_empty() {
            vec![(
                identities.resolve(&thread_native_id),
                partner_name.as_deref(),
            )]
        } else {
            group
                .iter()
                .map(|(identity, name)| (identities.resolve(identity), name.as_deref()))
                .collect()
        };
        let members = resolved
            .iter()
            .map(
                |(identity, name)| crate::relationships::ConversationMember {
                    identity,
                    display_name: *name,
                },
            )
            .collect::<Vec<_>>();
        crate::relationships::replace_members(
            crm,
            "whatsapp",
            &thread_native_id,
            partner_name.as_deref(),
            &members,
        )?;
    }
    Ok(())
}

fn removed_message_ids(connection: &Connection) -> Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT COALESCE(m.ZSTANZAID, 'pk:' || m.Z_PK)
         FROM ZWAMESSAGE m JOIN ZWACHATSESSION s ON s.Z_PK=m.ZCHATSESSION
         WHERE COALESCE(s.ZREMOVED, 0) != 0",
    )?;
    statement
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()
        .map_err(Into::into)
}

#[cfg(test)]
#[path = "whatsapp/tests.rs"]
mod tests;

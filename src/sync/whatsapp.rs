use rusqlite::Connection;
use std::collections::HashSet;

use super::whatsapp_identity::LidResolver;
use super::{SyncReport, finish_source, open_source, replace_participant, upsert_interaction};
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
    let (source, fingerprint, run_at) = open_source(
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
    let mut statement = source.connection().prepare(
        "SELECT m.Z_PK, m.ZSTANZAID, s.ZCONTACTJID, datetime(m.ZMESSAGEDATE + 978307200, 'unixepoch'),
                m.ZISFROMME, m.ZTEXT, m.ZFROMJID, m.ZTOJID, m.ZMESSAGETYPE,
                profile.ZPUSHNAME, m.ZPUSHNAME, s.ZPARTNERNAME, COUNT(*) OVER()
         FROM ZWAMESSAGE m JOIN ZWACHATSESSION s ON s.Z_PK = m.ZCHATSESSION
         LEFT JOIN ZWAPROFILEPUSHNAME profile
           ON profile.ZJID = CASE WHEN m.ZISFROMME = 1 THEN m.ZTOJID ELSE m.ZFROMJID END
         WHERE m.ZMESSAGEDATE IS NOT NULL AND COALESCE(s.ZREMOVED, 0) = 0",
    )?;
    let mut imported = HashSet::new();
    let mut rows = statement.query([])?;
    let mut processed = 0_u64;
    let mut total = 0_u64;
    progress.stage(
        "Reading WhatsApp conversations",
        stage_current,
        stage_total,
        1,
        false,
        "query",
    );
    while let Some(row) = rows.next()? {
        total = u64::try_from(row.get::<_, i64>(12)?).unwrap_or_default();
        let pk: i64 = row.get(0)?;
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
        let interaction_id = upsert_interaction(
            crm,
            "whatsapp",
            &native_id,
            row.get::<_, Option<String>>(2)?.as_deref(),
            "whatsapp",
            "message",
            &row.get::<_, String>(3)?,
            Some(if from_me { "outgoing" } else { "incoming" }),
            None,
            row.get::<_, Option<String>>(5)?.as_deref(),
            &serde_json::json!({"message_type": row.get::<_, i64>(8)?}),
            &run_at,
        )?;
        if let Some(identity) = if from_me { recipient } else { sender } {
            replace_participant(
                crm,
                &interaction_id,
                identities.resolve(&identity),
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
    progress.finish_stage(
        "Read WhatsApp conversations",
        processed,
        total,
        false,
        "messages",
    );
    progress.progress_now("Finalizing WhatsApp sync", 0, 1, false, "step");
    let deleted = finish_source(crm, "whatsapp", &run_at)?;
    progress.finish_stage("Finalized WhatsApp sync", 1, 1, false, "step");
    Ok(SyncReport {
        source: "whatsapp".into(),
        imported: imported.len(),
        deleted,
        schema_fingerprint: fingerprint,
    })
}

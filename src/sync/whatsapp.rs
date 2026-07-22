use rusqlite::Connection;
use std::collections::HashSet;

use super::{SyncReport, add_participant, finish_source, open_source, upsert_interaction};
use crate::error::{CrmError, Result};

pub fn sync(config: &crate::config::Config, crm: &Connection) -> Result<SyncReport> {
    let path = config
        .paths
        .whatsapp
        .as_ref()
        .ok_or_else(|| CrmError::InvalidConfig("WhatsApp path is not configured".into()))?;
    let (source, fingerprint, run_at) = open_source(
        crm,
        "whatsapp",
        "whatsapp",
        path,
        "ZWAMESSAGE",
        &["Z_PK", "ZMESSAGEDATE", "ZISFROMME", "ZTEXT"],
    )?;
    let mut statement = source.connection().prepare(
        "SELECT m.Z_PK, m.ZSTANZAID, s.ZCONTACTJID, datetime(m.ZMESSAGEDATE + 978307200, 'unixepoch'),
                m.ZISFROMME, m.ZTEXT, m.ZFROMJID, m.ZTOJID, m.ZMESSAGETYPE
         FROM ZWAMESSAGE m LEFT JOIN ZWACHATSESSION s ON s.Z_PK = m.ZCHATSESSION
         WHERE m.ZMESSAGEDATE IS NOT NULL",
    )?;
    let mut imported = HashSet::new();
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let pk: i64 = row.get(0)?;
        let native_id = row
            .get::<_, Option<String>>(1)?
            .unwrap_or_else(|| format!("pk:{pk}"));
        imported.insert(native_id.clone());
        let from_me = row.get::<_, i64>(4)? != 0;
        let sender: Option<String> = row.get(6)?;
        let recipient: Option<String> = row.get(7)?;
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
            add_participant(
                crm,
                &interaction_id,
                &identity,
                if from_me { "recipient" } else { "sender" },
            )?;
        }
    }
    let deleted = finish_source(crm, "whatsapp", &run_at)?;
    Ok(SyncReport {
        source: "whatsapp".into(),
        imported: imported.len(),
        deleted,
        schema_fingerprint: fingerprint,
    })
}

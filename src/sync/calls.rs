use rusqlite::Connection;
use std::collections::HashSet;

use super::{SyncReport, add_participant, finish_source, open_source, upsert_interaction};
use crate::error::{CrmError, Result};

pub fn sync(config: &crate::config::Config, crm: &Connection) -> Result<Vec<SyncReport>> {
    Ok(vec![sync_apple(config, crm)?, sync_whatsapp(config, crm)?])
}

fn sync_apple(config: &crate::config::Config, crm: &Connection) -> Result<SyncReport> {
    let path = config
        .paths
        .apple_calls
        .as_ref()
        .ok_or_else(|| CrmError::InvalidConfig("Apple calls path is not configured".into()))?;
    let (source, fingerprint, run_at) = open_source(
        crm,
        "apple_calls",
        "calls",
        path,
        "ZCALLRECORD",
        &["Z_PK", "ZDATE", "ZDURATION", "ZADDRESS", "ZNAME"],
    )?;
    let mut statement = source.connection().prepare(
        "SELECT Z_PK, COALESCE(ZUNIQUE_ID, 'pk:' || Z_PK), datetime(ZDATE + 978307200, 'unixepoch'),
                ZDURATION, ZADDRESS, ZORIGINATED, ZANSWERED, ZCALLTYPE, ZSERVICE_PROVIDER,
                ZNAME
         FROM ZCALLRECORD WHERE ZDATE IS NOT NULL",
    )?;
    let mut imported = HashSet::new();
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let native_id: String = row.get(1)?;
        imported.insert(native_id.clone());
        let originated = row.get::<_, i64>(5)? != 0;
        let answered = row.get::<_, i64>(6)? != 0;
        let metadata = serde_json::json!({"duration_seconds": row.get::<_, f64>(3)?, "answered": answered, "call_type": row.get::<_, i64>(7)?, "provider": row.get::<_, Option<String>>(8)?});
        let interaction_id = upsert_interaction(
            crm,
            "apple_calls",
            &native_id,
            None,
            "apple_call",
            "call",
            &row.get::<_, String>(2)?,
            Some(if originated { "outgoing" } else { "incoming" }),
            None,
            None,
            &metadata,
            &run_at,
        )?;
        if let Some(identity) = row.get::<_, Option<String>>(4)? {
            add_participant(
                crm,
                &interaction_id,
                &identity,
                row.get::<_, Option<String>>(9)?.as_deref(),
                if originated { "recipient" } else { "caller" },
            )?;
        }
    }
    let deleted = finish_source(crm, "apple_calls", &run_at)?;
    Ok(SyncReport {
        source: "apple_calls".into(),
        imported: imported.len(),
        deleted,
        schema_fingerprint: fingerprint,
    })
}

fn sync_whatsapp(config: &crate::config::Config, crm: &Connection) -> Result<SyncReport> {
    let path =
        config.paths.whatsapp_calls.as_ref().ok_or_else(|| {
            CrmError::InvalidConfig("WhatsApp calls path is not configured".into())
        })?;
    let (source, fingerprint, run_at) = open_source(
        crm,
        "whatsapp_calls",
        "calls",
        path,
        "ZWACDCALLEVENT",
        &["Z_PK", "ZDATE", "ZDURATION", "ZCALLIDSTRING"],
    )?;
    let mut statement = source.connection().prepare(
        "SELECT e.Z_PK, COALESCE(e.ZCALLIDSTRING, 'pk:' || e.Z_PK), datetime(e.ZDATE + 978307200, 'unixepoch'),
                e.ZDURATION, e.ZOUTCOME, p.ZJIDSTRING
         FROM ZWACDCALLEVENT e LEFT JOIN ZWACDCALLEVENTPARTICIPANT p ON p.Z1PARTICIPANTS = e.Z_PK
         WHERE e.ZDATE IS NOT NULL",
    )?;
    let mut imported = HashSet::new();
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let native_id: String = row.get(1)?;
        imported.insert(native_id.clone());
        let metadata = serde_json::json!({"duration_seconds": row.get::<_, f64>(3)?, "outcome": row.get::<_, i64>(4)?});
        let interaction_id = upsert_interaction(
            crm,
            "whatsapp_calls",
            &native_id,
            None,
            "whatsapp_call",
            "call",
            &row.get::<_, String>(2)?,
            None,
            None,
            None,
            &metadata,
            &run_at,
        )?;
        if let Some(identity) = row.get::<_, Option<String>>(5)? {
            add_participant(crm, &interaction_id, &identity, None, "participant")?;
        }
    }
    let deleted = finish_source(crm, "whatsapp_calls", &run_at)?;
    Ok(SyncReport {
        source: "whatsapp_calls".into(),
        imported: imported.len(),
        deleted,
        schema_fingerprint: fingerprint,
    })
}

use rusqlite::Connection;
use std::collections::HashSet;

use super::incremental::{finish_incremental_source, incremental_floor, open_incremental_source};
use super::whatsapp_identity::LidResolver;
use super::{SyncReport, add_participant, upsert_interaction};
use crate::error::{CrmError, Result};
use crate::progress::ProgressTracker;

pub(super) fn sync_apple(
    config: &crate::config::Config,
    crm: &Connection,
    progress: &mut ProgressTracker,
    stage_current: u64,
    stage_total: u64,
) -> Result<SyncReport> {
    let path = config
        .paths
        .apple_calls
        .as_ref()
        .ok_or_else(|| CrmError::InvalidConfig("Apple calls path is not configured".into()))?;
    let source = open_incremental_source(
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
                ZNAME, COUNT(*) OVER()
         FROM ZCALLRECORD WHERE ZDATE IS NOT NULL AND Z_PK > ?1",
    )?;
    let mut imported = HashSet::new();
    let mut processed = 0_u64;
    let mut total = 0_u64;
    let mut cursor = if source.audit { 0 } else { source.cursor };
    progress.stage(
        "Reading Apple call history",
        stage_current,
        stage_total,
        1,
        false,
        "query",
    );
    let mut rows = statement.query([incremental_floor(&source)])?;
    while let Some(row) = rows.next()? {
        total = u64::try_from(row.get::<_, i64>(10)?).unwrap_or_default();
        cursor = cursor.max(row.get(0)?);
        let native_id: String = row.get(1)?;
        imported.insert(native_id.clone());
        let originated = row.get::<_, i64>(5)? != 0;
        let answered = row.get::<_, i64>(6)? != 0;
        let identity: Option<String> = row.get(4)?;
        let name: Option<String> = row.get(9)?;
        let occurred_at: String = row.get(2)?;
        progress.focus([format!(
            "{} · Apple call · {} · {}",
            name.as_deref()
                .or(identity.as_deref())
                .unwrap_or("Unknown caller"),
            if originated { "outgoing" } else { "incoming" },
            occurred_at.get(..10).unwrap_or(&occurred_at),
        )]);
        let metadata = serde_json::json!({"duration_seconds": row.get::<_, f64>(3)?, "answered": answered, "call_type": row.get::<_, i64>(7)?, "provider": row.get::<_, Option<String>>(8)?});
        let interaction_id = upsert_interaction(
            crm,
            "apple_calls",
            &native_id,
            None,
            "apple_call",
            "call",
            &occurred_at,
            Some(if originated { "outgoing" } else { "incoming" }),
            None,
            None,
            &metadata,
            &source.run_at,
        )?;
        if let Some(identity) = identity {
            add_participant(
                crm,
                &interaction_id,
                &identity,
                name.as_deref(),
                if originated { "recipient" } else { "caller" },
            )?;
        }
        processed += 1;
        progress.progress(
            "Reading Apple call history",
            processed,
            total,
            false,
            "calls",
        );
    }
    progress.finish_stage("Read Apple call history", processed, total, false, "calls");
    progress.progress_now("Finalizing Apple call sync", 0, 1, false, "step");
    let deleted = finish_incremental_source(crm, "apple_calls", &source, cursor)?;
    progress.finish_stage("Finalized Apple call sync", 1, 1, false, "step");
    Ok(SyncReport {
        source: "apple_calls".into(),
        imported: imported.len(),
        deleted,
        schema_fingerprint: source.fingerprint.clone(),
        changed: !imported.is_empty() || deleted > 0,
    })
}

pub(super) fn sync_whatsapp(
    config: &crate::config::Config,
    crm: &Connection,
    progress: &mut ProgressTracker,
    stage_current: u64,
    stage_total: u64,
) -> Result<SyncReport> {
    let chat_path = config
        .paths
        .whatsapp
        .as_ref()
        .ok_or_else(|| CrmError::InvalidConfig("WhatsApp path is not configured".into()))?;
    let identities = LidResolver::load(chat_path)?;
    let path =
        config.paths.whatsapp_calls.as_ref().ok_or_else(|| {
            CrmError::InvalidConfig("WhatsApp calls path is not configured".into())
        })?;
    let source = open_incremental_source(
        crm,
        "whatsapp_calls",
        "calls",
        path,
        "ZWACDCALLEVENT",
        &["Z_PK", "ZDATE", "ZDURATION", "ZCALLIDSTRING"],
    )?;
    let mut statement = source.connection().prepare(
        "SELECT e.Z_PK, COALESCE(e.ZCALLIDSTRING, 'pk:' || e.Z_PK), datetime(e.ZDATE + 978307200, 'unixepoch'),
                e.ZDURATION, e.ZOUTCOME, p.ZJIDSTRING, COUNT(*) OVER()
         FROM ZWACDCALLEVENT e LEFT JOIN ZWACDCALLEVENTPARTICIPANT p ON p.Z1PARTICIPANTS = e.Z_PK
         WHERE e.ZDATE IS NOT NULL AND e.Z_PK > ?1",
    )?;
    let mut imported = HashSet::new();
    let mut cleared_participants = HashSet::new();
    let mut processed = 0_u64;
    let mut total = 0_u64;
    let mut cursor = if source.audit { 0 } else { source.cursor };
    progress.stage(
        "Reading WhatsApp call history",
        stage_current,
        stage_total,
        1,
        false,
        "query",
    );
    let mut rows = statement.query([incremental_floor(&source)])?;
    while let Some(row) = rows.next()? {
        total = u64::try_from(row.get::<_, i64>(6)?).unwrap_or_default();
        cursor = cursor.max(row.get(0)?);
        let native_id: String = row.get(1)?;
        imported.insert(native_id.clone());
        let identity: Option<String> = row.get(5)?;
        let occurred_at: String = row.get(2)?;
        progress.focus([format!(
            "{} · WhatsApp call · {}",
            identity.as_deref().unwrap_or("Unknown participant"),
            occurred_at.get(..10).unwrap_or(&occurred_at),
        )]);
        let metadata = serde_json::json!({"duration_seconds": row.get::<_, f64>(3)?, "outcome": row.get::<_, i64>(4)?});
        let interaction_id = upsert_interaction(
            crm,
            "whatsapp_calls",
            &native_id,
            None,
            "whatsapp_call",
            "call",
            &occurred_at,
            None,
            None,
            None,
            &metadata,
            &source.run_at,
        )?;
        if cleared_participants.insert(interaction_id.clone()) {
            crm.execute(
                "DELETE FROM interaction_participants WHERE interaction_id=?1 AND role='participant'",
                [&interaction_id],
            )?;
        }
        if let Some(identity) = identity {
            add_participant(
                crm,
                &interaction_id,
                identities.resolve(&identity),
                None,
                "participant",
            )?;
        }
        processed += 1;
        progress.progress(
            "Reading WhatsApp call history",
            processed,
            total,
            false,
            "call records",
        );
    }
    progress.finish_stage(
        "Read WhatsApp call history",
        processed,
        total,
        false,
        "call records",
    );
    progress.progress_now("Finalizing WhatsApp call sync", 0, 1, false, "step");
    let deleted = finish_incremental_source(crm, "whatsapp_calls", &source, cursor)?;
    progress.finish_stage("Finalized WhatsApp call sync", 1, 1, false, "step");
    Ok(SyncReport {
        source: "whatsapp_calls".into(),
        imported: imported.len(),
        deleted,
        schema_fingerprint: source.fingerprint.clone(),
        changed: !imported.is_empty() || deleted > 0,
    })
}

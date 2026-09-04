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
        let interaction_id = upsert_interaction(
            crm,
            "whatsapp",
            &native_id,
            row.get::<_, Option<String>>(2)?.as_deref(),
            "whatsapp",
            "message",
            &occurred_at,
            Some(if from_me { "outgoing" } else { "incoming" }),
            None,
            row.get::<_, Option<String>>(5)?.as_deref(),
            &serde_json::json!({"message_type": row.get::<_, i64>(8)?}),
            &source.run_at,
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
mod tests {
    use super::*;
    use crate::config::{Config, SourcePaths};

    fn fixture(count: i64) -> (tempfile::TempDir, std::path::PathBuf, Connection, Config) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ChatStorage.sqlite");
        let source = Connection::open(&path).unwrap();
        source
            .execute_batch(
                "CREATE TABLE ZWACHATSESSION (
                    Z_PK INTEGER PRIMARY KEY, ZCONTACTJID TEXT, ZPARTNERNAME TEXT,
                    ZREMOVED INTEGER
                 );
                 CREATE TABLE ZWAPROFILEPUSHNAME (ZJID TEXT, ZPUSHNAME TEXT);
                 CREATE TABLE ZWAMESSAGE (
                    Z_PK INTEGER PRIMARY KEY, ZSTANZAID TEXT, ZMESSAGEDATE REAL,
                    ZISFROMME INTEGER, ZTEXT TEXT, ZFROMJID TEXT, ZTOJID TEXT,
                    ZMESSAGETYPE INTEGER, ZPUSHNAME TEXT, ZCHATSESSION INTEGER
                 );
                 INSERT INTO ZWACHATSESSION VALUES
                    (1, '15550100@s.whatsapp.net', 'Alex', 0);",
            )
            .unwrap();
        source
            .execute(
                "WITH RECURSIVE sequence(value) AS (
                    SELECT 1 UNION ALL SELECT value + 1 FROM sequence WHERE value < ?1
                 )
                 INSERT INTO ZWAMESSAGE
                 SELECT value, printf('message-%d', value), value, 0, 'hello',
                        '15550100@s.whatsapp.net', '19990100@s.whatsapp.net',
                        0, 'Alex', 1 FROM sequence",
                [count],
            )
            .unwrap();
        let crm = crate::db::open(&directory.path().join("crm.sqlite3")).unwrap();
        let mut config = Config::new("Me".into(), Vec::new()).unwrap();
        config.paths = SourcePaths {
            whatsapp: Some(path.clone()),
            ..SourcePaths::default()
        };
        (directory, path, crm, config)
    }

    #[test]
    fn incremental_sync_reads_only_the_cursor_overlap_and_new_rows() {
        let (_directory, path, crm, config) = fixture(1_005);
        let mut progress = ProgressTracker::disabled();
        assert_eq!(
            sync(&config, &crm, &mut progress, 1, 1).unwrap().imported,
            1_005
        );
        let source = Connection::open(path).unwrap();
        crm.execute(
            "UPDATE interactions SET analysis_state='complete' WHERE native_id='message-1005'",
            [],
        )
        .unwrap();
        source
            .execute("UPDATE ZWAMESSAGE SET ZTEXT='edited' WHERE Z_PK=1005", [])
            .unwrap();
        source
            .execute(
                "INSERT INTO ZWAMESSAGE VALUES
                 (1006, 'message-1006', 1006, 0, 'new', '15550100@s.whatsapp.net',
                  '19990100@s.whatsapp.net', 0, 'Alex', 1)",
                [],
            )
            .unwrap();

        let report = sync(&config, &crm, &mut progress, 1, 1).unwrap();

        assert_eq!(report.imported, 1_001);
        let cursor: String = crm
            .query_row(
                "SELECT cursor FROM sources WHERE id='whatsapp'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cursor, "1006");
        let state: String = crm
            .query_row(
                "SELECT analysis_state FROM interactions WHERE native_id='message-1005'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "pending");
    }

    #[test]
    fn daily_full_audit_tombstones_hard_deletions() {
        let (_directory, path, crm, config) = fixture(2);
        let mut progress = ProgressTracker::disabled();
        sync(&config, &crm, &mut progress, 1, 1).unwrap();
        let source = Connection::open(path).unwrap();
        source
            .execute("DELETE FROM ZWAMESSAGE WHERE Z_PK=2", [])
            .unwrap();
        crm.execute(
            "UPDATE sources SET last_reconcile_at='2000-01-01' WHERE id='whatsapp'",
            [],
        )
        .unwrap();

        let report = sync(&config, &crm, &mut progress, 1, 1).unwrap();

        assert_eq!(report.deleted, 1);
        let deleted: bool = crm
            .query_row(
                "SELECT deleted_at IS NOT NULL FROM interactions WHERE native_id='message-2'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(deleted);
        let cursor: String = crm
            .query_row(
                "SELECT cursor FROM sources WHERE id='whatsapp'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cursor, "1");
    }
}

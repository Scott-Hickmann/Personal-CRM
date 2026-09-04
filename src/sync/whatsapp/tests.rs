use super::*;
use crate::config::{Config, SourcePaths};
use rusqlite::params;

fn fixture(count: i64) -> (tempfile::TempDir, std::path::PathBuf, Connection, Config) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ChatStorage.sqlite");
    let source = Connection::open(&path).unwrap();
    source
        .execute_batch(
            "CREATE TABLE ZWACHATSESSION (
            Z_PK INTEGER PRIMARY KEY, ZCONTACTJID TEXT, ZPARTNERNAME TEXT, ZREMOVED INTEGER
         );
         CREATE TABLE ZWAGROUPMEMBER (
            ZCHATSESSION INTEGER, ZISACTIVE INTEGER, ZMEMBERJID TEXT,
            ZCONTACTNAME TEXT, ZFIRSTNAME TEXT
         );
         CREATE TABLE ZWAPROFILEPUSHNAME (ZJID TEXT, ZPUSHNAME TEXT);
         CREATE TABLE ZWAMESSAGE (
            Z_PK INTEGER PRIMARY KEY, ZSTANZAID TEXT, ZMESSAGEDATE REAL,
            ZISFROMME INTEGER, ZTEXT TEXT, ZFROMJID TEXT, ZTOJID TEXT,
            ZMESSAGETYPE INTEGER, ZPUSHNAME TEXT, ZCHATSESSION INTEGER
         );
         INSERT INTO ZWACHATSESSION VALUES (1, '15550100@s.whatsapp.net', 'Alex', 0);",
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
    let body: String = crm
        .query_row(
            "SELECT body FROM interactions WHERE native_id='message-1005'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(body, "edited");
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

#[test]
fn group_roster_includes_silent_members() {
    let (_directory, path, crm, _config) = fixture(1);
    let source = Connection::open(&path).unwrap();
    crm.execute(
        "INSERT INTO sources(id, kind) VALUES ('whatsapp', 'whatsapp')",
        [],
    )
    .unwrap();
    source
        .execute("UPDATE ZWACHATSESSION SET ZCONTACTJID='group@g.us'", [])
        .unwrap();
    source
        .execute_batch(
            "INSERT INTO ZWAGROUPMEMBER VALUES
           (1, 1, '15550100@s.whatsapp.net', 'Alex', NULL),
           (1, 1, '15550200@s.whatsapp.net', 'Blair', NULL);",
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
             VALUES (?1, ?1, 'whatsapp', ?2, ?2)",
            params![id, identity],
        )
        .unwrap();
    }
    let resolver = LidResolver::load(&path).unwrap();
    refresh_memberships(&source, &crm, &resolver).unwrap();
    let count: i64 = crm
        .query_row(
            "SELECT COUNT(DISTINCT person_id) FROM conversation_memberships
         WHERE source_id='whatsapp' AND thread_native_id='group@g.us'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
}

use super::*;
use crate::config::SourcePaths;
use crate::db;

#[test]
fn review_refresh_resolves_candidate_from_deleted_whatsapp_chat() {
    let directory = tempfile::tempdir().unwrap();
    let whatsapp_path = directory.path().join("ChatStorage.sqlite");
    let source = rusqlite::Connection::open(&whatsapp_path).unwrap();
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
                 (1, '15550100@s.whatsapp.net', 'Alex', 0);
             INSERT INTO ZWAPROFILEPUSHNAME VALUES
                 ('15550100@s.whatsapp.net', 'Alex');
             INSERT INTO ZWAMESSAGE VALUES
                 (1, 'message-1', 1, 0, 'hello', '15550100@s.whatsapp.net',
                  '19990100@s.whatsapp.net', 0, 'Alex', 1);",
        )
        .unwrap();
    drop(source);
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    let mut config = Config::new("Me".into(), Vec::new()).unwrap();
    config.paths = SourcePaths {
        whatsapp: Some(whatsapp_path.clone()),
        ..SourcePaths::default()
    };

    refresh_whatsapp_reviews(&config, &connection).unwrap();
    assert_eq!(review::pending(&connection).unwrap().len(), 1);

    let source = rusqlite::Connection::open(&whatsapp_path).unwrap();
    source
        .execute("UPDATE ZWACHATSESSION SET ZREMOVED=1", [])
        .unwrap();
    drop(source);

    refresh_whatsapp_reviews(&config, &connection).unwrap();

    assert!(review::pending(&connection).unwrap().is_empty());
    let deleted: bool = connection
        .query_row(
            "SELECT deleted_at IS NOT NULL FROM interactions
             WHERE source_id='whatsapp' AND native_id='message-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(deleted);
}

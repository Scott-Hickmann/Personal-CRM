use super::*;

fn database() -> (tempfile::TempDir, Connection) {
    let directory = tempfile::tempdir().unwrap();
    let connection = crate::db::open(&directory.path().join("crm.sqlite3")).unwrap();
    (directory, connection)
}

#[test]
fn requests_coalesce_into_generations_instead_of_rows() {
    let (_directory, connection) = database();
    assert_eq!(
        request(&connection, WorkKind::Gmail, "first", Duration::zero()).unwrap(),
        1
    );
    assert_eq!(
        request(&connection, WorkKind::Gmail, "second", Duration::zero()).unwrap(),
        2
    );
    let state: (i64, i64, String) = connection
        .query_row(
            "SELECT COUNT(*), requested_generation, state
             FROM source_sync_state WHERE kind='gmail'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(state, (1, 2, "pending".into()));
}

#[test]
fn interrupted_source_work_keeps_its_stage() {
    let (_directory, connection) = database();
    request(&connection, WorkKind::Whatsapp, "test", Duration::zero()).unwrap();
    connection
        .execute(
            "UPDATE source_sync_state SET state='running', step='relationships'
             WHERE kind='whatsapp'",
            [],
        )
        .unwrap();

    assert_eq!(recover_interrupted(&connection).unwrap(), 1);

    let state: (String, String) = connection
        .query_row(
            "SELECT state, step FROM source_sync_state WHERE kind='whatsapp'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, ("pending".into(), "relationships".into()));
}

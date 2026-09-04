use super::*;
use crate::db;

#[test]
fn seeds_only_active_icloud_email_scopes() {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    connection
        .execute_batch(
            "INSERT INTO sources(id, kind) VALUES ('gmail:test', 'gmail');
             INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
             VALUES ('active', 'Active', 'apple-1', 'active'),
                    ('ignored', 'List', 'apple-list', 'active'),
                    ('retired', 'Retired', 'apple-2', 'retired');
             INSERT INTO identities(id, person_id, kind, value, normalized_value, active)
             VALUES ('a', 'active', 'email', 'active@example.com', 'active@example.com', 1),
                    ('i', 'ignored', 'email', 'group@lists.stanford.edu',
                     'group@lists.stanford.edu', 1),
                    ('r', 'retired', 'email', 'retired@example.com', 'retired@example.com', 0);",
        )
        .unwrap();

    seed(
        &connection,
        "gmail:test",
        &HashSet::new(),
        &["lists.stanford.edu".into()],
    )
    .unwrap();

    let scopes: Vec<String> = connection
        .prepare("SELECT scope_key FROM gmail_sync_scopes ORDER BY scope_key")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(scopes.len(), 2);
    assert!(
        scopes
            .iter()
            .any(|scope| scope.contains("active@example.com"))
    );
    assert!(scopes.iter().any(|scope| scope.contains("discovery")));
    assert!(
        !scopes
            .iter()
            .any(|scope| scope.contains("lists.stanford.edu"))
    );
}

#[test]
fn known_scope_requeues_a_message_skipped_during_discovery() {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    connection
        .execute(
            "INSERT INTO sources(id, kind) VALUES ('gmail:test', 'gmail')",
            [],
        )
        .unwrap();
    enqueue_from_scope(&connection, "gmail:test", "message", "discovery").unwrap();
    mark(
        &connection,
        "gmail:test",
        "message",
        "skipped",
        Some("not_personal"),
    )
    .unwrap();

    enqueue_from_scope(&connection, "gmail:test", "message", "contact").unwrap();

    let status: String = connection
        .query_row(
            "SELECT status FROM gmail_message_state WHERE message_id='message'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "queued");
    let (queued, _) = queued(&connection, "gmail:test").unwrap();
    assert!(queued[0].known_scope);
}

#[test]
fn resetting_all_requeues_every_gmail_source() {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    connection
        .execute_batch(
            "INSERT INTO sources(id, kind) VALUES
             ('gmail:first', 'gmail'), ('gmail:second', 'gmail');
             INSERT INTO gmail_sync_scopes(
                 source_id, scope_key, kind, query, completed_at, messages_found
             ) VALUES
             ('gmail:first', 'one', 'discovery', 'in:sent', CURRENT_TIMESTAMP, 2),
             ('gmail:second', 'two', 'discovery', 'in:sent', CURRENT_TIMESTAMP, 3);
             INSERT INTO gmail_message_state(source_id, message_id, status, reason) VALUES
             ('gmail:first', 'one', 'accepted', NULL),
             ('gmail:second', 'two', 'skipped', 'incoming_unknown');",
        )
        .unwrap();

    reset_all(&connection).unwrap();

    let queued: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM gmail_message_state WHERE status='queued' AND reason IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let pending_scopes: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM gmail_sync_scopes
             WHERE completed_at IS NULL AND page_token IS NULL AND messages_found=0",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(queued, 2);
    assert_eq!(pending_scopes, 2);
}

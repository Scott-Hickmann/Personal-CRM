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
                    ('retired', 'Retired', 'apple-2', 'retired');
             INSERT INTO identities(id, person_id, kind, value, normalized_value, active)
             VALUES ('a', 'active', 'email', 'active@example.com', 'active@example.com', 1),
                    ('r', 'retired', 'email', 'retired@example.com', 'retired@example.com', 0);",
        )
        .unwrap();

    seed(&connection, "gmail:test", &HashSet::new()).unwrap();

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

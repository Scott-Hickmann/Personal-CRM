use super::*;
use crate::db;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn fixture() -> (tempfile::TempDir, Connection) {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    connection
        .execute(
            "INSERT INTO sources(id, kind) VALUES ('source', 'test')",
            [],
        )
        .unwrap();
    for (id, name, email) in [
        ("a", "Alex", "alex@example.com"),
        ("b", "Blair", "blair@example.com"),
    ] {
        connection
            .execute(
                "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
                 VALUES (?1, ?2, ?1, 'active')",
                params![id, name],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO identities(id, person_id, kind, value, normalized_value, active)
                 VALUES (?1, ?1, 'email', ?2, ?2, 1)",
                params![id, email],
            )
            .unwrap();
    }
    connection
        .execute(
            "INSERT INTO interactions(
                 id, source_id, native_id, thread_native_id, channel, kind, occurred_at
             ) VALUES ('message', 'source', 'message', 'thread', 'gmail', 'email', '2026-01-01')",
            [],
        )
        .unwrap();
    (directory, connection)
}

#[test]
fn shared_membership_creates_one_canonical_relationship() {
    let (_directory, connection) = fixture();
    observe_member(
        &connection,
        "source",
        "thread",
        ConversationMember {
            identity: "alex@example.com",
            display_name: Some("Alex"),
        },
    )
    .unwrap();
    observe_member(
        &connection,
        "source",
        "thread",
        ConversationMember {
            identity: "blair@example.com",
            display_name: Some("Blair"),
        },
    )
    .unwrap();

    assert_eq!(reconcile_source(&connection, "source").unwrap(), 1);
    let row: (String, String, String, i64) = connection
        .query_row(
            "SELECT source_person_id, target_person_id, relationship_type,
                    shared_context_count FROM relationships",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(row, ("a".into(), "b".into(), "unclear".into(), 1));

    connection
        .execute(
            "UPDATE relationships SET relationship_type='friend',
                 classification_confidence=0.9, classification_state='complete'",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE interactions SET occurred_at='2026-01-02' WHERE id='message'",
            [],
        )
        .unwrap();
    reconcile_source(&connection, "source").unwrap();
    let reset: (String, Option<f64>, String) = connection
        .query_row(
            "SELECT relationship_type, classification_confidence, classification_state
                 FROM relationships",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(reset, ("unclear".into(), None, "pending".into()));
}

#[test]
fn removing_the_last_context_removes_the_relationship() {
    let (_directory, connection) = fixture();
    for identity in ["alex@example.com", "blair@example.com"] {
        observe_member(
            &connection,
            "source",
            "thread",
            ConversationMember {
                identity,
                display_name: None,
            },
        )
        .unwrap();
    }
    reconcile_source(&connection, "source").unwrap();
    connection
        .execute("UPDATE interactions SET deleted_at=CURRENT_TIMESTAMP", [])
        .unwrap();

    reconcile_source(&connection, "source").unwrap();

    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM relationships", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn reconciliation_waits_for_an_active_writer_before_reading() {
    let (directory, connection) = fixture();
    let active = db::immediate_transaction(&connection).unwrap();
    active
        .execute("UPDATE sources SET status='syncing' WHERE id='source'", [])
        .unwrap();

    let path = directory.path().join("crm.sqlite3");
    let (started_tx, started_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let waiting = db::open(&path).unwrap();
        started_tx.send(()).unwrap();
        reconcile_source(&waiting, "source")
    });
    started_rx.recv().unwrap();
    thread::sleep(Duration::from_millis(100));
    active.commit().unwrap();

    worker.join().unwrap().unwrap();
}

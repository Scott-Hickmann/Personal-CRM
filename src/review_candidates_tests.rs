use rusqlite::{Connection, params};

use crate::{db, review, review_candidates};

fn interaction(connection: &Connection, id: &str, channel: &str, identity: &str, name: &str) {
    connection
        .execute(
            "INSERT INTO interactions(
                 id, source_id, native_id, channel, kind, occurred_at, last_seen_at
             ) VALUES (?1, 'whatsapp', ?1, ?2, 'message',
                       '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            params![id, channel],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO interaction_participants(
                 interaction_id, identity_value, display_name, role
             ) VALUES (?1, ?2, ?3, 'sender')",
            params![id, identity, name],
        )
        .unwrap();
}

fn database() -> (tempfile::TempDir, Connection) {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    connection
        .execute(
            "INSERT INTO sources(id, kind) VALUES ('whatsapp', 'whatsapp')",
            [],
        )
        .unwrap();
    (directory, connection)
}

#[test]
fn candidate_includes_source_and_name() {
    let (_directory, connection) = database();
    interaction(
        &connection,
        "message",
        "whatsapp",
        "15550100@s.whatsapp.net",
        "Alex",
    );

    review_candidates::enqueue(&connection).unwrap();

    let item = review::pending(&connection).unwrap().pop().unwrap();
    assert_eq!(item.source.as_deref(), Some("WhatsApp"));
    assert_eq!(item.details["name"], "Alex");
}

#[test]
fn equivalent_whatsapp_forms_become_one_candidate() {
    let (_directory, connection) = database();
    interaction(
        &connection,
        "message-1",
        "whatsapp",
        "+33651427844",
        "Christiane",
    );
    interaction(
        &connection,
        "message-2",
        "whatsapp_call",
        "33651427844@s.whatsapp.net",
        "HICKMANN Christiane",
    );

    assert_eq!(review_candidates::enqueue(&connection).unwrap(), 1);

    let items = review::pending(&connection).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].subject_key, "+33651427844");
    assert_eq!(items[0].details["interaction_count"], 2);
}

#[test]
fn canonical_phone_match_is_not_a_candidate() {
    let (_directory, connection) = database();
    let normalized = crate::repository::normalize_identity("phone", "+33 06 51 42 78 44");
    connection
        .execute_batch(
            "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
             VALUES ('person', 'Christiane', 'apple-1', 'active');",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO identities(id, person_id, kind, value, normalized_value, active)
             VALUES ('phone', 'person', 'phone', '+33 06 51 42 78 44', ?1, 1)",
            [normalized],
        )
        .unwrap();
    interaction(
        &connection,
        "message",
        "whatsapp",
        "33651427844@s.whatsapp.net",
        "Christiane",
    );

    assert_eq!(review_candidates::enqueue(&connection).unwrap(), 0);
    assert!(review::pending(&connection).unwrap().is_empty());
}

#[test]
fn group_existing_contact_and_sms_are_ineligible() {
    let (_directory, connection) = database();
    connection
        .execute_batch(
            "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
             VALUES ('person', 'Alex', 'apple-1', 'active');
             INSERT INTO identities(id, person_id, kind, value, normalized_value, active)
             VALUES ('phone', 'person', 'phone', '+1 555 0100', '15550100', 1);",
        )
        .unwrap();
    interaction(
        &connection,
        "group",
        "whatsapp",
        "120363000000@g.us",
        "Family",
    );
    interaction(
        &connection,
        "person",
        "whatsapp",
        "15550100@s.whatsapp.net",
        "Alex",
    );
    interaction(&connection, "sms", "SMS", "+16660100", "Unknown");
    review::enqueue(
        &connection,
        "contact_candidate",
        "+16660100",
        "Create an iCloud contact for Unknown?",
        serde_json::json!({"identity": "+16660100", "channels": "SMS"}),
    )
    .unwrap();

    assert_eq!(review_candidates::enqueue(&connection).unwrap(), 0);
    assert!(review::pending(&connection).unwrap().is_empty());
}

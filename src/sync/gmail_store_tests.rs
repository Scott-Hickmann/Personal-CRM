use super::*;
use crate::db;
use crate::gmail::MessagePart;
use crate::gmail::api::{Header, MessageBody};
use base64::Engine as _;

fn message(id: &str, thread_id: &str, body: &str) -> GmailMessage {
    GmailMessage {
        id: id.into(),
        thread_id: thread_id.into(),
        label_ids: Vec::new(),
        internal_date: "0".into(),
        payload: MessagePart {
            mime_type: "text/plain".into(),
            filename: String::new(),
            headers: vec![Header {
                name: "From".into(),
                value: "Me <me@example.com>".into(),
            }],
            body: MessageBody {
                size: body.len() as i64,
                data: Some(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(body)),
                attachment_id: None,
            },
            parts: Vec::new(),
        },
    }
}

#[test]
fn shared_thread_members_create_a_relationship_without_analysis() {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    connection
        .execute(
            "INSERT INTO sources(id, kind) VALUES ('gmail:test', 'gmail')",
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
                "INSERT INTO identities(id, person_id, kind, value, normalized_value)
             VALUES (?1, ?1, 'email', ?2, ?2)",
                params![id, email],
            )
            .unwrap();
    }
    let participants = vec![
        Mailbox {
            email: "alex@example.com".into(),
            name: Some("Alex".into()),
        },
        Mailbox {
            email: "blair@example.com".into(),
            name: Some("Blair".into()),
        },
    ];

    persist_message(
        &connection,
        "gmail:test",
        &message("message", "thread", "Dinner next week?"),
        true,
        false,
        &participants,
    )
    .unwrap();
    crate::relationships::reconcile_source(&connection, "gmail:test").unwrap();

    let relationship: (String, String) = connection
        .query_row(
            "SELECT source_person_id, target_person_id FROM relationships",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(relationship, ("a".into(), "b".into()));
}

#[test]
fn prunes_legacy_noise_but_keeps_linked_people_and_qualified_candidates() {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    connection
        .execute_batch(
            "INSERT INTO sources(id, kind) VALUES ('gmail:test', 'gmail');
             INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
             VALUES ('person', 'Alex', 'apple-1', 'active');
             INSERT INTO interactions(
                 id, source_id, native_id, channel, kind, occurred_at, metadata_json
             ) VALUES
               ('linked', 'gmail:test', 'linked', 'gmail', 'email', '2026-01-01', '{}'),
               ('unknown', 'gmail:test', 'unknown', 'gmail', 'email', '2026-01-01', '{}'),
               ('candidate', 'gmail:test', 'candidate', 'gmail', 'email', '2026-01-01',
                '{\"candidate_eligible\":true}'),
               ('bulk', 'gmail:test', 'bulk', 'gmail', 'email', '2026-01-01',
                '{\"classification\":\"automated\"}');
             INSERT INTO interaction_participants(
                 interaction_id, person_id, identity_value, role
             ) VALUES
               ('linked', 'person', 'alex@example.com', 'sender'),
               ('unknown', NULL, 'unknown@example.com', 'sender'),
               ('candidate', NULL, 'candidate@example.com', 'recipient'),
               ('bulk', 'person', 'alex@example.com', 'sender');",
        )
        .unwrap();

    assert_eq!(prune_legacy_noise(&connection, "gmail:test").unwrap(), 2);

    let active: Vec<String> = connection
        .prepare("SELECT id FROM interactions WHERE deleted_at IS NULL ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(active, ["candidate", "linked"]);
}

#[test]
fn prunes_existing_interactions_from_ignored_domains() {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    connection
        .execute_batch(
            "INSERT INTO sources(id, kind) VALUES ('gmail:test', 'gmail');
             INSERT INTO interactions(
                 id, source_id, native_id, channel, kind, occurred_at, metadata_json
             ) VALUES
               ('ignored', 'gmail:test', 'ignored', 'gmail', 'email', '2026-01-01', '{}'),
               ('subdomain', 'gmail:test', 'subdomain', 'gmail', 'email', '2026-01-01', '{}'),
               ('allowed', 'gmail:test', 'allowed', 'gmail', 'email', '2026-01-01', '{}');
             INSERT INTO interaction_participants(
                 interaction_id, identity_value, role
             ) VALUES
               ('ignored', 'group@lists.stanford.edu', 'recipient'),
               ('subdomain', 'group@dept.lists.stanford.edu', 'recipient'),
               ('allowed', 'person@stanford.edu', 'recipient');",
        )
        .unwrap();

    assert_eq!(
        prune_ignored_domains(&connection, "gmail:test", &["lists.stanford.edu".into()]).unwrap(),
        2
    );
    let active: Vec<String> = connection
        .prepare("SELECT native_id FROM interactions WHERE deleted_at IS NULL ORDER BY native_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(active, ["allowed"]);
}

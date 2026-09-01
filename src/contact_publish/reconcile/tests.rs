use super::*;
use crate::db;
use crate::google_contacts::{Name, TypedValue};

#[test]
fn destructive_google_action_becomes_review_item() {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    let action = PlannedAction {
        kind: ActionKind::Delete,
        desired: None,
        remote: Some(Person {
            resource_name: Some("people/google-1".into()),
            ..Person::default()
        }),
        apple_id: "apple-1".into(),
        account: "personal@example.com".into(),
    };
    enqueue_delete(&connection, &action).unwrap();
    let items = review::pending(&connection).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind, "google_delete");
    assert_eq!(items[0].details["google_resource_name"], "people/google-1");
}

#[test]
fn google_candidate_summary_includes_phone_and_email() {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    let person = Person {
        resource_name: Some("people/google-1".into()),
        names: vec![Name {
            given_name: "Scott".into(),
            family_name: "Hickmann".into(),
            ..Name::default()
        }],
        phone_numbers: vec![TypedValue {
            value: "+1234567890".into(),
            kind: None,
        }],
        email_addresses: vec![TypedValue {
            value: "scott@example.com".into(),
            kind: None,
        }],
        ..Person::default()
    };

    enqueue_unmanaged_candidates(&connection, "personal@example.com", &[person]).unwrap();

    let item = review::pending(&connection).unwrap().pop().unwrap();
    assert_eq!(
        item.summary,
        "Create an iCloud contact for Scott Hickmann (+1234567890, scott@example.com)?"
    );
}

#[test]
fn unmanaged_google_contact_matching_icloud_is_not_a_candidate() {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    connection
        .execute_batch(
            "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
             VALUES ('person', 'Alex', 'apple-1', 'active');
             INSERT INTO identities(
                 id, person_id, kind, value, normalized_value, active
             ) VALUES ('phone', 'person', 'phone', '+1 555 0100', '15550100', 1);",
        )
        .unwrap();
    let person = Person {
        resource_name: Some("people/google-1".into()),
        phone_numbers: vec![TypedValue {
            value: "+1 (555) 0100".into(),
            kind: None,
        }],
        ..Person::default()
    };

    enqueue_unmanaged_candidates(&connection, "personal@example.com", &[person]).unwrap();

    assert!(review::pending(&connection).unwrap().is_empty());
}

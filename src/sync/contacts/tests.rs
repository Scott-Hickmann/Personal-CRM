use super::*;
use crate::db;

#[test]
fn retiring_contact_preserves_history_and_overlays() {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    connection
        .execute_batch(
            "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
             VALUES ('person', 'Alex', 'apple-1', 'active');
             INSERT INTO notes(id, person_id, body) VALUES ('note', 'person', 'keep me');
             INSERT INTO identities(id, person_id, kind, value, normalized_value, active)
             VALUES ('identity', 'person', 'email', 'alex@example.com', 'alex@example.com', 1);",
        )
        .unwrap();

    retire_missing(&connection, &HashSet::new()).unwrap();

    let state: String = connection
        .query_row(
            "SELECT lifecycle_state FROM people WHERE id='person'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let notes: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM notes WHERE person_id='person'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let identity_active: bool = connection
        .query_row(
            "SELECT active FROM identities WHERE id='identity'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "retired");
    assert_eq!(notes, 1);
    assert!(!identity_active);
}

#[test]
fn duplicate_identity_is_not_claimed_by_either_contact() {
    let mut first = sample_contact("apple-1");
    let mut second = sample_contact("apple-2");
    first.emails[0].value = "same@example.com".into();
    second.emails[0].value = "same@example.com".into();
    let duplicates = duplicate_identities(&[first, second]);
    assert_eq!(duplicates["same@example.com"], ["apple-1", "apple-2"]);
}

#[test]
fn company_contacts_are_retired_and_suppressed_from_suggestions() {
    let directory = tempfile::tempdir().unwrap();
    let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
    connection
        .execute_batch(
            "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
             VALUES ('company', 'Example Inc', 'apple-company', 'active');
             INSERT INTO identities(id, person_id, kind, value, normalized_value, active)
             VALUES ('company-email', 'company', 'email', 'hello@example.com',
                     'hello@example.com', 1);",
        )
        .unwrap();
    let mut company = sample_contact("apple-company");
    company.is_company = true;
    company.given_name.clear();
    company.family_name.clear();
    company.organization = "Example Inc".into();
    company.emails[0].value = "hello@example.com".into();
    let mut employee = sample_contact("apple-person");
    employee.organization = "Example Inc".into();

    let (contacts, companies) = partition_contacts(vec![company, employee]);
    assert_eq!(contacts.len(), 1);
    assert_eq!(contacts[0].id, "apple-person");
    assert_eq!(companies.len(), 1);
    refresh_company_exclusions(&connection, &companies).unwrap();
    let seen = contacts.iter().map(|contact| contact.id.as_str()).collect();
    retire_missing(&connection, &seen).unwrap();

    let state: String = connection
        .query_row(
            "SELECT lifecycle_state FROM people WHERE id='company'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "retired");
    assert!(
        crate::review_candidates::identity_belongs_to_icloud_contact(
            &connection,
            "hello@example.com"
        )
        .unwrap()
    );
}

fn sample_contact(id: &str) -> AppleContact {
    AppleContact {
        id: id.into(),
        is_company: false,
        name_prefix: String::new(),
        given_name: "Alex".into(),
        middle_name: String::new(),
        family_name: "Example".into(),
        name_suffix: String::new(),
        nickname: String::new(),
        emails: vec![apple::LabeledValue {
            label: None,
            value: "alex@example.com".into(),
        }],
        phones: Vec::new(),
        organization: String::new(),
        department: String::new(),
        job_title: String::new(),
    }
}

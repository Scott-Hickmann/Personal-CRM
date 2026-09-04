use super::*;

#[test]
fn self_addresses_include_contact_aliases_and_authorized_accounts() {
    let directory = tempfile::tempdir().unwrap();
    let connection = crate::db::open(&directory.path().join("crm.sqlite3")).unwrap();
    connection
        .execute_batch(
            "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
             VALUES ('self', 'Me', 'apple-self', 'active');
             INSERT INTO identities(
                 id, person_id, kind, value, normalized_value, is_self, active
             ) VALUES ('alias', 'self', 'email', 'Alias@Example.com',
                       'alias@example.com', 1, 1);",
        )
        .unwrap();
    let mut config = crate::config::Config::new("Me".into(), Vec::new()).unwrap();
    config.gmail.accounts = vec!["Mailbox@Example.com".into()];

    assert_eq!(
        self_addresses(&config, &connection).unwrap(),
        HashSet::from(["alias@example.com".into(), "mailbox@example.com".into()])
    );
}

#[test]
fn self_addresses_require_a_linked_contact_email() {
    let directory = tempfile::tempdir().unwrap();
    let connection = crate::db::open(&directory.path().join("crm.sqlite3")).unwrap();
    let mut config = crate::config::Config::new("Me".into(), Vec::new()).unwrap();
    config.gmail.accounts = vec!["mailbox@example.com".into()];

    let error = self_addresses(&config, &connection).unwrap_err();

    assert!(error.to_string().contains("linked iCloud self contact"));
}

#[test]
fn policy_fingerprint_is_stable_and_changes_with_ignored_domains() {
    let mut config = crate::config::Config::new("Me".into(), Vec::new()).unwrap();
    let original = policy_fingerprint(&config);
    config.gmail.ignored_domains = vec!["lists.stanford.edu".into(), " Lists.Stanford.EDU ".into()];

    assert_ne!(policy_fingerprint(&config), original);
    assert_eq!(
        policy_fingerprint(&config),
        format!("{POLICY_FINGERPRINT}:lists.stanford.edu")
    );
}

#[test]
fn failure_is_recorded_only_for_the_current_account() {
    let directory = tempfile::tempdir().unwrap();
    let connection = crate::db::open(&directory.path().join("crm.sqlite3")).unwrap();
    connection
        .execute_batch(
            "INSERT INTO sources(id, kind, account, status) VALUES
             ('gmail:first@example.com', 'gmail', 'first@example.com', 'ok'),
             ('gmail:second@example.com', 'gmail', 'second@example.com', 'ok');",
        )
        .unwrap();

    mark_failed(
        &connection,
        "first@example.com",
        POLICY_FINGERPRINT,
        "database is locked",
    );

    let first: (String, Option<String>) = connection
        .query_row(
            "SELECT status, error FROM sources WHERE id='gmail:first@example.com'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let second: (String, Option<String>) = connection
        .query_row(
            "SELECT status, error FROM sources WHERE id='gmail:second@example.com'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(first, ("failed".into(), Some("database is locked".into())));
    assert_eq!(second, ("ok".into(), None));
}

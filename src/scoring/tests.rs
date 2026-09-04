use super::*;

#[test]
fn explaining_a_missing_score_does_not_persist_it() {
    let directory = tempfile::tempdir().unwrap();
    let connection = crate::db::open(&directory.path().join("crm.sqlite3")).unwrap();
    connection
        .execute(
            "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
             VALUES ('person', 'Alex', 'apple-1', 'active')",
            [],
        )
        .unwrap();

    let explanation = explain(&connection, "person").unwrap();

    assert_eq!(explanation.display_name, "Alex");
    let metrics: i64 = connection
        .query_row("SELECT COUNT(*) FROM metrics", [], |row| row.get(0))
        .unwrap();
    assert_eq!(metrics, 0);
    let affinity: Option<f64> = connection
        .query_row(
            "SELECT affinity_score FROM people WHERE id='person'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(affinity, None);
}

#[test]
fn tiers_and_activity_have_explicit_boundaries() {
    assert_eq!(tier(80.0), "core");
    assert_eq!(tier(59.9), "familiar");
    assert_eq!(tier(0.0), "peripheral");
    assert_eq!(activity(Some(91.0)), "dormant");
    assert_eq!(activity(None), "never");
}

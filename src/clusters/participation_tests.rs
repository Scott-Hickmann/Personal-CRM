use super::*;

fn message(connection: &Connection, id: &str, person: &str, thread: &str, role: &str) {
    connection.execute(
        "INSERT INTO interactions(id,source_id,native_id,thread_native_id,channel,kind,occurred_at)
         VALUES (?1,'test',?1,?2,'imessage','message','2026')", params![id,thread],
    ).unwrap();
    connection
        .execute(
            "INSERT INTO interaction_participants(interaction_id,person_id,identity_value,role)
         VALUES (?1,?2,?2,?3)",
            params![id, person, role],
        )
        .unwrap();
}

fn weight(connection: &Connection, a: usize, b: usize) -> f64 {
    input::load(connection)
        .unwrap()
        .edges
        .iter()
        .find(|e| e.0 == a && e.1 == b)
        .unwrap()
        .2
}

#[test]
fn boost_requires_both_senders_and_has_diminishing_returns() {
    let (_directory, connection) = tests::database();
    let baseline = weight(&connection, 0, 1);
    for i in 0..100 {
        message(&connection, &format!("a{i}"), "a", "big", "sender");
    }
    assert_eq!(weight(&connection, 0, 1), baseline);
    message(&connection, "b0", "b", "big", "sender");
    assert!((weight(&connection, 0, 1) - baseline - 2_f64.ln() / 3.0).abs() < 1e-9);
    for i in 1..100 {
        message(&connection, &format!("b{i}"), "b", "big", "sender");
    }
    assert!((weight(&connection, 0, 1) - baseline - 101_f64.ln() / 3.0).abs() < 1e-9);
    assert_eq!(weight(&connection, 0, 2), 1.0 / 3.0);
    assert_eq!(input::load(&connection).unwrap().edges[0].3, 2.0);
}

#[test]
fn counts_ignore_recipients_deleted_messages_duplicates_and_other_threads() {
    let (_directory, connection) = tests::database();
    let baseline = weight(&connection, 0, 1);
    message(&connection, "a", "a", "big", "sender");
    message(&connection, "recipient", "b", "big", "recipient");
    message(&connection, "direct", "b", "direct-with-owner", "sender");
    message(&connection, "deleted", "b", "big", "sender");
    connection
        .execute(
            "UPDATE interactions SET deleted_at='2026' WHERE id='deleted'",
            [],
        )
        .unwrap();
    assert_eq!(weight(&connection, 0, 1), baseline);
    let before = load(&connection).unwrap();
    message(&connection, "b", "b", "big", "sender");
    connection
        .execute(
            "INSERT INTO interaction_participants(interaction_id,person_id,identity_value,role) VALUES ('b','b','alias','sender')",
            [],
        )
        .unwrap();
    assert!((weight(&connection, 0, 1) - baseline - 2_f64.ln() / 3.0).abs() < 1e-9);
    let after = load(&connection).unwrap();
    assert_ne!(json(&before).unwrap(), json(&after).unwrap());
    connection.execute("INSERT INTO identities(id,person_id,kind,value,normalized_value,is_self) VALUES ('self','b','email','b','b',1)", []).unwrap();
    assert!(
        !input::load(&connection)
            .unwrap()
            .people
            .iter()
            .any(|p| p.0 == "b")
    );
}

use rusqlite::{Connection, OptionalExtension};

use crate::error::{CrmError, Result};
use crate::repository;

pub fn delete_review_person(connection: &Connection, review_id: &str) -> Result<String> {
    let (kind, person_id): (String, String) = connection
        .query_row(
            "SELECT kind, subject_key FROM review_items WHERE id=?1 AND status='pending'",
            [review_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| {
            CrmError::InvalidConfig(format!("pending review item not found: {review_id}"))
        })?;
    if kind != "migration_person" {
        return Err(CrmError::InvalidConfig(
            "--delete-person is only valid for migration-person reviews".into(),
        ));
    }
    let lifecycle: String = connection
        .query_row(
            "SELECT lifecycle_state FROM people WHERE id=?1",
            [&person_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| CrmError::PersonNotFound(person_id.clone()))?;
    if lifecycle != "migration_pending" {
        return Err(CrmError::InvalidConfig(
            "only migration-pending people can be deleted from review".into(),
        ));
    }
    delete_person(connection, &person_id)?;
    Ok(person_id)
}

pub fn delete_retired_person(connection: &Connection, reference: &str) -> Result<String> {
    let person_id = repository::resolve_person_id(connection, reference)?;
    let lifecycle: String = connection.query_row(
        "SELECT lifecycle_state FROM people WHERE id=?1",
        [&person_id],
        |row| row.get(0),
    )?;
    if lifecycle != "retired" {
        return Err(CrmError::InvalidConfig(
            "only retired people can be deleted directly".into(),
        ));
    }
    delete_person(connection, &person_id)?;
    Ok(person_id)
}

fn delete_person(connection: &Connection, person_id: &str) -> Result<()> {
    let is_self: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM identities WHERE person_id=?1 AND is_self=1)",
        [person_id],
        |row| row.get(0),
    )?;
    if is_self {
        return Err(CrmError::InvalidConfig(
            "the configured self person cannot be deleted".into(),
        ));
    }

    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE interaction_participants SET person_id=NULL WHERE person_id=?1",
        [person_id],
    )?;
    transaction.execute(
        "UPDATE mentions SET person_id=NULL, status='unresolved' WHERE person_id=?1",
        [person_id],
    )?;
    transaction.execute(
        "DELETE FROM relationships WHERE source_person_id=?1 OR target_person_id=?1",
        [person_id],
    )?;
    transaction.execute(
        "DELETE FROM identity_candidates WHERE candidate_person_id=?1",
        [person_id],
    )?;
    transaction.execute("DELETE FROM review_items WHERE subject_key=?1", [person_id])?;
    transaction.execute("DELETE FROM people WHERE id=?1", [person_id])?;
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db, review};

    #[test]
    fn deletes_only_the_migration_person_and_preserves_interactions() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute_batch(
                "INSERT INTO people(id, display_name) VALUES ('legacy', 'Legacy');
                 INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
                 VALUES ('other', 'Other', 'apple-other', 'active');
                 INSERT INTO identities(id, person_id, kind, value, normalized_value)
                 VALUES ('identity', 'legacy', 'email', 'legacy@example.com', 'legacy@example.com');
                 INSERT INTO notes(id, person_id, body) VALUES ('note', 'legacy', 'old note');
                 INSERT INTO sources(id, kind) VALUES ('source', 'test');
                 INSERT INTO interactions(id, source_id, native_id, channel, kind, occurred_at)
                 VALUES ('interaction', 'source', 'native', 'email', 'message', '2026-01-01');
                 INSERT INTO interaction_participants(interaction_id, person_id, identity_value, role)
                 VALUES ('interaction', 'legacy', 'legacy@example.com', 'sender');
                 INSERT INTO relationships(
                     id, source_person_id, target_person_id, relationship_type, confidence
                 ) VALUES ('relationship', 'legacy', 'other', 'knows', 0.8);
                 INSERT INTO mentions(id, interaction_id, text, person_id, confidence, status)
                 VALUES ('mention', 'interaction', 'Legacy', 'legacy', 0.8, 'resolved');",
            )
            .unwrap();
        let review_id = review::enqueue(
            &connection,
            "migration_person",
            "legacy",
            "Link Legacy",
            serde_json::json!({}),
        )
        .unwrap();

        let person_id = delete_review_person(&connection, &review_id).unwrap();

        assert_eq!(person_id, "legacy");
        let people: i64 = connection
            .query_row("SELECT COUNT(*) FROM people WHERE id='legacy'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(people, 0);
        let participant: (Option<String>, String) = connection
            .query_row(
                "SELECT person_id, identity_value FROM interaction_participants",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(participant, (None, "legacy@example.com".into()));
        let mention: (Option<String>, String) = connection
            .query_row("SELECT person_id, status FROM mentions", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(mention, (None, "unresolved".into()));
        assert!(review::pending(&connection).unwrap().is_empty());
    }

    #[test]
    fn refuses_to_delete_an_active_person() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute(
                "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
                 VALUES ('active', 'Alex', 'apple-1', 'active')",
                [],
            )
            .unwrap();
        let review_id = review::enqueue(
            &connection,
            "migration_person",
            "active",
            "Link Alex",
            serde_json::json!({}),
        )
        .unwrap();

        let error = delete_review_person(&connection, &review_id).unwrap_err();

        assert!(error.to_string().contains("only migration-pending"));
        let people: i64 = connection
            .query_row("SELECT COUNT(*) FROM people WHERE id='active'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(people, 1);
    }

    #[test]
    fn deletes_a_retired_person_by_id() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute(
                "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
                 VALUES ('retired', 'Alex', 'apple-1', 'retired')",
                [],
            )
            .unwrap();

        let person_id = delete_retired_person(&connection, "retired").unwrap();

        assert_eq!(person_id, "retired");
        assert!(repository::get_person(&connection, "retired").is_err());
    }

    #[test]
    fn direct_delete_refuses_an_active_person() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute(
                "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
                 VALUES ('active', 'Alex', 'apple-1', 'active')",
                [],
            )
            .unwrap();

        let error = delete_retired_person(&connection, "active").unwrap_err();

        assert!(error.to_string().contains("only retired"));
        assert!(repository::get_person(&connection, "active").is_ok());
    }
}

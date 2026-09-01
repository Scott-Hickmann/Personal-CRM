use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use uuid::Uuid;

use crate::error::{CrmError, Result};

#[derive(Debug, Clone, Serialize)]
pub struct ReviewItem {
    pub id: String,
    pub kind: String,
    pub source: Option<String>,
    pub subject_key: String,
    pub summary: String,
    pub details: serde_json::Value,
    pub status: String,
    pub created_at: String,
}

pub fn enqueue(
    connection: &Connection,
    kind: &str,
    subject_key: &str,
    summary: &str,
    details: serde_json::Value,
) -> Result<String> {
    if let Some(id) = connection
        .query_row(
            "SELECT id FROM review_items WHERE kind=?1 AND subject_key=?2 AND status='pending'",
            params![kind, subject_key],
            |row| row.get(0),
        )
        .optional()?
    {
        connection.execute(
            "UPDATE review_items SET summary=?2, details_json=?3, updated_at=CURRENT_TIMESTAMP WHERE id=?1",
            params![id, summary, details.to_string()],
        )?;
        return Ok(id);
    }
    let id = Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO review_items(id, kind, subject_key, summary, details_json)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, kind, subject_key, summary, details.to_string()],
    )?;
    Ok(id)
}

pub fn pending(connection: &Connection) -> Result<Vec<ReviewItem>> {
    let mut statement = connection.prepare(
        "SELECT id, kind, subject_key, summary, details_json, status, created_at
         FROM review_items WHERE status='pending' ORDER BY created_at, id",
    )?;
    Ok(statement
        .query_map([], |row| {
            let kind: String = row.get(1)?;
            let details: String = row.get(4)?;
            let details: serde_json::Value = serde_json::from_str(&details).unwrap_or_default();
            Ok(ReviewItem {
                id: row.get(0)?,
                source: (kind == "contact_candidate")
                    .then(|| crate::review_candidates::source(&details))
                    .flatten(),
                kind,
                subject_key: row.get(2)?,
                summary: row.get(3)?,
                details,
                status: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?)
}

pub fn get_pending(connection: &Connection, id: &str) -> Result<ReviewItem> {
    pending(connection)?
        .into_iter()
        .find(|item| item.id == id)
        .ok_or_else(|| CrmError::InvalidConfig(format!("pending review item not found: {id}")))
}

pub fn resolve(connection: &Connection, id: &str) -> Result<()> {
    connection.execute(
        "UPDATE review_items SET status='resolved', resolved_at=CURRENT_TIMESTAMP,
         updated_at=CURRENT_TIMESTAMP WHERE id=?1 AND status='pending'",
        [id],
    )?;
    Ok(())
}

pub fn resolve_absent(
    connection: &Connection,
    kind: &str,
    active_subjects: &std::collections::HashSet<String>,
) -> Result<()> {
    let mut statement = connection
        .prepare("SELECT id, subject_key FROM review_items WHERE kind=?1 AND status='pending'")?;
    let rows: Vec<(String, String)> = statement
        .query_map([kind], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    for (id, subject) in rows {
        if !active_subjects.contains(&subject) {
            resolve(connection, &id)?;
        }
    }
    Ok(())
}

pub fn link_migration_person(
    connection: &Connection,
    review_id: &str,
    apple_contact_id: &str,
) -> Result<String> {
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
            "--link-icloud is only valid for migration-person reviews".into(),
        ));
    }
    if let Some(target_person_id) = connection
        .query_row(
            "SELECT id FROM people WHERE apple_contact_id=?1 AND id<>?2",
            params![apple_contact_id, person_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        let transaction = connection.unchecked_transaction()?;
        merge_migration_shell(&transaction, &person_id, &target_person_id)?;
        resolve(&transaction, review_id)?;
        transaction.commit()?;
        return Ok(target_person_id);
    }
    connection.execute(
        "UPDATE people SET apple_contact_id=?2, lifecycle_state='active', retired_at=NULL,
         updated_at=CURRENT_TIMESTAMP WHERE id=?1",
        params![person_id, apple_contact_id],
    )?;
    connection.execute(
        "UPDATE review_items SET status='resolved', resolved_at=CURRENT_TIMESTAMP,
         updated_at=CURRENT_TIMESTAMP WHERE id=?1",
        [review_id],
    )?;
    Ok(person_id)
}

pub fn reject(connection: &Connection, review_id: &str) -> Result<()> {
    let kind: String = connection
        .query_row(
            "SELECT kind FROM review_items WHERE id=?1 AND status='pending'",
            [review_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| {
            CrmError::InvalidConfig(format!("pending review item not found: {review_id}"))
        })?;
    if kind == "migration_person" {
        return Err(CrmError::InvalidConfig(
            "migration people must be linked to an iCloud contact".into(),
        ));
    }
    connection.execute(
        "UPDATE review_items SET status='rejected', resolved_at=CURRENT_TIMESTAMP,
         updated_at=CURRENT_TIMESTAMP WHERE id=?1",
        [review_id],
    )?;
    Ok(())
}

pub fn enqueue_unresolved_candidates(connection: &Connection) -> Result<usize> {
    crate::review_candidates::enqueue(connection)
}

pub fn pending_migration_count(connection: &Connection) -> Result<usize> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM people p WHERE lifecycle_state='migration_pending'
         AND NOT EXISTS (SELECT 1 FROM person_merges m WHERE m.source_person_id=p.id)",
        [],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}

fn merge_migration_shell(
    connection: &Connection,
    source_person_id: &str,
    target_person_id: &str,
) -> Result<()> {
    let has_history: bool = connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM interaction_participants WHERE person_id=?1
             UNION ALL SELECT 1 FROM notes WHERE person_id=?1
             UNION ALL SELECT 1 FROM facts WHERE person_id=?1
             UNION ALL SELECT 1 FROM tags WHERE person_id=?1
             UNION ALL SELECT 1 FROM identity_candidates WHERE candidate_person_id=?1
             UNION ALL SELECT 1 FROM important_dates WHERE person_id=?1
             UNION ALL SELECT 1 FROM followups WHERE person_id=?1
             UNION ALL SELECT 1 FROM cadences WHERE person_id=?1
             UNION ALL SELECT 1 FROM relationships
                 WHERE source_person_id=?1 OR target_person_id=?1
             UNION ALL SELECT 1 FROM mentions WHERE person_id=?1
             UNION ALL SELECT 1 FROM metrics WHERE person_id=?1
             UNION ALL SELECT 1 FROM semantic_chunks WHERE person_id=?1
             UNION ALL SELECT 1 FROM photo_links WHERE person_id=?1
         )",
        [source_person_id],
        |row| row.get(0),
    )?;
    if has_history {
        return Err(CrmError::Contacts(format!(
            "CRM person {source_person_id} has history and cannot be merged automatically"
        )));
    }
    connection.execute(
        "INSERT INTO person_merges(source_person_id, target_person_id) VALUES (?1, ?2)",
        params![source_person_id, target_person_id],
    )?;
    connection.execute(
        "UPDATE identities SET active=0 WHERE person_id=?1",
        [source_person_id],
    )?;
    connection.execute(
        "UPDATE review_items SET status='resolved', resolved_at=CURRENT_TIMESTAMP,
         updated_at=CURRENT_TIMESTAMP
         WHERE kind='identity_collision' AND status='pending'
         AND subject_key IN (
             SELECT normalized_value FROM identities WHERE person_id=?1
         )",
        [source_person_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn migration_link_preserves_person_and_resolves_review() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute(
                "INSERT INTO people(id, display_name) VALUES ('person', 'Alex')",
                [],
            )
            .unwrap();
        let review_id = enqueue(
            &connection,
            "migration_person",
            "person",
            "Link Alex",
            serde_json::json!({}),
        )
        .unwrap();
        let person_id = link_migration_person(&connection, &review_id, "apple-1").unwrap();
        assert_eq!(person_id, "person");
        let row: (String, String) = connection
            .query_row(
                "SELECT apple_contact_id, lifecycle_state FROM people WHERE id='person'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row, ("apple-1".into(), "active".into()));
        assert!(pending(&connection).unwrap().is_empty());
    }

    #[test]
    fn migration_link_merges_empty_shell_into_existing_icloud_person() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute_batch(
                "INSERT INTO people(id, display_name) VALUES ('legacy', 'Alex');
                 INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
                 VALUES ('canonical', 'Alex', 'apple-1', 'active');
                 INSERT INTO identities(
                     id, person_id, kind, value, normalized_value, is_self, active
                 ) VALUES (
                     'identity', 'legacy', 'email', 'alex@example.com',
                     'alex@example.com', 1, 1
                 );",
            )
            .unwrap();
        let migration_review = enqueue(
            &connection,
            "migration_person",
            "legacy",
            "Link Alex",
            serde_json::json!({}),
        )
        .unwrap();
        enqueue(
            &connection,
            "identity_collision",
            "alex@example.com",
            "Identity collision",
            serde_json::json!({}),
        )
        .unwrap();

        let person_id = link_migration_person(&connection, &migration_review, "apple-1").unwrap();

        assert_eq!(person_id, "canonical");
        let merge: String = connection
            .query_row(
                "SELECT target_person_id FROM person_merges WHERE source_person_id='legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(merge, "canonical");
        let active: bool = connection
            .query_row(
                "SELECT active FROM identities WHERE id='identity'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!active);
        assert_eq!(pending_migration_count(&connection).unwrap(), 0);
        assert!(pending(&connection).unwrap().is_empty());
    }

    #[test]
    fn migration_link_does_not_merge_a_person_with_history() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute_batch(
                "INSERT INTO people(id, display_name) VALUES ('legacy', 'Alex');
                 INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
                 VALUES ('canonical', 'Alex', 'apple-1', 'active');
                 INSERT INTO notes(id, person_id, body)
                 VALUES ('note', 'legacy', 'Keep me');",
            )
            .unwrap();
        let review_id = enqueue(
            &connection,
            "migration_person",
            "legacy",
            "Link Alex",
            serde_json::json!({}),
        )
        .unwrap();

        let error = link_migration_person(&connection, &review_id, "apple-1").unwrap_err();

        assert!(error.to_string().contains("has history"));
        let merges: i64 = connection
            .query_row("SELECT COUNT(*) FROM person_merges", [], |row| row.get(0))
            .unwrap();
        assert_eq!(merges, 0);
        assert_eq!(pending_migration_count(&connection).unwrap(), 1);
    }
}

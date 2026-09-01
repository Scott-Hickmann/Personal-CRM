use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use uuid::Uuid;

use crate::error::{CrmError, Result};

#[derive(Debug, Clone, Serialize)]
pub struct ReviewItem {
    pub id: String,
    pub kind: String,
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
            let details: String = row.get(4)?;
            Ok(ReviewItem {
                id: row.get(0)?,
                kind: row.get(1)?,
                subject_key: row.get(2)?,
                summary: row.get(3)?,
                details: serde_json::from_str(&details).unwrap_or_default(),
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
    if connection
        .query_row(
            "SELECT id FROM people WHERE apple_contact_id=?1 AND id<>?2",
            params![apple_contact_id, person_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .is_some()
    {
        return Err(CrmError::Contacts(format!(
            "iCloud contact {apple_contact_id} is already linked to another CRM person"
        )));
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
    resolve_non_person_contact_candidates(connection)?;
    let mut statement = connection.prepare(
        "SELECT lower(trim(ip.identity_value)), COUNT(*), group_concat(DISTINCT i.channel),
                MAX(NULLIF(trim(ip.display_name), ''))
         FROM interaction_participants ip JOIN interactions i ON i.id=ip.interaction_id
         WHERE ip.person_id IS NULL AND ip.identity_value IS NOT NULL
           AND trim(ip.identity_value) != '' AND i.deleted_at IS NULL
         GROUP BY lower(trim(ip.identity_value)) ORDER BY COUNT(*) DESC",
    )?;
    let mut rows: Vec<(String, i64, String, Option<String>)> = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    rows.retain(|row| !is_non_person_whatsapp_identity(&row.0));
    for (identity, count, channels, name) in &rows {
        let label = name.as_deref().unwrap_or(identity);
        enqueue(
            connection,
            "contact_candidate",
            identity,
            &format!("Create an iCloud contact for {label}?"),
            serde_json::json!({"name": name, "identity": identity, "interaction_count": count, "channels": channels}),
        )?;
    }
    Ok(rows.len())
}

fn resolve_non_person_contact_candidates(connection: &Connection) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT id, subject_key FROM review_items
         WHERE kind='contact_candidate' AND status='pending'",
    )?;
    let rows: Vec<(String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    for (id, subject) in rows {
        if is_non_person_whatsapp_identity(&subject) {
            resolve(connection, &id)?;
        }
    }
    Ok(())
}

fn is_non_person_whatsapp_identity(identity: &str) -> bool {
    let identity = identity.trim().to_ascii_lowercase();
    ["@g.us", "@broadcast", "@newsletter"]
        .iter()
        .any(|suffix| identity.ends_with(suffix))
}

pub fn pending_migration_count(connection: &Connection) -> Result<usize> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM people WHERE lifecycle_state='migration_pending'",
        [],
        |row| row.get(0),
    )?;
    Ok(count as usize)
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
    fn contact_candidate_uses_source_name() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute_batch(
                "INSERT INTO sources(id, kind) VALUES ('messages', 'messages');
                 INSERT INTO interactions(
                     id, source_id, native_id, channel, kind, occurred_at, last_seen_at
                 ) VALUES ('message', 'messages', 'native', 'iMessage', 'message',
                           '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
                 INSERT INTO interaction_participants(
                     interaction_id, identity_value, display_name, role
                 ) VALUES ('message', '+15550100', 'Alex Example', 'sender');",
            )
            .unwrap();

        enqueue_unresolved_candidates(&connection).unwrap();

        let item = pending(&connection).unwrap().pop().unwrap();
        assert_eq!(item.summary, "Create an iCloud contact for Alex Example?");
        assert_eq!(item.details["name"], "Alex Example");
        assert_eq!(item.details["identity"], "+15550100");
    }

    #[test]
    fn whatsapp_conversation_entities_are_not_contact_candidates() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute_batch(
                "INSERT INTO sources(id, kind) VALUES ('whatsapp', 'whatsapp');
                 INSERT INTO interactions(
                     id, source_id, native_id, channel, kind, occurred_at, last_seen_at
                 ) VALUES
                     ('group-message', 'whatsapp', 'group', 'whatsapp', 'message',
                      '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'),
                     ('person-message', 'whatsapp', 'person', 'whatsapp', 'message',
                      '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
                 INSERT INTO interaction_participants(
                     interaction_id, identity_value, display_name, role
                 ) VALUES
                     ('group-message', '120363000000@g.us', 'Family', 'recipient'),
                     ('person-message', '15550100@s.whatsapp.net', 'Alex', 'sender');",
            )
            .unwrap();
        let group_review = enqueue(
            &connection,
            "contact_candidate",
            "120363000000@g.us",
            "Create an iCloud contact for Family?",
            serde_json::json!({}),
        )
        .unwrap();

        assert_eq!(enqueue_unresolved_candidates(&connection).unwrap(), 1);

        let items = pending(&connection).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].subject_key, "15550100@s.whatsapp.net");
        let group_status: String = connection
            .query_row(
                "SELECT status FROM review_items WHERE id=?1",
                [group_review],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(group_status, "resolved");
    }
}

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
    let mut statement = connection.prepare(
        "SELECT lower(trim(identity_value)), COUNT(*), group_concat(DISTINCT i.channel)
         FROM interaction_participants ip JOIN interactions i ON i.id=ip.interaction_id
         WHERE ip.person_id IS NULL AND ip.identity_value IS NOT NULL
           AND trim(ip.identity_value) != '' AND i.deleted_at IS NULL
         GROUP BY lower(trim(identity_value)) ORDER BY COUNT(*) DESC",
    )?;
    let rows: Vec<(String, i64, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    for (identity, count, channels) in &rows {
        enqueue(
            connection,
            "contact_candidate",
            identity,
            &format!("Create an iCloud contact for {identity}?"),
            serde_json::json!({"identity": identity, "interaction_count": count, "channels": channels}),
        )?;
    }
    Ok(rows.len())
}

pub fn pending_migration_count(connection: &Connection) -> Result<usize> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM people WHERE lifecycle_state='migration_pending'",
        [],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use uuid::Uuid;

use crate::config::SelfIdentity;
use crate::error::{CrmError, Result};

#[derive(Debug, Serialize)]
pub struct Person {
    pub id: String,
    pub display_name: String,
    pub apple_contact_id: Option<String>,
    pub lifecycle_state: String,
    pub affinity_score: Option<f64>,
    pub affinity_tier: Option<String>,
    pub activity_state: Option<String>,
    pub identities: Vec<Identity>,
    pub notes: Vec<Note>,
    pub facts: Vec<Fact>,
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Identity {
    pub kind: String,
    pub value: String,
    pub is_self: bool,
}

#[derive(Debug, Serialize)]
pub struct Note {
    pub id: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct Fact {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Serialize)]
pub struct MutationPreview {
    pub operation: String,
    pub person_id: String,
    pub value: serde_json::Value,
    pub dry_run: bool,
}

pub fn ensure_self(connection: &Connection, identity: &SelfIdentity) -> Result<String> {
    let apple_person_id = identity
        .apple_contact_id
        .as_deref()
        .map(|apple_id| {
            connection
                .query_row(
                    "SELECT id FROM people WHERE apple_contact_id=?1",
                    [apple_id],
                    |row| row.get(0),
                )
                .optional()
        })
        .transpose()?
        .flatten();
    let person_id = apple_person_id
        .or(connection
            .query_row(
                "SELECT i.person_id FROM identities i
                 WHERE i.is_self=1 AND i.active=1
                 AND NOT EXISTS (
                     SELECT 1 FROM person_merges m WHERE m.source_person_id=i.person_id
                 )
                 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    connection.execute(
        "INSERT INTO people(id, display_name, apple_contact_id) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, updated_at = CURRENT_TIMESTAMP",
        params![person_id, identity.name, identity.apple_contact_id],
    )?;
    for phone in &identity.phones {
        upsert_identity(connection, &person_id, "phone", phone, true)?;
    }
    for whatsapp in &identity.whatsapp_ids {
        upsert_identity(connection, &person_id, "whatsapp", whatsapp, true)?;
    }
    Ok(person_id)
}

pub fn active_self_emails(connection: &Connection) -> Result<HashSet<String>> {
    let mut statement = connection.prepare(
        "SELECT DISTINCT lower(trim(i.normalized_value))
         FROM identities i JOIN people p ON p.id=i.person_id
         WHERE i.kind='email' AND i.active=1 AND i.is_self=1
           AND p.lifecycle_state='active' AND p.apple_contact_id IS NOT NULL",
    )?;
    Ok(statement
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?)
}

pub fn resolve_person_id(connection: &Connection, reference: &str) -> Result<String> {
    if let Some(id) = connection
        .query_row("SELECT id FROM people WHERE id = ?1", [reference], |row| {
            row.get(0)
        })
        .optional()?
    {
        return Ok(id);
    }
    let mut statement = connection.prepare(
        "SELECT id FROM people WHERE lifecycle_state != 'migration_pending'
         AND display_name LIKE ?1 COLLATE NOCASE ORDER BY lifecycle_state, display_name LIMIT 2",
    )?;
    let pattern = format!("%{}%", reference.replace('%', "\\%").replace('_', "\\_"));
    let ids: Vec<String> = statement
        .query_map([pattern], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    match ids.as_slice() {
        [] => Err(CrmError::PersonNotFound(reference.into())),
        [id] => Ok(id.clone()),
        _ => Err(CrmError::AmbiguousPerson(reference.into())),
    }
}

pub fn get_person(connection: &Connection, reference: &str) -> Result<Person> {
    let id = resolve_person_id(connection, reference)?;
    let mut person = connection.query_row(
        "SELECT id, display_name, apple_contact_id, lifecycle_state,
                affinity_score, affinity_tier, activity_state FROM people WHERE id = ?1",
        [&id],
        |row| {
            Ok(Person {
                id: row.get(0)?,
                display_name: row.get(1)?,
                apple_contact_id: row.get(2)?,
                lifecycle_state: row.get(3)?,
                affinity_score: row.get(4)?,
                affinity_tier: row.get(5)?,
                activity_state: row.get(6)?,
                identities: Vec::new(),
                notes: Vec::new(),
                facts: Vec::new(),
                tags: Vec::new(),
            })
        },
    )?;
    person.identities = collect(
        connection,
        "SELECT kind, value, is_self FROM identities WHERE person_id=?1
         AND (
             active=1 OR EXISTS(
                 SELECT 1 FROM people WHERE id=?1 AND lifecycle_state='retired'
             )
         )",
        &id,
        |row| {
            Ok(Identity {
                kind: row.get(0)?,
                value: row.get(1)?,
                is_self: row.get::<_, i64>(2)? != 0,
            })
        },
    )?;
    person.notes = collect(
        connection,
        "SELECT id, body, created_at FROM notes WHERE person_id = ?1 ORDER BY created_at DESC",
        &id,
        |row| {
            Ok(Note {
                id: row.get(0)?,
                body: row.get(1)?,
                created_at: row.get(2)?,
            })
        },
    )?;
    person.facts = collect(
        connection,
        "SELECT key, value FROM facts WHERE person_id = ?1 ORDER BY key",
        &id,
        |row| {
            Ok(Fact {
                key: row.get(0)?,
                value: row.get(1)?,
            })
        },
    )?;
    person.tags = collect(
        connection,
        "SELECT tag FROM tags WHERE person_id = ?1 ORDER BY tag",
        &id,
        |row| row.get(0),
    )?;
    Ok(person)
}

pub fn add_note(
    connection: &Connection,
    reference: &str,
    body: &str,
    dry_run: bool,
) -> Result<MutationPreview> {
    mutate(
        connection,
        reference,
        "note.add",
        serde_json::json!({"body": body}),
        dry_run,
        |connection, person_id| {
            connection.execute(
                "INSERT INTO notes(id, person_id, body) VALUES (?1, ?2, ?3)",
                params![Uuid::new_v4().to_string(), person_id, body],
            )?;
            Ok(())
        },
    )
}

pub fn set_fact(
    connection: &Connection,
    reference: &str,
    key: &str,
    value: &str,
    dry_run: bool,
) -> Result<MutationPreview> {
    mutate(
        connection,
        reference,
        "fact.set",
        serde_json::json!({"key": key, "value": value}),
        dry_run,
        |connection, person_id| {
            connection.execute("INSERT INTO facts(person_id, key, value) VALUES (?1, ?2, ?3) ON CONFLICT(person_id, key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP", params![person_id, key, value])?;
            Ok(())
        },
    )
}

pub fn add_tag(
    connection: &Connection,
    reference: &str,
    tag: &str,
    dry_run: bool,
) -> Result<MutationPreview> {
    mutate(
        connection,
        reference,
        "tag.add",
        serde_json::json!({"tag": tag}),
        dry_run,
        |connection, person_id| {
            connection.execute(
                "INSERT OR IGNORE INTO tags(person_id, tag) VALUES (?1, ?2)",
                params![person_id, tag],
            )?;
            Ok(())
        },
    )
}

pub fn add_followup(
    connection: &Connection,
    reference: &str,
    body: &str,
    due_at: Option<&str>,
    dry_run: bool,
) -> Result<MutationPreview> {
    mutate(
        connection,
        reference,
        "followup.add",
        serde_json::json!({"body": body, "due_at": due_at}),
        dry_run,
        |connection, person_id| {
            connection.execute(
                "INSERT INTO followups(id, person_id, body, due_at) VALUES (?1, ?2, ?3, ?4)",
                params![Uuid::new_v4().to_string(), person_id, body, due_at],
            )?;
            Ok(())
        },
    )
}

pub(crate) fn upsert_identity(
    connection: &Connection,
    person_id: &str,
    kind: &str,
    value: &str,
    is_self: bool,
) -> Result<()> {
    let normalized = normalize_identity(kind, value);
    let owner: Option<String> = connection
        .query_row(
            "SELECT person_id FROM identities WHERE kind=?1 AND normalized_value=?2 AND active=1",
            params![kind, normalized],
            |row| row.get(0),
        )
        .optional()?;
    if owner.as_deref().is_some_and(|owner| owner != person_id) {
        return Err(CrmError::Contacts(format!(
            "identity {value} is already assigned to another active iCloud contact"
        )));
    }
    if let Some(id) = connection
        .query_row(
            "SELECT id FROM identities WHERE person_id=?1 AND kind=?2 AND normalized_value=?3 LIMIT 1",
            params![person_id, kind, normalized],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        connection.execute(
            "UPDATE identities SET value=?2, is_self=MAX(is_self, ?3), active=1 WHERE id=?1",
            params![id, value, is_self as i64],
        )?;
    } else {
        connection.execute(
            "INSERT INTO identities(id, person_id, kind, value, normalized_value, is_self, active)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
            params![Uuid::new_v4().to_string(), person_id, kind, value, normalized, is_self as i64],
        )?;
    }
    Ok(())
}

pub(crate) fn normalize_identity(kind: &str, value: &str) -> String {
    match kind {
        "phone" | "whatsapp" => crate::phone::normalize(value),
        _ => value.trim().to_lowercase(),
    }
}

pub(crate) fn normalize_observed_identity(value: &str) -> String {
    let value = value.trim().to_lowercase();
    if let Some((local, domain)) = value.rsplit_once('@')
        && matches!(domain, "s.whatsapp.net" | "c.us")
    {
        return normalize_identity("phone", local);
    }
    if value.contains('@') {
        normalize_identity("email", &value)
    } else {
        normalize_identity("phone", &value)
    }
}

fn mutate<F>(
    connection: &Connection,
    reference: &str,
    operation: &str,
    value: serde_json::Value,
    dry_run: bool,
    action: F,
) -> Result<MutationPreview>
where
    F: FnOnce(&Connection, &str) -> Result<()>,
{
    let person_id = resolve_person_id(connection, reference)?;
    if !dry_run {
        action(connection, &person_id)?;
    }
    Ok(MutationPreview {
        operation: operation.into(),
        person_id,
        value,
        dry_run,
    })
}

fn collect<T, F>(connection: &Connection, sql: &str, id: &str, map: F) -> Result<Vec<T>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut statement = connection.prepare(sql)?;
    Ok(statement
        .query_map([id], map)?
        .collect::<std::result::Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn normalizes_observed_phone_and_whatsapp_identities() {
        assert_eq!(normalize_observed_identity("+1 (555) 0100"), "15550100");
        assert_eq!(
            normalize_observed_identity("15550100@s.whatsapp.net"),
            "15550100"
        );
        assert_eq!(
            normalize_observed_identity("2207730634782@lid"),
            "2207730634782@lid"
        );
        assert_eq!(
            normalize_observed_identity("Alex@Example.com"),
            "alex@example.com"
        );
    }

    #[test]
    fn ensure_self_prefers_the_configured_icloud_person_over_a_merged_identity() {
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
                     'alex@example.com', 1, 0
                 );
                 INSERT INTO person_merges(source_person_id, target_person_id)
                 VALUES ('legacy', 'canonical');",
            )
            .unwrap();
        let identity = SelfIdentity {
            name: "Alex".into(),
            apple_contact_id: Some("apple-1".into()),
            phones: Vec::new(),
            whatsapp_ids: Vec::new(),
        };

        let person_id = ensure_self(&connection, &identity).unwrap();

        assert_eq!(person_id, "canonical");
    }

    #[test]
    fn active_self_emails_come_only_from_the_linked_active_contact() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute_batch(
                "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state) VALUES
                 ('self', 'Me', 'apple-self', 'active'),
                 ('other', 'Other', 'apple-other', 'active'),
                 ('retired', 'Old Me', 'apple-retired', 'retired');
                 INSERT INTO identities(
                     id, person_id, kind, value, normalized_value, is_self, active
                 ) VALUES
                 ('alias', 'self', 'email', 'Alias@Example.com', 'alias@example.com', 1, 1),
                 ('phone', 'self', 'phone', '+15550100', '15550100', 1, 1),
                 ('other', 'other', 'email', 'other@example.com', 'other@example.com', 0, 1),
                 ('retired', 'retired', 'email', 'old@example.com', 'old@example.com', 1, 0);",
            )
            .unwrap();

        assert_eq!(
            active_self_emails(&connection).unwrap(),
            HashSet::from(["alias@example.com".into()])
        );
    }

    #[test]
    fn retired_person_show_includes_preserved_inactive_identities() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute_batch(
                "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
                 VALUES ('retired', 'Alex', 'apple-1', 'retired');
                 INSERT INTO identities(
                     id, person_id, kind, value, normalized_value, active
                 ) VALUES (
                     'identity', 'retired', 'email', 'alex@example.com',
                     'alex@example.com', 0
                 );",
            )
            .unwrap();

        let person = get_person(&connection, "retired").unwrap();

        assert_eq!(person.identities.len(), 1);
        assert_eq!(person.identities[0].value, "alex@example.com");
    }
}

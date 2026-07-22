use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use uuid::Uuid;

use crate::config::SelfIdentity;
use crate::error::{CrmError, Result};

#[derive(Debug, Serialize)]
pub struct Person {
    pub id: String,
    pub display_name: String,
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
    let person_id = connection
        .query_row(
            "SELECT person_id FROM identities WHERE is_self = 1 LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    connection.execute(
        "INSERT INTO people(id, display_name) VALUES (?1, ?2)
         ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, updated_at = CURRENT_TIMESTAMP",
        params![person_id, identity.name],
    )?;
    for email in &identity.emails {
        upsert_identity(connection, &person_id, "email", email, true)?;
    }
    for phone in &identity.phones {
        upsert_identity(connection, &person_id, "phone", phone, true)?;
    }
    for whatsapp in &identity.whatsapp_ids {
        upsert_identity(connection, &person_id, "whatsapp", whatsapp, true)?;
    }
    Ok(person_id)
}

pub fn create_person(
    connection: &Connection,
    name: &str,
    dry_run: bool,
) -> Result<MutationPreview> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CrmError::InvalidConfig(
            "person name cannot be empty".into(),
        ));
    }
    let id = Uuid::new_v4().to_string();
    if !dry_run {
        connection.execute(
            "INSERT INTO people(id, display_name) VALUES (?1, ?2)",
            params![id, name],
        )?;
    }
    Ok(MutationPreview {
        operation: "person.add".into(),
        person_id: id,
        value: serde_json::json!({"display_name": name}),
        dry_run,
    })
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
        "SELECT id FROM people WHERE display_name LIKE ?1 COLLATE NOCASE ORDER BY display_name LIMIT 2",
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
        "SELECT id, display_name, affinity_score, affinity_tier, activity_state FROM people WHERE id = ?1",
        [&id],
        |row| Ok(Person {
            id: row.get(0)?, display_name: row.get(1)?, affinity_score: row.get(2)?,
            affinity_tier: row.get(3)?, activity_state: row.get(4)?, identities: Vec::new(),
            notes: Vec::new(), facts: Vec::new(), tags: Vec::new(),
        }),
    )?;
    person.identities = collect(
        connection,
        "SELECT kind, value, is_self FROM identities WHERE person_id = ?1",
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

fn upsert_identity(
    connection: &Connection,
    person_id: &str,
    kind: &str,
    value: &str,
    is_self: bool,
) -> Result<()> {
    let normalized = normalize_identity(kind, value);
    connection.execute(
        "INSERT INTO identities(id, person_id, kind, value, normalized_value, is_self)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(kind, normalized_value) DO UPDATE SET person_id = excluded.person_id, value = excluded.value, is_self = MAX(is_self, excluded.is_self)",
        params![Uuid::new_v4().to_string(), person_id, kind, value, normalized, is_self as i64],
    )?;
    Ok(())
}

fn normalize_identity(kind: &str, value: &str) -> String {
    match kind {
        "phone" | "whatsapp" => value
            .chars()
            .filter(|character| character.is_ascii_digit() || *character == '+')
            .collect(),
        _ => value.trim().to_lowercase(),
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

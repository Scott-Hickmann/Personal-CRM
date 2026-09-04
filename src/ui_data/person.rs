use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{CrmError, Result};
use crate::{repository, scoring};

use super::{
    Cadence, Followup, ImportantDate, InteractionBody, InteractionPreview, PersonDetail, PhotoLink,
    Relationship, attachments,
};

pub fn load(connection: &Connection, reference: &str, limit: u32) -> Result<PersonDetail> {
    let person_id = repository::resolve_person_id(connection, reference)?;
    let mut person = repository::get_person(connection, &person_id)?;
    for identity in &mut person.identities {
        if matches!(identity.kind.as_str(), "phone" | "whatsapp") {
            identity.value = crate::phone::format_for_display(&identity.value);
        }
    }
    Ok(PersonDetail {
        person,
        score: scoring::explain(connection, &person_id)?,
        interactions: interactions(connection, &person_id, limit)?,
        relationships: relationships(connection, &person_id)?,
        important_dates: important_dates(connection, &person_id)?,
        followups: followups(connection, &person_id)?,
        cadence: cadence(connection, &person_id)?,
        photo: photo(connection, &person_id)?,
    })
}

pub fn load_interaction(connection: &Connection, id: &str) -> Result<InteractionBody> {
    connection
        .query_row(
            "SELECT id, subject, body FROM interactions WHERE id=?1 AND deleted_at IS NULL",
            [id],
            |row| {
                Ok(InteractionBody {
                    id: row.get(0)?,
                    subject: row.get(1)?,
                    body: row.get(2)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| CrmError::InvalidQuery(format!("interaction not found: {id}")))
}

fn interactions(
    connection: &Connection,
    person_id: &str,
    limit: u32,
) -> Result<Vec<InteractionPreview>> {
    let mut statement = connection.prepare(
        "SELECT DISTINCT i.id, i.channel, i.kind, i.occurred_at, i.direction, i.subject, i.body
         FROM interactions i JOIN interaction_participants ip ON ip.interaction_id=i.id
         WHERE ip.person_id=?1 AND i.deleted_at IS NULL ORDER BY i.occurred_at DESC LIMIT ?2",
    )?;
    let rows = statement
        .query_map(params![person_id, limit], |row| {
            let body: Option<String> = row.get(6)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                body,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(
            |(id, channel, kind, occurred_at, direction, subject, body)| {
                let preview = body
                    .as_deref()
                    .map(|value| value.chars().take(240).collect());
                Ok(InteractionPreview {
                    attachments: attachments(connection, &id)?,
                    id,
                    channel,
                    kind,
                    occurred_at,
                    direction,
                    subject,
                    preview,
                    has_body: body.is_some_and(|value| !value.is_empty()),
                })
            },
        )
        .collect()
}

fn relationships(connection: &Connection, person_id: &str) -> Result<Vec<Relationship>> {
    let mut statement = connection.prepare(
        "SELECT r.id,
                CASE WHEN r.source_person_id=?1 THEN target.id ELSE source.id END,
                CASE WHEN r.source_person_id=?1 THEN target.display_name ELSE source.display_name END,
                r.shared_context_count,
                r.first_observed_at, r.last_observed_at
         FROM relationships r
         JOIN people source ON source.id=r.source_person_id
         JOIN people target ON target.id=r.target_person_id
         WHERE r.source_person_id=?1 OR r.target_person_id=?1
         ORDER BY r.shared_context_count DESC, r.last_observed_at DESC",
    )?;
    Ok(statement
        .query_map([person_id], |row| {
            Ok(Relationship {
                id: row.get(0)?,
                person_id: row.get(1)?,
                display_name: row.get(2)?,
                shared_context_count: row.get(3)?,
                first_observed_at: row.get(4)?,
                last_observed_at: row.get(5)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?)
}

fn important_dates(connection: &Connection, person_id: &str) -> Result<Vec<ImportantDate>> {
    let mut statement = connection.prepare(
        "SELECT id, label, date, recurring FROM important_dates WHERE person_id=?1 ORDER BY date",
    )?;
    Ok(statement
        .query_map([person_id], |row| {
            Ok(ImportantDate {
                id: row.get(0)?,
                label: row.get(1)?,
                date: row.get(2)?,
                recurring: row.get::<_, i64>(3)? != 0,
            })
        })?
        .collect::<std::result::Result<_, _>>()?)
}

fn followups(connection: &Connection, person_id: &str) -> Result<Vec<Followup>> {
    let mut statement = connection.prepare(
        "SELECT id, body, due_at, completed_at, created_at FROM followups
         WHERE person_id=?1 ORDER BY completed_at IS NOT NULL, due_at IS NULL, due_at, created_at DESC",
    )?;
    Ok(statement
        .query_map([person_id], |row| {
            Ok(Followup {
                id: row.get(0)?,
                body: row.get(1)?,
                due_at: row.get(2)?,
                completed_at: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?)
}

fn cadence(connection: &Connection, person_id: &str) -> Result<Option<Cadence>> {
    Ok(connection
        .query_row(
            "SELECT interval_days, updated_at FROM cadences WHERE person_id=?1",
            [person_id],
            |row| {
                Ok(Cadence {
                    interval_days: row.get(0)?,
                    updated_at: row.get(1)?,
                })
            },
        )
        .optional()?)
}

fn photo(connection: &Connection, person_id: &str) -> Result<Option<PhotoLink>> {
    Ok(connection
        .query_row(
            "SELECT photos_name_snapshot, photos_asset_id, state, reviewed_at, updated_at
         FROM photo_links WHERE person_id=?1",
            [person_id],
            |row| {
                Ok(PhotoLink {
                    photos_name: row.get(0)?,
                    photos_asset_id: row.get(1)?,
                    state: row.get(2)?,
                    reviewed_at: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            },
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn interaction_body_is_available_only_from_explicit_lookup() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection.execute_batch(
            "INSERT INTO sources(id, kind) VALUES ('source', 'test');
             INSERT INTO interactions(id, source_id, native_id, channel, kind, occurred_at, body)
             VALUES ('interaction', 'source', 'native', 'email', 'message', '2026-01-01', 'private body');",
        ).unwrap();

        let body = load_interaction(&connection, "interaction").unwrap();

        assert_eq!(body.body.as_deref(), Some("private body"));
    }

    #[test]
    fn person_detail_includes_followups_written_through_repository() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute(
                "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
             VALUES ('person', 'Alex', 'apple-alex', 'active')",
                [],
            )
            .unwrap();
        repository::add_followup(
            &connection,
            "person",
            "Check in",
            Some("2026-09-10T10:00"),
            false,
        )
        .unwrap();

        let detail = load(&connection, "person", 10).unwrap();

        assert_eq!(detail.followups.len(), 1);
        assert_eq!(detail.followups[0].body, "Check in");
    }
}

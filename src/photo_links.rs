use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::error::Result;
use crate::face_matching::BoundingBox;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PhotoReviewPerson {
    pub person_id: String,
    pub display_name: String,
    pub affinity_score: Option<f64>,
    pub link: Option<PhotoLink>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PhotoLink {
    pub photos_person_uuid: Option<String>,
    pub photos_name_snapshot: Option<String>,
    pub photos_asset_id: Option<String>,
    pub selected_face_index: Option<usize>,
    pub selected_face_bounds: Option<BoundingBox>,
    pub source_sha256: Option<String>,
    pub state: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct PhotoStatus {
    pub total_people: usize,
    pub pending: usize,
    pub deferred: usize,
    pub asset_linked: usize,
    pub person_linked: usize,
    pub not_applicable: usize,
    pub stale: usize,
}

pub(crate) fn review_people(
    connection: &Connection,
    person_id: Option<&str>,
) -> Result<Vec<PhotoReviewPerson>> {
    let mut statement = connection.prepare(
        "SELECT p.id, p.display_name, p.affinity_score,
                l.photos_person_uuid, l.photos_name_snapshot, l.photos_asset_id,
                l.selected_face_index, l.selected_face_bounds_json, l.source_sha256, l.state
         FROM people p
         LEFT JOIN photo_links l ON l.person_id = p.id
         WHERE p.lifecycle_state='active' AND (?1 IS NULL OR p.id = ?1)
         ORDER BY CASE COALESCE(l.state, 'pending')
                    WHEN 'pending' THEN 0 WHEN 'deferred' THEN 1 ELSE 2 END,
                  COALESCE(p.affinity_score, -1) DESC, p.display_name COLLATE NOCASE",
    )?;
    let rows = statement.query_map([person_id], |row| {
        let state: Option<String> = row.get(9)?;
        let bounds: Option<String> = row.get(7)?;
        let photos_person_uuid: Option<String> = row.get(3)?;
        let photos_name_snapshot: Option<String> = row.get(4)?;
        let photos_asset_id: Option<String> = row.get(5)?;
        let selected_face_index: Option<i64> = row.get(6)?;
        let source_sha256: Option<String> = row.get(8)?;
        let link = state.map(|state| PhotoLink {
            photos_person_uuid,
            photos_name_snapshot,
            photos_asset_id,
            selected_face_index: selected_face_index.and_then(|value| usize::try_from(value).ok()),
            selected_face_bounds: bounds
                .and_then(|value| serde_json::from_str::<BoundingBox>(&value).ok()),
            source_sha256,
            state,
        });
        Ok(PhotoReviewPerson {
            person_id: row.get(0)?,
            display_name: row.get(1)?,
            affinity_score: row.get(2)?,
            link,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub(crate) fn set_review_state(
    connection: &Connection,
    person_id: &str,
    state: &str,
) -> Result<()> {
    connection.execute(
        "INSERT INTO photo_links(person_id, state, reviewed_at)
         VALUES (?1, ?2, CURRENT_TIMESTAMP)
         ON CONFLICT(person_id) DO UPDATE SET
             photos_person_uuid = CASE WHEN excluded.state = 'not_applicable' THEN NULL ELSE photo_links.photos_person_uuid END,
             photos_name_snapshot = CASE WHEN excluded.state = 'not_applicable' THEN NULL ELSE photo_links.photos_name_snapshot END,
             photos_asset_id = CASE WHEN excluded.state = 'not_applicable' THEN NULL ELSE photo_links.photos_asset_id END,
             selected_face_index = CASE WHEN excluded.state = 'not_applicable' THEN NULL ELSE photo_links.selected_face_index END,
             selected_face_bounds_json = CASE WHEN excluded.state = 'not_applicable' THEN NULL ELSE photo_links.selected_face_bounds_json END,
             source_sha256 = CASE WHEN excluded.state = 'not_applicable' THEN NULL ELSE photo_links.source_sha256 END,
             state = excluded.state,
             reviewed_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP",
        params![person_id, state],
    )?;
    Ok(())
}

pub(crate) fn link_photos_person(
    connection: &Connection,
    person_id: &str,
    photos_person_uuid: &str,
    photos_name: &str,
    key_asset_id: Option<&str>,
) -> Result<()> {
    let existing_owner = connection
        .query_row(
            "SELECT p.id, p.display_name
             FROM photo_links l JOIN people p ON p.id = l.person_id
             WHERE l.photos_person_uuid = ?1 AND l.person_id <> ?2",
            params![photos_person_uuid, person_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    if let Some((_, owner_name)) = existing_owner {
        return Err(crate::error::CrmError::Photos(format!(
            "Photos person {photos_name} is already linked to CRM person {owner_name}"
        )));
    }
    connection.execute(
        "INSERT INTO photo_links(
             person_id, photos_person_uuid, photos_name_snapshot, photos_asset_id,
             state, reviewed_at
         ) VALUES (?1, ?2, ?3, ?4, 'person_linked', CURRENT_TIMESTAMP)
         ON CONFLICT(person_id) DO UPDATE SET
             photos_person_uuid = excluded.photos_person_uuid,
             photos_name_snapshot = excluded.photos_name_snapshot,
             photos_asset_id = COALESCE(photo_links.photos_asset_id, excluded.photos_asset_id),
             state = 'person_linked', reviewed_at = CURRENT_TIMESTAMP,
             updated_at = CURRENT_TIMESTAMP",
        params![person_id, photos_person_uuid, photos_name, key_asset_id],
    )?;
    Ok(())
}

pub(crate) fn link_asset(
    connection: &Connection,
    person_id: &str,
    asset_id: &str,
    face_index: usize,
    bounds: &BoundingBox,
    sha256: &str,
) -> Result<()> {
    let bounds = serde_json::to_string(bounds)
        .map_err(|error| crate::error::CrmError::Serialization(error.to_string()))?;
    connection.execute(
        "INSERT INTO photo_links(
             person_id, photos_asset_id, selected_face_index,
             selected_face_bounds_json, source_sha256, state, reviewed_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, 'asset_linked', CURRENT_TIMESTAMP)
         ON CONFLICT(person_id) DO UPDATE SET
             photos_person_uuid = NULL, photos_name_snapshot = NULL,
             photos_asset_id = excluded.photos_asset_id,
             selected_face_index = excluded.selected_face_index,
             selected_face_bounds_json = excluded.selected_face_bounds_json,
             source_sha256 = excluded.source_sha256, state = 'asset_linked',
             reviewed_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP",
        params![person_id, asset_id, face_index as i64, bounds, sha256],
    )?;
    Ok(())
}

pub(crate) fn asset_for_hash(connection: &Connection, sha256: &str) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT photos_asset_id FROM photo_links
             WHERE source_sha256 = ?1 AND photos_asset_id IS NOT NULL LIMIT 1",
            [sha256],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

pub(crate) fn status(connection: &Connection) -> Result<PhotoStatus> {
    let total_people: i64 =
        connection.query_row("SELECT COUNT(*) FROM people", [], |row| row.get(0))?;
    let count = |state: &str| -> Result<i64> {
        connection
            .query_row(
                "SELECT COUNT(*) FROM photo_links WHERE state = ?1",
                [state],
                |row| row.get(0),
            )
            .map_err(Into::into)
    };
    let deferred = count("deferred")?;
    let asset_linked = count("asset_linked")?;
    let person_linked = count("person_linked")?;
    let not_applicable = count("not_applicable")?;
    let stale = count("stale")?;
    let represented = deferred + asset_linked + person_linked + not_applicable + stale;
    Ok(PhotoStatus {
        total_people: usize::try_from(total_people).unwrap_or_default(),
        pending: usize::try_from(total_people - represented).unwrap_or_default(),
        deferred: usize::try_from(deferred).unwrap_or_default(),
        asset_linked: usize::try_from(asset_linked).unwrap_or_default(),
        person_linked: usize::try_from(person_linked).unwrap_or_default(),
        not_applicable: usize::try_from(not_applicable).unwrap_or_default(),
        stale: usize::try_from(stale).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn tracks_asset_then_person_link() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute(
                "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
                 VALUES ('p1', 'Ada', 'apple-1', 'active')",
                [],
            )
            .unwrap();
        let bounds = BoundingBox {
            x: 0.1,
            y: 0.2,
            width: 0.3,
            height: 0.4,
        };
        link_asset(&connection, "p1", "asset/L0/001", 2, &bounds, "abc").unwrap();
        assert_eq!(status(&connection).unwrap().asset_linked, 1);
        link_photos_person(&connection, "p1", "photos-person", "Ada", None).unwrap();
        let people = review_people(&connection, Some("p1")).unwrap();
        assert_eq!(people[0].link.as_ref().unwrap().state, "person_linked");
        assert_eq!(
            people[0].link.as_ref().unwrap().photos_asset_id.as_deref(),
            Some("asset/L0/001")
        );
    }

    #[test]
    fn allows_one_asset_for_multiple_people_but_not_one_photos_person() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute(
                "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
                 VALUES ('p1', 'Ada', 'apple-1', 'active'),
                        ('p2', 'Grace', 'apple-2', 'active')",
                [],
            )
            .unwrap();
        let bounds = BoundingBox {
            x: 0.0,
            y: 0.0,
            width: 0.2,
            height: 0.2,
        };
        link_asset(&connection, "p1", "shared", 1, &bounds, "hash").unwrap();
        link_asset(&connection, "p2", "shared", 2, &bounds, "hash").unwrap();
        link_photos_person(&connection, "p1", "photos-person", "Ada", None).unwrap();
        let duplicate = link_photos_person(&connection, "p2", "photos-person", "Ada", None);
        assert!(duplicate.is_err());
    }

    #[test]
    fn not_applicable_releases_existing_links() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute(
                "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
                 VALUES ('p1', 'Ada', 'apple-1', 'active')",
                [],
            )
            .unwrap();
        link_photos_person(&connection, "p1", "photos-person", "Ada", Some("asset")).unwrap();
        set_review_state(&connection, "p1", "not_applicable").unwrap();
        let link = review_people(&connection, Some("p1")).unwrap()[0]
            .link
            .clone()
            .unwrap();
        assert_eq!(link.state, "not_applicable");
        assert!(link.photos_person_uuid.is_none());
        assert!(link.photos_asset_id.is_none());
    }
}

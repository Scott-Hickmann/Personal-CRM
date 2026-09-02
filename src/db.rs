use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use crate::error::{CrmError, Result};

const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/001_initial.sql")),
    (2, include_str!("../migrations/002_overlay.sql")),
    (3, include_str!("../migrations/003_sync.sql")),
    (4, include_str!("../migrations/004_photos.sql")),
    (5, include_str!("../migrations/005_contact_publish.sql")),
    (6, include_str!("../migrations/006_authority.sql")),
    (7, include_str!("../migrations/007_participant_names.sql")),
    (
        8,
        include_str!("../migrations/008_excluded_icloud_contacts.sql"),
    ),
    (9, include_str!("../migrations/009_person_merges.sql")),
    (
        10,
        include_str!("../migrations/010_participant_person_index.sql"),
    ),
    (11, include_str!("../migrations/011_gmail_people_sync.sql")),
    (12, include_str!("../migrations/012_hybrid_affinity.sql")),
    (13, include_str!("../migrations/013_concurrent_jobs.sql")),
];

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| CrmError::Io {
            path: parent.to_owned(),
            source,
        })?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(|source| {
            CrmError::Io {
                path: parent.to_owned(),
                source,
            }
        })?;
    }
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
    migrate(&connection)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|source| {
        CrmError::Io {
            path: path.to_owned(),
            source,
        }
    })?;
    Ok(connection)
}

fn migrate(connection: &Connection) -> Result<()> {
    let current = connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_versions",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0);
    for (version, sql) in MIGRATIONS {
        if *version > current {
            connection.execute_batch(sql)?;
        }
    }
    Ok(())
}

pub fn schema_version(connection: &Connection) -> Result<i64> {
    connection
        .query_row(
            "SELECT version FROM schema_versions ORDER BY version DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("crm.sqlite3");
        drop(open(&path).unwrap());
        let connection = open(&path).unwrap();
        assert_eq!(schema_version(&connection).unwrap(), 13);
    }

    #[test]
    fn hybrid_affinity_migration_invalidates_legacy_scores_and_requeues_analysis() {
        let connection = Connection::open_in_memory().unwrap();
        for (version, sql) in MIGRATIONS.iter().filter(|(version, _)| *version <= 11) {
            connection.execute_batch(sql).unwrap();
            assert_eq!(schema_version(&connection).unwrap(), *version);
        }
        connection
            .execute_batch(
                "INSERT INTO sources(id, kind) VALUES ('source', 'test');
                 INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state, affinity_score)
                 VALUES ('person', 'Alex', 'apple-1', 'active', 99.0);
                 INSERT INTO interactions(id, source_id, native_id, channel, kind, occurred_at, body, analysis_state)
                 VALUES ('interaction', 'source', 'native', 'imessage', 'message', '2026-09-01', 'hello', 'complete');
                 INSERT INTO metrics(person_id, behavioral_score, semantic_score, components_json)
                 VALUES ('person', 90.0, 90.0, '{}');",
            )
            .unwrap();

        migrate(&connection).unwrap();

        let score: Option<f64> = connection
            .query_row(
                "SELECT affinity_score FROM people WHERE id='person'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let state: String = connection
            .query_row(
                "SELECT analysis_state FROM interactions WHERE id='interaction'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(schema_version(&connection).unwrap(), 13);
        assert_eq!(score, None);
        assert_eq!(state, "pending");
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM metrics", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn concurrent_jobs_migration_splits_legacy_communications_work() {
        let connection = Connection::open_in_memory().unwrap();
        for (_, sql) in MIGRATIONS.iter().filter(|(version, _)| *version <= 12) {
            connection.execute_batch(sql).unwrap();
        }
        connection
            .execute(
                "INSERT INTO jobs(kind, reason) VALUES ('communications', 'legacy')",
                [],
            )
            .unwrap();

        migrate(&connection).unwrap();

        let kinds: Vec<String> = connection
            .prepare("SELECT kind FROM jobs ORDER BY kind")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(
            kinds,
            ["apple_calls", "imessage", "whatsapp", "whatsapp_calls"]
        );
    }
}

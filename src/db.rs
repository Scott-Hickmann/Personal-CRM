use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, Transaction, TransactionBehavior};

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
    (
        14,
        include_str!("../migrations/014_contact_content_fingerprint.sql"),
    ),
    (
        15,
        include_str!("../migrations/015_structural_relationships.sql"),
    ),
    (
        16,
        include_str!("../migrations/016_conversation_titles.sql"),
    ),
    (
        18,
        include_str!("../migrations/018_deterministic_orchestration.sql"),
    ),
    (19, include_str!("../migrations/019_work_state_rows.sql")),
    (20, include_str!("../migrations/020_network_clusters.sql")),
    (21, include_str!("../migrations/021_contact_images.sql")),
];

const RELATIONSHIP_STRUCTURE_REVISION_MIGRATION: i64 = 17;

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
    backup_before_destructive_migration(&connection, path)?;
    migrate(&connection)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|source| {
        CrmError::Io {
            path: path.to_owned(),
            source,
        }
    })?;
    Ok(connection)
}

fn backup_before_destructive_migration(connection: &Connection, path: &Path) -> Result<()> {
    let current = connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_versions",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0);
    if current == 0 || current >= 18 || !path.exists() {
        return Ok(());
    }
    let backup = path.with_extension("pre-deterministic-v18.sqlite3");
    if backup.exists() {
        return Ok(());
    }
    connection.execute("VACUUM INTO ?1", [backup.to_string_lossy().as_ref()])?;
    fs::set_permissions(&backup, fs::Permissions::from_mode(0o600)).map_err(|source| {
        CrmError::Io {
            path: backup,
            source,
        }
    })?;
    Ok(())
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
    migrate_relationship_structure_revision(connection, current)?;
    Ok(())
}

fn migrate_relationship_structure_revision(connection: &Connection, current: i64) -> Result<()> {
    if current >= RELATIONSHIP_STRUCTURE_REVISION_MIGRATION {
        return Ok(());
    }
    let transaction = immediate_transaction(connection)?;
    let has_column = transaction
        .prepare("PRAGMA table_info(relationships)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .iter()
        .any(|column| column == "structure_revision");
    if !has_column {
        transaction.execute(
            "ALTER TABLE relationships ADD COLUMN structure_revision INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }
    transaction.execute(
        "INSERT OR IGNORE INTO schema_versions(version) VALUES (?1)",
        [RELATIONSHIP_STRUCTURE_REVISION_MIGRATION],
    )?;
    transaction.commit()?;
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

pub fn immediate_transaction(connection: &Connection) -> Result<Transaction<'_>> {
    Transaction::new_unchecked(connection, TransactionBehavior::Immediate).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;

    #[test]
    fn migration_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("crm.sqlite3");
        drop(open(&path).unwrap());
        let connection = open(&path).unwrap();
        assert_eq!(schema_version(&connection).unwrap(), 21);
    }

    #[test]
    fn destructive_migration_creates_a_consistent_backup() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("crm.sqlite3");
        let connection = Connection::open(&path).unwrap();
        for (_, sql) in MIGRATIONS.iter().filter(|(version, _)| *version <= 16) {
            connection.execute_batch(sql).unwrap();
        }
        connection
            .execute(
                "INSERT OR IGNORE INTO schema_versions(version) VALUES (17)",
                [],
            )
            .unwrap();
        drop(connection);

        let migrated = open(&path).unwrap();
        let backup_path = path.with_extension("pre-deterministic-v18.sqlite3");
        let backup = Connection::open(backup_path).unwrap();

        assert_eq!(schema_version(&migrated).unwrap(), 21);
        assert_eq!(schema_version(&backup).unwrap(), 17);
        let legacy_jobs: bool = backup
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='jobs')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(legacy_jobs);
    }

    #[test]
    fn immediate_transaction_waits_for_an_active_writer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("crm.sqlite3");
        let connection = open(&path).unwrap();
        let active = immediate_transaction(&connection).unwrap();
        active
            .execute(
                "INSERT INTO sources(id, kind) VALUES ('active', 'test')",
                [],
            )
            .unwrap();

        let (started_tx, started_rx) = mpsc::channel();
        let worker_path = path.clone();
        let worker = thread::spawn(move || {
            let waiting = open(&worker_path).unwrap();
            started_tx.send(()).unwrap();
            let transaction = immediate_transaction(&waiting).unwrap();
            transaction
                .execute(
                    "INSERT INTO sources(id, kind) VALUES ('waiting', 'test')",
                    [],
                )
                .unwrap();
            transaction.commit().unwrap();
        });
        started_rx.recv().unwrap();
        thread::sleep(Duration::from_millis(100));
        active.commit().unwrap();
        worker.join().unwrap();

        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM sources", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn deterministic_migration_removes_model_state_and_queues_scoring() {
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
        assert_eq!(schema_version(&connection).unwrap(), 21);
        assert_eq!(score, None);
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM metrics", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM dirty_people", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT state FROM maintenance_state WHERE kind='scoring'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "pending"
        );
    }

    #[test]
    fn structural_relationship_migration_backfills_shared_threads() {
        let connection = Connection::open_in_memory().unwrap();
        for (_, sql) in MIGRATIONS.iter().filter(|(version, _)| *version <= 14) {
            connection.execute_batch(sql).unwrap();
        }
        connection
            .execute_batch(
                "INSERT INTO sources(id, kind) VALUES ('gmail:test', 'gmail');
                 INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state) VALUES
                   ('a', 'Alex', 'apple-a', 'active'),
                   ('b', 'Blair', 'apple-b', 'active');
                 INSERT INTO interactions(
                   id, source_id, native_id, thread_native_id, channel, kind, occurred_at
                 ) VALUES ('message', 'gmail:test', 'message', 'thread', 'gmail', 'email',
                           '2026-01-01');
                 INSERT INTO interaction_participants(
                   interaction_id, person_id, identity_value, role
                 ) VALUES
                   ('message', 'a', 'alex@example.com', 'sender'),
                   ('message', 'b', 'blair@example.com', 'recipient');",
            )
            .unwrap();

        migrate(&connection).unwrap();

        let relationship: (String, String) = connection
            .query_row(
                "SELECT source_person_id, target_person_id FROM relationships",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(relationship, ("a".into(), "b".into()));
    }

    #[test]
    fn migration_repairs_missing_relationship_structure_revision() {
        let connection = Connection::open_in_memory().unwrap();
        for (_, sql) in MIGRATIONS.iter().filter(|(version, _)| *version <= 14) {
            connection.execute_batch(sql).unwrap();
        }
        let migration = MIGRATIONS
            .iter()
            .find(|(version, _)| *version == 15)
            .unwrap()
            .1
            .replace("    structure_revision INTEGER NOT NULL DEFAULT 1,\n", "");
        connection.execute_batch(&migration).unwrap();
        connection
            .execute_batch(
                MIGRATIONS
                    .iter()
                    .find(|(version, _)| *version == 16)
                    .unwrap()
                    .1,
            )
            .unwrap();

        migrate(&connection).unwrap();

        let columns: Vec<String> = connection
            .prepare("PRAGMA table_info(relationships)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert!(columns.iter().any(|column| column == "structure_revision"));
        assert_eq!(schema_version(&connection).unwrap(), 21);
    }

    #[test]
    fn deterministic_migration_removes_the_legacy_job_queue() {
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

        let jobs_exist: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='jobs')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!jobs_exist);
        assert_eq!(schema_version(&connection).unwrap(), 21);
    }
}

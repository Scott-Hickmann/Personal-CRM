use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

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
        assert_eq!(schema_version(&connection).unwrap(), 8);
    }
}

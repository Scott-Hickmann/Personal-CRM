use std::path::Path;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::{Connection, OpenFlags};

use crate::error::{CrmError, Result};

pub struct ReadOnlySource {
    connection: Connection,
}

impl ReadOnlySource {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.pragma_update(None, "query_only", true)?;
        connection.authorizer(Some(authorize_read_only))?;
        Ok(Self { connection })
    }

    pub fn require_columns(&self, table: &str, required: &[&str]) -> Result<()> {
        let columns = self.columns(table)?;
        let missing: Vec<_> = required
            .iter()
            .filter(|column| !columns.iter().any(|actual| actual == **column))
            .copied()
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(CrmError::IncompatibleSource(format!(
                "table {table} is missing columns: {}",
                missing.join(", ")
            )))
        }
    }

    pub fn has_columns(&self, table: &str, required: &[&str]) -> Result<bool> {
        let columns = self.columns(table)?;
        Ok(required
            .iter()
            .all(|column| columns.iter().any(|actual| actual == *column)))
    }

    fn columns(&self, table: &str) -> Result<Vec<String>> {
        let escaped = table.replace('"', "\"\"");
        let sql = format!("PRAGMA table_info(\"{escaped}\")");
        let mut statement = self.connection.prepare(&sql)?;
        statement
            .query_map([], |row| row.get(1))?
            .collect::<std::result::Result<_, _>>()
            .map_err(Into::into)
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    pub fn schema_fingerprint(&self) -> Result<String> {
        let mut statement = self.connection.prepare(
            "SELECT type, name, COALESCE(sql, '') FROM sqlite_master ORDER BY type, name",
        )?;
        let rows: Vec<(String, String, String)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut hasher = DefaultHasher::new();
        rows.hash(&mut hasher);
        Ok(format!("{:016x}", hasher.finish()))
    }
}

fn authorize_read_only(context: AuthContext<'_>) -> Authorization {
    match context.action {
        AuthAction::Select
        | AuthAction::Read { .. }
        | AuthAction::Function { .. }
        | AuthAction::Recursive
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. } => Authorization::Allow,
        AuthAction::Pragma { pragma_name, .. }
            if matches!(
                pragma_name.to_ascii_lowercase().as_str(),
                "table_info" | "table_xinfo" | "database_list" | "query_only"
            ) =>
        {
            Authorization::Allow
        }
        _ => Authorization::Deny,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denies_every_source_write() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.sqlite3");
        let writable = Connection::open(&path).unwrap();
        writable
            .execute_batch("CREATE TABLE items(value TEXT); INSERT INTO items VALUES ('safe');")
            .unwrap();
        drop(writable);

        let source = ReadOnlySource::open(&path).unwrap();
        assert_eq!(
            source
                .connection
                .query_row("SELECT value FROM items", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "safe"
        );
        for sql in [
            "INSERT INTO items VALUES ('bad')",
            "UPDATE items SET value = 'bad'",
            "DELETE FROM items",
            "DROP TABLE items",
            "PRAGMA user_version = 2",
            "ATTACH DATABASE ':memory:' AS other",
        ] {
            assert!(
                source.connection.execute_batch(sql).is_err(),
                "write unexpectedly succeeded: {sql}"
            );
        }
    }
}

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::SyncReport;
use crate::error::{CrmError, Result};
use crate::repository;
use crate::source::ReadOnlySource;

pub fn sync(config: &crate::config::Config, crm: &Connection) -> Result<SyncReport> {
    let configured = config
        .paths
        .contacts
        .as_ref()
        .ok_or_else(|| CrmError::InvalidConfig("contacts path is not configured".into()))?;
    let paths = contact_databases(configured)?;
    let mut imported = 0;
    let mut fingerprints = Vec::new();
    for path in paths {
        let source = ReadOnlySource::open(&path)?;
        source.require_columns(
            "ZABCDRECORD",
            &["Z_PK", "ZUNIQUEID", "ZFIRSTNAME", "ZLASTNAME"],
        )?;
        fingerprints.push(source.schema_fingerprint()?);
        imported += import_database(source.connection(), crm)?;
    }
    let fingerprint = fingerprints.join(":");
    crm.execute(
        "INSERT INTO sources(id, kind, schema_fingerprint, status, last_sync_at) VALUES ('contacts', 'contacts', ?1, 'ok', CURRENT_TIMESTAMP)
         ON CONFLICT(id) DO UPDATE SET schema_fingerprint=excluded.schema_fingerprint, status='ok', last_sync_at=CURRENT_TIMESTAMP, error=NULL",
        [fingerprint.as_str()],
    )?;
    Ok(SyncReport {
        source: "contacts".into(),
        imported,
        deleted: 0,
        schema_fingerprint: fingerprint,
    })
}

fn contact_databases(configured: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = vec![configured.to_owned()];
    let Some(root) = configured.parent() else {
        return Ok(paths);
    };
    let sources = root.join("Sources");
    if sources.exists() {
        for entry in fs::read_dir(&sources).map_err(|source| CrmError::Io {
            path: sources.clone(),
            source,
        })? {
            let path = entry
                .map_err(|source| CrmError::Io {
                    path: sources.clone(),
                    source,
                })?
                .path()
                .join("AddressBook-v22.abcddb");
            if path.exists() {
                paths.push(path);
            }
        }
    }
    Ok(paths)
}

fn import_database(source: &Connection, crm: &Connection) -> Result<usize> {
    let mut statement = source.prepare(
        "SELECT Z_PK, ZUNIQUEID, TRIM(COALESCE(ZFIRSTNAME, '') || ' ' || COALESCE(ZLASTNAME, '')), ZORGANIZATION
         FROM ZABCDRECORD WHERE ZUNIQUEID IS NOT NULL",
    )?;
    let contacts: Vec<(i64, String, String, Option<String>)> = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    for (owner, native_id, name, organization) in &contacts {
        let display_name = if name.is_empty() {
            organization.as_deref().unwrap_or("Unnamed contact")
        } else {
            name
        };
        let person_id = crm
            .query_row(
                "SELECT id FROM people WHERE apple_contact_id = ?1",
                [native_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        crm.execute(
            "INSERT INTO people(id, display_name, apple_contact_id) VALUES (?1, ?2, ?3)
             ON CONFLICT(apple_contact_id) DO UPDATE SET display_name=excluded.display_name, updated_at=CURRENT_TIMESTAMP",
            params![person_id, display_name, native_id],
        )?;
        import_identities(source, crm, *owner, &person_id)?;
    }
    Ok(contacts.len())
}

fn import_identities(
    source: &Connection,
    crm: &Connection,
    owner: i64,
    person_id: &str,
) -> Result<()> {
    for (table, column, kind) in [
        ("ZABCDEMAILADDRESS", "ZADDRESS", "email"),
        ("ZABCDPHONENUMBER", "ZFULLNUMBER", "phone"),
    ] {
        let sql =
            format!("SELECT {column} FROM {table} WHERE ZOWNER = ?1 AND {column} IS NOT NULL");
        let mut statement = source.prepare(&sql)?;
        let values: Vec<String> = statement
            .query_map([owner], |row| row.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        for value in values {
            repository::upsert_identity(crm, person_id, kind, &value, false)?;
        }
    }
    Ok(())
}

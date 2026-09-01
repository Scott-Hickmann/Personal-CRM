use rusqlite::{Connection, params};

use crate::error::Result;

#[derive(Debug, Clone)]
pub struct Mirror {
    pub apple_id: String,
    pub account: String,
    pub resource_name: String,
}

pub fn list(connection: &Connection) -> Result<Vec<Mirror>> {
    let mut statement = connection.prepare(
        "SELECT apple_contact_id, google_account, google_resource_name FROM contact_mirrors",
    )?;
    statement
        .query_map([], |row| {
            Ok(Mirror {
                apple_id: row.get(0)?,
                account: row.get(1)?,
                resource_name: row.get(2)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()
        .map_err(Into::into)
}

pub fn upsert(
    connection: &Connection,
    apple_id: &str,
    account: &str,
    resource_name: &str,
    etag: Option<&str>,
    content_hash: &str,
) -> Result<()> {
    connection.execute(
        "INSERT INTO contact_mirrors(apple_contact_id, google_account, google_resource_name, google_etag, content_hash)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(apple_contact_id, google_account) DO UPDATE SET
            google_resource_name=excluded.google_resource_name,
            google_etag=excluded.google_etag,
            content_hash=excluded.content_hash,
            updated_at=CURRENT_TIMESTAMP",
        params![apple_id, account, resource_name, etag, content_hash],
    )?;
    Ok(())
}

pub fn remove(connection: &Connection, apple_id: &str, account: &str) -> Result<()> {
    connection.execute(
        "DELETE FROM contact_mirrors WHERE apple_contact_id = ?1 AND google_account = ?2",
        params![apple_id, account],
    )?;
    Ok(())
}

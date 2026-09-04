use std::{fs, path::Path};

use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{error::Result, source::ReadOnlySource};

// Core Data stores external binary values as 0x02 + UUID + NUL.
fn image_bytes(root: &Path, value: &[u8]) -> Option<Vec<u8>> {
    if value.first() == Some(&2) {
        let name = std::str::from_utf8(value.get(1..value.len().checked_sub(1)?)?).ok()?;
        if value.last() != Some(&0) || uuid::Uuid::parse_str(name).is_err() {
            return None;
        }
        fs::read(
            root.join(".AddressBook-v22_SUPPORT/_EXTERNAL_DATA")
                .join(name),
        )
        .ok()
    } else {
        Some(value.strip_prefix(&[1]).unwrap_or(value).to_vec())
    }
}

fn mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else {
        None
    }
}

pub fn sync(crm: &Connection, database: &Path) -> Result<bool> {
    let source = ReadOnlySource::open(database)?;
    source.require_columns(
        "ZABCDRECORD",
        &["ZUNIQUEID", "ZTHUMBNAILIMAGEDATA", "ZIMAGEDATA"],
    )?;
    let mut statement = source.connection().prepare(
        "SELECT ZUNIQUEID, ZTHUMBNAILIMAGEDATA, ZIMAGEDATA FROM ZABCDRECORD WHERE ZUNIQUEID IS NOT NULL")?;
    let mut rows = statement.query([])?;
    let mut images = Vec::new();
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        for index in [1, 2] {
            let value: Option<Vec<u8>> = row.get(index)?;
            if let Some(bytes) =
                value.and_then(|value| image_bytes(database.parent().unwrap(), &value))
                && let Some(kind) = mime(&bytes)
            {
                let version = format!("{:x}", Sha256::digest(&bytes));
                images.push((id, version, kind, bytes));
                break;
            }
        }
    }
    let transaction = crm.unchecked_transaction()?;
    transaction.execute_batch("CREATE TEMP TABLE current_contact_images(id TEXT PRIMARY KEY)")?;
    let mut changed = false;
    for (id, version, kind, bytes) in images {
        transaction.execute("INSERT INTO current_contact_images VALUES (?1)", [&id])?;
        changed |= transaction.execute(
            "INSERT INTO contact_images(apple_contact_id, version, mime_type, data) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(apple_contact_id) DO UPDATE SET version=excluded.version, mime_type=excluded.mime_type, data=excluded.data
             WHERE contact_images.version != excluded.version", params![id, version, kind, bytes])? > 0;
    }
    changed |= transaction.execute("DELETE FROM contact_images WHERE apple_contact_id NOT IN (SELECT id FROM current_contact_images)", [])? > 0;
    transaction.execute_batch("DROP TABLE current_contact_images")?;
    transaction.commit()?;
    Ok(changed)
}

pub fn version(connection: &Connection, person: &str) -> Result<Option<String>> {
    Ok(connection.query_row("SELECT c.version FROM contact_images c JOIN people p ON p.apple_contact_id=c.apple_contact_id WHERE p.id=?1", [person], |row| row.get(0)).optional()?)
}

#[derive(Serialize)]
pub struct Image {
    pub version: String,
    pub mime_type: String,
    pub data: String,
}

pub fn load(connection: &Connection, person: &str) -> Result<Option<Image>> {
    Ok(connection.query_row("SELECT c.version, c.mime_type, c.data FROM contact_images c JOIN people p ON p.apple_contact_id=c.apple_contact_id WHERE p.id=?1", [person], |row| {
        Ok(Image { version: row.get(0)?, mime_type: row.get(1)?, data: STANDARD.encode(row.get::<_, Vec<u8>>(2)?) })
    }).optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_backfills_updates_and_removes_external_images() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("AddressBook-v22.abcddb");
        let source = Connection::open(&path).unwrap();
        source.execute_batch("CREATE TABLE ZABCDRECORD(ZUNIQUEID TEXT, ZTHUMBNAILIMAGEDATA BLOB, ZIMAGEDATA BLOB)").unwrap();
        let external = directory
            .path()
            .join(".AddressBook-v22_SUPPORT/_EXTERNAL_DATA");
        fs::create_dir_all(&external).unwrap();
        let id = "388B2F33-11F2-4954-8EBA-CFB5F221A2B9";
        let mut reference = vec![2];
        reference.extend(id.as_bytes());
        reference.push(0);
        fs::write(external.join(id), [255, 216, 255, 1]).unwrap();
        source
            .execute(
                "INSERT INTO ZABCDRECORD VALUES ('apple', ?1, NULL)",
                [reference],
            )
            .unwrap();
        let crm = crate::db::open(&directory.path().join("crm.db")).unwrap();
        crm.execute("INSERT INTO people(id, display_name, apple_contact_id) VALUES ('person', 'Test', 'apple')", []).unwrap();
        assert!(sync(&crm, &path).unwrap());
        let first = load(&crm, "person").unwrap().unwrap();
        assert_eq!(first.mime_type, "image/jpeg");
        assert!(!sync(&crm, &path).unwrap());
        fs::write(external.join(id), [255, 216, 255, 2]).unwrap();
        assert!(sync(&crm, &path).unwrap());
        assert_ne!(version(&crm, "person").unwrap().unwrap(), first.version);
        source
            .execute("UPDATE ZABCDRECORD SET ZTHUMBNAILIMAGEDATA=NULL", [])
            .unwrap();
        assert!(sync(&crm, &path).unwrap());
        assert!(load(&crm, "person").unwrap().is_none());
        assert!(load(&crm, "missing").unwrap().is_none());
    }

    #[test]
    fn reads_inline_images_and_rejects_invalid_references() {
        let root = Path::new("/unused");
        assert_eq!(
            image_bytes(root, &[1, 255, 216, 255]).unwrap(),
            [255, 216, 255]
        );
        assert!(image_bytes(root, b"\x02../secret\0").is_none());
        assert!(mime(b"invalid").is_none());
    }
}

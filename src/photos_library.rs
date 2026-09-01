use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{CrmError, Result};
use crate::source::ReadOnlySource;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NamedPhotosPerson {
    pub person_uuid: String,
    pub name: String,
    pub key_asset_id: Option<String>,
}

pub(crate) struct PhotosCatalog {
    source: ReadOnlySource,
}

impl PhotosCatalog {
    pub(crate) fn open(library: &Path) -> Result<Self> {
        let database = library.join("database/Photos.sqlite");
        if !database.is_file() {
            return Err(CrmError::PhotoFaceMatching(format!(
                "Photos database not found at {}",
                database.display()
            )));
        }
        let source = ReadOnlySource::open(&database)?;
        source.require_columns(
            "ZPERSON",
            &["Z_PK", "ZDISPLAYNAME", "ZPERSONUUID", "ZKEYFACE"],
        )?;
        source.require_columns(
            "ZDETECTEDFACE",
            &["Z_PK", "ZPERSONFORFACE", "ZASSETFORFACE"],
        )?;
        source.require_columns("ZASSET", &["Z_PK", "ZUUID"])?;
        Ok(Self { source })
    }

    pub(crate) fn named_people(&self) -> Result<Vec<NamedPhotosPerson>> {
        let mut statement = self.source.connection().prepare(
            "SELECT p.ZPERSONUUID, p.ZDISPLAYNAME, a.ZUUID
             FROM ZPERSON p
             LEFT JOIN ZDETECTEDFACE f ON f.Z_PK = p.ZKEYFACE
             LEFT JOIN ZASSET a ON a.Z_PK = f.ZASSETFORFACE
             WHERE p.ZPERSONUUID IS NOT NULL
               AND COALESCE(TRIM(p.ZDISPLAYNAME), '') <> ''
             ORDER BY p.ZDISPLAYNAME COLLATE NOCASE, p.ZPERSONUUID",
        )?;
        let rows = statement.query_map([], map_person)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub(crate) fn named_people_for_asset(&self, asset_id: &str) -> Result<Vec<NamedPhotosPerson>> {
        let asset_uuid = asset_uuid(asset_id);
        let mut statement = self.source.connection().prepare(
            "SELECT DISTINCT p.ZPERSONUUID, p.ZDISPLAYNAME, key_asset.ZUUID
             FROM ZASSET a
             JOIN ZDETECTEDFACE f ON f.ZASSETFORFACE = a.Z_PK
             JOIN ZPERSON p ON p.Z_PK = f.ZPERSONFORFACE
             LEFT JOIN ZDETECTEDFACE key_face ON key_face.Z_PK = p.ZKEYFACE
             LEFT JOIN ZASSET key_asset ON key_asset.Z_PK = key_face.ZASSETFORFACE
             WHERE a.ZUUID = ?1
               AND p.ZPERSONUUID IS NOT NULL
               AND COALESCE(TRIM(p.ZDISPLAYNAME), '') <> ''
             ORDER BY p.ZDISPLAYNAME COLLATE NOCASE, p.ZPERSONUUID",
        )?;
        let rows = statement.query_map([asset_uuid], map_person)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

fn map_person(row: &rusqlite::Row<'_>) -> rusqlite::Result<NamedPhotosPerson> {
    Ok(NamedPhotosPerson {
        person_uuid: row.get(0)?,
        name: row.get(1)?,
        key_asset_id: row.get(2)?,
    })
}

pub(crate) fn discover_library(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(library) = explicit {
        return Ok(library);
    }
    let pictures = dirs::picture_dir().ok_or_else(|| {
        CrmError::PhotoFaceMatching("cannot determine the Pictures directory".into())
    })?;
    let conventional = pictures.join("Photos Library.photoslibrary");
    if conventional.join("database/Photos.sqlite").is_file() {
        return Ok(conventional);
    }
    let entries = std::fs::read_dir(&pictures).map_err(|source| CrmError::Io {
        path: pictures.clone(),
        source,
    })?;
    let mut libraries = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|value| value == "photoslibrary")
                && path.join("database/Photos.sqlite").is_file()
        })
        .collect::<Vec<_>>();
    libraries.sort();
    match libraries.as_slice() {
        [library] => Ok(library.clone()),
        [] => Err(CrmError::PhotoFaceMatching(format!(
            "no Photos library found in {}; pass --library PATH",
            pictures.display()
        ))),
        _ => Err(CrmError::PhotoFaceMatching(format!(
            "multiple Photos libraries found in {}; pass --library PATH",
            pictures.display()
        ))),
    }
}

fn asset_uuid(local_identifier: &str) -> &str {
    local_identifier
        .split('/')
        .next()
        .unwrap_or(local_identifier)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    #[test]
    fn finds_named_people_attached_to_an_asset() {
        let directory = tempfile::tempdir().unwrap();
        let library = directory.path().join("Library.photoslibrary");
        std::fs::create_dir_all(library.join("database")).unwrap();
        let connection = Connection::open(library.join("database/Photos.sqlite")).unwrap();
        connection.execute_batch(
            "CREATE TABLE ZPERSON(Z_PK INTEGER PRIMARY KEY, ZDISPLAYNAME TEXT, ZPERSONUUID TEXT, ZKEYFACE INTEGER);
             CREATE TABLE ZDETECTEDFACE(Z_PK INTEGER PRIMARY KEY, ZPERSONFORFACE INTEGER, ZASSETFORFACE INTEGER);
             CREATE TABLE ZASSET(Z_PK INTEGER PRIMARY KEY, ZUUID TEXT);
             INSERT INTO ZASSET VALUES (1, 'asset-uuid');
             INSERT INTO ZPERSON VALUES (1, 'Ada', 'person-uuid', 1);
             INSERT INTO ZDETECTEDFACE VALUES (1, 1, 1);",
        ).unwrap();
        drop(connection);
        let catalog = PhotosCatalog::open(&library).unwrap();
        let people = catalog.named_people_for_asset("asset-uuid/L0/001").unwrap();
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].name, "Ada");
        assert_eq!(
            catalog.named_people().unwrap()[0].key_asset_id.as_deref(),
            Some("asset-uuid")
        );
    }
}

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::error::{CrmError, Result};
use crate::source::ReadOnlySource;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppleContainer {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppleContact {
    pub id: String,
    pub name_prefix: String,
    pub given_name: String,
    pub middle_name: String,
    pub family_name: String,
    pub name_suffix: String,
    pub nickname: String,
    pub emails: Vec<LabeledValue>,
    pub phones: Vec<LabeledValue>,
    pub organization: String,
    pub department: String,
    pub job_title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabeledValue {
    pub label: Option<String>,
    pub value: String,
}

pub fn containers(configured: &Path) -> Result<Vec<AppleContainer>> {
    let accounts = account_metadata()?;
    let mut output = Vec::new();
    for (id, _) in database_paths(configured)? {
        let (name, kind) = if id == "local" {
            ("On My Mac".into(), "local".into())
        } else if let Some(account) = accounts.get(&id) {
            (account.name.clone(), account.kind.clone())
        } else {
            ("Unknown Contacts account".into(), "unknown".into())
        };
        output.push(AppleContainer { id, name, kind });
    }
    output.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    Ok(output)
}

pub fn contacts(configured: &Path, container_id: &str) -> Result<Vec<AppleContact>> {
    let path = container_path(configured, container_id)?;
    let source = ReadOnlySource::open(&path)?;
    source.require_columns(
        "ZABCDRECORD",
        &[
            "Z_PK",
            "ZUNIQUEID",
            "ZTITLE",
            "ZFIRSTNAME",
            "ZMIDDLENAME",
            "ZLASTNAME",
            "ZSUFFIX",
            "ZNICKNAME",
            "ZORGANIZATION",
            "ZDEPARTMENT",
            "ZJOBTITLE",
        ],
    )?;
    source.require_columns(
        "ZABCDEMAILADDRESS",
        &["ZOWNER", "ZADDRESS", "ZLABEL", "ZORDERINGINDEX"],
    )?;
    source.require_columns(
        "ZABCDPHONENUMBER",
        &["ZOWNER", "ZFULLNUMBER", "ZLABEL", "ZORDERINGINDEX"],
    )?;
    let connection = source.connection();
    let mut statement = connection.prepare(
        "SELECT Z_PK, ZUNIQUEID, COALESCE(ZTITLE, ''), COALESCE(ZFIRSTNAME, ''),
                COALESCE(ZMIDDLENAME, ''), COALESCE(ZLASTNAME, ''),
                COALESCE(ZSUFFIX, ''), COALESCE(ZNICKNAME, ''),
                COALESCE(ZORGANIZATION, ''), COALESCE(ZDEPARTMENT, ''),
                COALESCE(ZJOBTITLE, '')
         FROM ZABCDRECORD WHERE ZUNIQUEID IS NOT NULL ORDER BY ZUNIQUEID",
    )?;
    let rows: Vec<_> = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                AppleContact {
                    id: row.get(1)?,
                    name_prefix: row.get(2)?,
                    given_name: row.get(3)?,
                    middle_name: row.get(4)?,
                    family_name: row.get(5)?,
                    name_suffix: row.get(6)?,
                    nickname: row.get(7)?,
                    organization: row.get(8)?,
                    department: row.get(9)?,
                    job_title: row.get(10)?,
                    emails: Vec::new(),
                    phones: Vec::new(),
                },
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;
    rows.into_iter()
        .map(|(owner, mut contact)| {
            contact.emails = labeled_values(connection, "ZABCDEMAILADDRESS", "ZADDRESS", owner)?;
            contact.phones = labeled_values(connection, "ZABCDPHONENUMBER", "ZFULLNUMBER", owner)?;
            Ok(contact)
        })
        .collect()
}

pub fn container_path(configured: &Path, container_id: &str) -> Result<PathBuf> {
    database_paths(configured)?
        .into_iter()
        .find(|(id, _)| id == container_id)
        .map(|(_, path)| path)
        .ok_or_else(|| {
            CrmError::Contacts(format!(
                "configured contact container {container_id} was not found"
            ))
        })
}

pub fn schema_fingerprint(configured: &Path, container_id: &str) -> Result<String> {
    let source = ReadOnlySource::open(&container_path(configured, container_id)?)?;
    source.schema_fingerprint()
}

fn labeled_values(
    connection: &rusqlite::Connection,
    table: &str,
    value_column: &str,
    owner: i64,
) -> Result<Vec<LabeledValue>> {
    let sql = format!(
        "SELECT ZLABEL, {value_column} FROM {table}
         WHERE ZOWNER = ?1 AND {value_column} IS NOT NULL
         ORDER BY ZORDERINGINDEX, Z_PK"
    );
    let mut statement = connection.prepare(&sql)?;
    statement
        .query_map(params![owner], |row| {
            Ok(LabeledValue {
                label: row.get(0)?,
                value: row.get(1)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()
        .map_err(Into::into)
}

fn database_paths(configured: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut paths = vec![("local".into(), configured.to_owned())];
    let Some(root) = configured.parent() else {
        return Ok(paths);
    };
    let sources = root.join("Sources");
    if !sources.exists() {
        return Ok(paths);
    }
    for entry in fs::read_dir(&sources).map_err(|source| CrmError::Io {
        path: sources.clone(),
        source,
    })? {
        let directory = entry
            .map_err(|source| CrmError::Io {
                path: sources.clone(),
                source,
            })?
            .path();
        let database = directory.join("AddressBook-v22.abcddb");
        if database.exists()
            && let Some(id) = directory.file_name().and_then(|name| name.to_str())
        {
            paths.push((id.into(), database));
        }
    }
    Ok(paths)
}

#[derive(Debug)]
struct AccountMetadata {
    name: String,
    kind: String,
}

fn account_metadata() -> Result<HashMap<String, AccountMetadata>> {
    let home = dirs::home_dir()
        .ok_or_else(|| CrmError::Contacts("cannot determine home directory".into()))?;
    let path = home.join("Library/Accounts/Accounts4.sqlite");
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let source = ReadOnlySource::open(&path)?;
    source.require_columns(
        "ZACCOUNT",
        &[
            "ZACCOUNTTYPE",
            "ZACCOUNTDESCRIPTION",
            "ZIDENTIFIER",
            "ZUSERNAME",
        ],
    )?;
    source.require_columns("ZACCOUNTTYPE", &["Z_PK", "ZIDENTIFIER"])?;
    let mut statement = source.connection().prepare(
        "SELECT a.ZIDENTIFIER,
                COALESCE(NULLIF(a.ZACCOUNTDESCRIPTION, ''), NULLIF(a.ZUSERNAME, ''), a.ZIDENTIFIER),
                t.ZIDENTIFIER
         FROM ZACCOUNT a JOIN ZACCOUNTTYPE t ON t.Z_PK = a.ZACCOUNTTYPE
         WHERE a.ZIDENTIFIER IS NOT NULL",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            AccountMetadata {
                name: row.get(1)?,
                kind: row.get(2)?,
            },
        ))
    })?;
    rows.collect::<std::result::Result<_, _>>()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_labeled_contact_fields_from_read_only_database() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("AddressBook-v22.abcddb");
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE ZABCDRECORD (
                    Z_PK INTEGER PRIMARY KEY, ZUNIQUEID TEXT, ZTITLE TEXT,
                    ZFIRSTNAME TEXT, ZMIDDLENAME TEXT, ZLASTNAME TEXT, ZSUFFIX TEXT,
                    ZNICKNAME TEXT, ZORGANIZATION TEXT, ZDEPARTMENT TEXT, ZJOBTITLE TEXT
                 );
                 CREATE TABLE ZABCDEMAILADDRESS (
                    Z_PK INTEGER PRIMARY KEY, ZOWNER INTEGER, ZADDRESS TEXT,
                    ZLABEL TEXT, ZORDERINGINDEX INTEGER
                 );
                 CREATE TABLE ZABCDPHONENUMBER (
                    Z_PK INTEGER PRIMARY KEY, ZOWNER INTEGER, ZFULLNUMBER TEXT,
                    ZLABEL TEXT, ZORDERINGINDEX INTEGER
                 );
                 INSERT INTO ZABCDRECORD VALUES
                    (1, 'apple-1', '', 'Alex', '', 'Example', '', '', 'Example Inc', 'R&D', 'Engineer');
                 INSERT INTO ZABCDEMAILADDRESS VALUES
                    (1, 1, 'alex@example.com', '$!<Work>!$', 0);
                 INSERT INTO ZABCDPHONENUMBER VALUES
                    (1, 1, '555-0100', '$!<Mobile>!$', 0);",
            )
            .unwrap();
        drop(connection);

        let exported = contacts(&path, "local").unwrap();
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0].id, "apple-1");
        assert_eq!(exported[0].emails[0].label.as_deref(), Some("$!<Work>!$"));
        assert_eq!(exported[0].phones[0].value, "555-0100");
    }
}

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rusqlite::params;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{CrmError, Result};
use crate::source::ReadOnlySource;

const SHOW_AS_MASK: i64 = 7;
const SHOW_AS_COMPANY: i64 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppleContainer {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
}

pub fn is_icloud(container: &AppleContainer) -> bool {
    container.name.eq_ignore_ascii_case("icloud")
        || container.kind.to_lowercase().contains("icloud")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppleContact {
    pub id: String,
    pub is_company: bool,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewContact {
    pub container_id: String,
    pub display_name: String,
    pub emails: Vec<LabeledValue>,
    pub phones: Vec<LabeledValue>,
    pub organization: String,
}

pub fn create(contact: &NewContact) -> Result<String> {
    const HELPER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/create-contact.swift");
    let mut child = Command::new("xcrun")
        .args(["swift", HELPER])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| CrmError::Contacts(format!("could not start Contacts helper: {error}")))?;
    use std::io::Write;
    let input =
        serde_json::to_vec(contact).map_err(|error| CrmError::Serialization(error.to_string()))?;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&input)
        .map_err(|error| CrmError::Contacts(format!("could not send contact data: {error}")))?;
    let response = child
        .wait_with_output()
        .map_err(|error| CrmError::Contacts(format!("Contacts helper failed: {error}")))?;
    if !response.status.success() {
        return Err(CrmError::Contacts(
            String::from_utf8_lossy(&response.stderr).trim().to_owned(),
        ));
    }
    let id = String::from_utf8_lossy(&response.stdout).trim().to_owned();
    if id.is_empty() {
        Err(CrmError::Contacts(
            "Contacts helper returned no identifier".into(),
        ))
    } else {
        Ok(id)
    }
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
            "ZDISPLAYFLAGS",
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
        "SELECT Z_PK, ZUNIQUEID, COALESCE(ZDISPLAYFLAGS, 0),
                COALESCE(ZTITLE, ''), COALESCE(ZFIRSTNAME, ''),
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
                    is_company: row.get::<_, i64>(2)? & SHOW_AS_MASK == SHOW_AS_COMPANY,
                    name_prefix: row.get(3)?,
                    given_name: row.get(4)?,
                    middle_name: row.get(5)?,
                    family_name: row.get(6)?,
                    name_suffix: row.get(7)?,
                    nickname: row.get(8)?,
                    organization: row.get(9)?,
                    department: row.get(10)?,
                    job_title: row.get(11)?,
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

pub fn change_token(configured: &Path, container_id: &str) -> Result<Option<String>> {
    let source = ReadOnlySource::open(&container_path(configured, container_id)?)?;
    for table in ["ZABCDRECORD", "ZABCDEMAILADDRESS", "ZABCDPHONENUMBER"] {
        if !source.has_columns(table, &["Z_PK", "Z_OPT"])? {
            return Ok(None);
        }
    }
    let mut digest = Sha256::new();
    for (table, owner) in [
        ("ZABCDRECORD", None),
        ("ZABCDEMAILADDRESS", Some("ZOWNER")),
        ("ZABCDPHONENUMBER", Some("ZOWNER")),
    ] {
        let owner = owner
            .map(|column| format!(", COALESCE({column}, 0)"))
            .unwrap_or_default();
        let sql = format!("SELECT Z_PK, COALESCE(Z_OPT, 0){owner} FROM {table} ORDER BY Z_PK");
        let mut statement = source.connection().prepare(&sql)?;
        let columns = if owner.is_empty() { 2 } else { 3 };
        let mut rows = statement.query([])?;
        digest.update(table.as_bytes());
        while let Some(row) = rows.next()? {
            for index in 0..columns {
                digest.update(row.get::<_, i64>(index)?.to_le_bytes());
            }
        }
    }
    Ok(Some(format!("{:x}", digest.finalize())))
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
mod tests;

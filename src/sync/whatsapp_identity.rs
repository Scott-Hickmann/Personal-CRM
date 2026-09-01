use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::error::{CrmError, Result};
use crate::source::ReadOnlySource;

pub(super) struct LidResolver {
    phones: HashMap<String, String>,
}

impl LidResolver {
    pub(super) fn load(chat_storage_path: &Path) -> Result<Self> {
        let directory = chat_storage_path.parent().ok_or_else(|| {
            CrmError::InvalidConfig("WhatsApp database path has no parent directory".into())
        })?;
        let mut phones = HashMap::new();
        let mut ambiguous = HashSet::new();
        load_pairs(
            &directory.join("ContactsV2.sqlite"),
            "ZWAADDRESSBOOKCONTACT",
            &mut phones,
            &mut ambiguous,
        )?;
        load_pairs(
            &directory.join("LID.sqlite"),
            "ZWAPHONENUMBERLIDPAIR",
            &mut phones,
            &mut ambiguous,
        )?;
        Ok(Self { phones })
    }

    pub(super) fn resolve<'a>(&'a self, identity: &'a str) -> &'a str {
        self.phones
            .get(&identity.trim().to_ascii_lowercase())
            .map(String::as_str)
            .unwrap_or(identity)
    }
}

fn load_pairs(
    path: &Path,
    table: &str,
    phones: &mut HashMap<String, String>,
    ambiguous: &mut HashSet<String>,
) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let source = ReadOnlySource::open(path)?;
    source.require_columns(table, &["ZLID", "ZPHONENUMBER"])?;
    let sql = format!(
        "SELECT ZLID, ZPHONENUMBER FROM {table}
         WHERE ZLID IS NOT NULL AND trim(ZLID) != ''
           AND ZPHONENUMBER IS NOT NULL AND trim(ZPHONENUMBER) != ''"
    );
    let mut statement = source.connection().prepare(&sql)?;
    let rows: Vec<(String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    for (lid, phone) in rows {
        insert_pair(phones, ambiguous, lid, phone);
    }
    Ok(())
}

fn insert_pair(
    phones: &mut HashMap<String, String>,
    ambiguous: &mut HashSet<String>,
    lid: String,
    phone: String,
) {
    let lid = lid.trim().to_ascii_lowercase();
    let phone = phone.trim().to_owned();
    if ambiguous.contains(&lid) {
        return;
    }
    if let Some(existing) = phones.get(&lid)
        && crate::repository::normalize_observed_identity(existing)
            != crate::repository::normalize_observed_identity(&phone)
    {
        phones.remove(&lid);
        ambiguous.insert(lid);
        return;
    }
    phones.insert(lid, phone);
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn database(path: &Path, table: &str, lid: &str, phone: &str) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(&format!(
                "CREATE TABLE {table}(ZLID TEXT, ZPHONENUMBER TEXT);"
            ))
            .unwrap();
        connection
            .execute(
                &format!("INSERT INTO {table}(ZLID, ZPHONENUMBER) VALUES (?1, ?2)"),
                [lid, phone],
            )
            .unwrap();
    }

    #[test]
    fn resolves_lid_from_whatsapp_contacts() {
        let directory = tempfile::tempdir().unwrap();
        database(
            &directory.path().join("ContactsV2.sqlite"),
            "ZWAADDRESSBOOKCONTACT",
            "2207730634782@lid",
            "+14153284536",
        );

        let resolver = LidResolver::load(&directory.path().join("ChatStorage.sqlite")).unwrap();

        assert_eq!(resolver.resolve("2207730634782@lid"), "+14153284536");
        assert_eq!(resolver.resolve("other@lid"), "other@lid");
    }

    #[test]
    fn conflicting_mappings_are_not_resolved() {
        let directory = tempfile::tempdir().unwrap();
        database(
            &directory.path().join("ContactsV2.sqlite"),
            "ZWAADDRESSBOOKCONTACT",
            "123@lid",
            "+15550100",
        );
        database(
            &directory.path().join("LID.sqlite"),
            "ZWAPHONENUMBERLIDPAIR",
            "123@lid",
            "+15550200",
        );

        let resolver = LidResolver::load(&directory.path().join("ChatStorage.sqlite")).unwrap();

        assert_eq!(resolver.resolve("123@lid"), "123@lid");
    }
}

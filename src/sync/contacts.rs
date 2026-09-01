use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::{SyncReport, rebind_unresolved_participants};
use crate::contact_publish::apple::{self, AppleContact};
use crate::error::{CrmError, Result};
use crate::{repository, review};

pub fn sync(config: &crate::config::Config, crm: &Connection) -> Result<SyncReport> {
    let configured = config
        .paths
        .contacts
        .as_deref()
        .ok_or_else(|| CrmError::InvalidConfig("contacts path is not configured".into()))?;
    let container = config
        .contact_publish
        .source_container
        .as_deref()
        .ok_or_else(|| {
            CrmError::Contacts("select the authoritative iCloud container first".into())
        })?;
    let contacts = apple::contacts(configured, container)?;
    let fingerprint = apple::schema_fingerprint(configured, container)?;
    let conflicts = duplicate_identities(&contacts);
    enqueue_collisions(crm, &conflicts)?;

    let seen: HashSet<_> = contacts.iter().map(|contact| contact.id.as_str()).collect();
    let mut imported = 0;
    for contact in &contacts {
        if reconcile_contact(crm, contact, &conflicts)? {
            imported += 1;
        }
    }
    retire_missing(crm, &seen)?;
    enqueue_migration_reviews(crm, &contacts)?;
    rebind_unresolved_participants(crm)?;
    review::enqueue_unresolved_candidates(crm)?;
    crm.execute(
        "INSERT INTO sources(id, kind, account, schema_fingerprint, status, last_sync_at, last_reconcile_at)
         VALUES ('contacts', 'contacts', ?1, ?2, 'ok', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
         ON CONFLICT(id) DO UPDATE SET account=excluded.account,
         schema_fingerprint=excluded.schema_fingerprint, status='ok', error=NULL,
         last_sync_at=CURRENT_TIMESTAMP, last_reconcile_at=CURRENT_TIMESTAMP",
        params![container, fingerprint],
    )?;
    Ok(SyncReport {
        source: "contacts".into(),
        imported,
        deleted: 0,
        schema_fingerprint: fingerprint,
    })
}

fn reconcile_contact(
    crm: &Connection,
    contact: &AppleContact,
    conflicts: &HashMap<String, Vec<String>>,
) -> Result<bool> {
    let existing: Option<String> = crm
        .query_row(
            "SELECT id FROM people WHERE apple_contact_id=?1",
            [&contact.id],
            |row| row.get(0),
        )
        .optional()?;
    let person_id = if let Some(id) = existing {
        id
    } else if let Some(candidate) = migration_candidate(crm, contact)? {
        review::enqueue(
            crm,
            "migration_person",
            &candidate,
            &format!("Link an existing CRM person to {}?", display_name(contact)),
            serde_json::json!({"suggested_apple_contact_id": contact.id, "display_name": display_name(contact)}),
        )?;
        return Ok(false);
    } else {
        let id = Uuid::new_v4().to_string();
        crm.execute(
            "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state, last_contact_sync_at)
             VALUES (?1, ?2, ?3, 'active', CURRENT_TIMESTAMP)",
            params![id, display_name(contact), contact.id],
        )?;
        id
    };
    crm.execute(
        "UPDATE people SET display_name=?2, lifecycle_state='active', retired_at=NULL,
         last_contact_sync_at=CURRENT_TIMESTAMP, updated_at=CURRENT_TIMESTAMP WHERE id=?1",
        params![person_id, display_name(contact)],
    )?;
    crm.execute(
        "UPDATE identities SET active=0 WHERE person_id=?1 AND is_self=0",
        [&person_id],
    )?;
    let is_self = crm.query_row(
        "SELECT EXISTS(SELECT 1 FROM identities WHERE person_id=?1 AND is_self=1)",
        [&person_id],
        |row| row.get::<_, bool>(0),
    )?;
    for (kind, value) in contact_identities(contact) {
        let normalized = repository::normalize_identity(kind, value);
        if conflicts.contains_key(&normalized) {
            continue;
        }
        if let Err(error) = repository::upsert_identity(crm, &person_id, kind, value, is_self) {
            review::enqueue(
                crm,
                "identity_collision",
                &normalized,
                &error.to_string(),
                serde_json::json!({"apple_contact_id": contact.id, "person_id": person_id, "value": value}),
            )?;
        }
    }
    Ok(true)
}

fn migration_candidate(crm: &Connection, contact: &AppleContact) -> Result<Option<String>> {
    let mut candidates = HashSet::new();
    for (kind, value) in contact_identities(contact) {
        let normalized = repository::normalize_identity(kind, value);
        let mut statement = crm.prepare(
            "SELECT DISTINCT person_id FROM identities WHERE normalized_value=?1 AND active=1
             AND person_id IN (SELECT id FROM people WHERE lifecycle_state='migration_pending')",
        )?;
        for id in statement
            .query_map([normalized], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?
        {
            candidates.insert(id);
        }
    }
    Ok((candidates.len() == 1).then(|| candidates.into_iter().next().unwrap()))
}

fn retire_missing(crm: &Connection, seen: &HashSet<&str>) -> Result<()> {
    let mut statement = crm.prepare(
        "SELECT id, apple_contact_id, display_name FROM people
         WHERE lifecycle_state='active' AND apple_contact_id IS NOT NULL",
    )?;
    let rows: Vec<(String, String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    for (person_id, apple_id, name) in rows {
        if seen.contains(apple_id.as_str()) {
            continue;
        }
        crm.execute(
            "UPDATE people SET lifecycle_state='retired', retired_at=CURRENT_TIMESTAMP,
             updated_at=CURRENT_TIMESTAMP WHERE id=?1",
            [&person_id],
        )?;
        crm.execute(
            "UPDATE identities SET active=0 WHERE person_id=?1",
            [&person_id],
        )?;
        review::enqueue(
            crm,
            "google_delete",
            &apple_id,
            &format!("Review Google deletion for retired iCloud contact {name}"),
            serde_json::json!({"person_id": person_id, "apple_contact_id": apple_id}),
        )?;
    }
    Ok(())
}

fn enqueue_migration_reviews(crm: &Connection, contacts: &[AppleContact]) -> Result<()> {
    let available: Vec<_> = contacts
        .iter()
        .map(|contact| serde_json::json!({"id": contact.id, "name": display_name(contact)}))
        .collect();
    let mut statement = crm
        .prepare("SELECT id, display_name FROM people WHERE lifecycle_state='migration_pending'")?;
    let people: Vec<(String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    for (person_id, name) in people {
        review::enqueue(
            crm,
            "migration_person",
            &person_id,
            &format!("Link CRM person {name} to an iCloud contact"),
            serde_json::json!({"display_name": name, "available_icloud_contacts": available}),
        )?;
    }
    Ok(())
}

fn enqueue_collisions(crm: &Connection, conflicts: &HashMap<String, Vec<String>>) -> Result<()> {
    for (normalized, apple_ids) in conflicts {
        review::enqueue(
            crm,
            "identity_collision",
            normalized,
            &format!("Identity {normalized} belongs to multiple iCloud contacts"),
            serde_json::json!({"apple_contact_ids": apple_ids}),
        )?;
    }
    Ok(())
}

fn contact_identities(contact: &AppleContact) -> impl Iterator<Item = (&'static str, &str)> {
    contact
        .emails
        .iter()
        .map(|item| ("email", item.value.as_str()))
        .chain(
            contact
                .phones
                .iter()
                .map(|item| ("phone", item.value.as_str())),
        )
}

fn duplicate_identities(contacts: &[AppleContact]) -> HashMap<String, Vec<String>> {
    let mut owners: HashMap<String, Vec<String>> = HashMap::new();
    for contact in contacts {
        for (kind, value) in contact_identities(contact) {
            owners
                .entry(repository::normalize_identity(kind, value))
                .or_default()
                .push(contact.id.clone());
        }
    }
    owners.retain(|_, ids| {
        ids.sort();
        ids.dedup();
        ids.len() > 1
    });
    owners
}

fn display_name(contact: &AppleContact) -> String {
    let name = format!(
        "{} {}",
        contact.given_name.trim(),
        contact.family_name.trim()
    )
    .trim()
    .to_owned();
    if !name.is_empty() {
        name
    } else if !contact.organization.trim().is_empty() {
        contact.organization.trim().into()
    } else {
        "Unnamed contact".into()
    }
}

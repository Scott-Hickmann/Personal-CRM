use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{SyncReport, rebind_unresolved_participants};
use crate::contact_publish::apple::{self, AppleContact};
use crate::error::{CrmError, Result};
use crate::progress::ProgressTracker;
use crate::{repository, review};

pub fn sync(
    config: &crate::config::Config,
    crm: &Connection,
    progress: &mut ProgressTracker,
) -> Result<SyncReport> {
    const STAGES: u64 = 5;
    let self_emails_before = repository::active_self_emails(crm)?;
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
    let selected = apple::containers(configured)?
        .into_iter()
        .find(|item| item.id == container)
        .ok_or_else(|| CrmError::Contacts("authoritative iCloud container was not found".into()))?;
    if !apple::is_icloud(&selected) {
        return Err(CrmError::Contacts(
            "the authoritative contact container is not an iCloud account".into(),
        ));
    }
    let fingerprint = apple::schema_fingerprint(configured, container)?;
    let change_token = apple::change_token(configured, container)?;
    let unchanged = if let Some(token) = change_token.as_deref() {
        source_matches_change_token(crm, token, &fingerprint)?
    } else {
        false
    };
    if unchanged {
        progress.stage(
            "Checking the iCloud contact change token",
            1,
            1,
            1,
            false,
            "query",
        );
        crm.execute(
            "UPDATE sources SET status='ok', error=NULL, last_sync_at=CURRENT_TIMESTAMP
             WHERE id='contacts'",
            [],
        )?;
        progress.finish_stage("iCloud contacts are unchanged", 1, 1, false, "query");
        return Ok(SyncReport {
            source: "contacts".into(),
            imported: 0,
            deleted: 0,
            schema_fingerprint: fingerprint,
            changed: false,
        });
    }
    progress.stage(
        "Loading the iCloud contact snapshot",
        1,
        STAGES,
        1,
        false,
        "query",
    );
    let snapshot = apple::contacts(configured, container)?;
    let content_fingerprint = content_fingerprint(&snapshot)?;
    if source_matches_content_fingerprint(crm, &content_fingerprint)? {
        crm.execute(
            "UPDATE sources SET schema_fingerprint=?1, content_fingerprint=?2, cursor=?3,
             status='ok', error=NULL, last_sync_at=CURRENT_TIMESTAMP WHERE id='contacts'",
            params![fingerprint, content_fingerprint, change_token],
        )?;
        progress.finish_stage("iCloud contact data is unchanged", 1, 1, false, "query");
        return Ok(SyncReport {
            source: "contacts".into(),
            imported: 0,
            deleted: 0,
            schema_fingerprint: fingerprint,
            changed: false,
        });
    }
    let (contacts, companies) = partition_contacts(snapshot);
    progress.finish_stage("Loaded the iCloud contact snapshot", 1, 1, false, "query");
    progress.stage(
        "Excluding iCloud company contacts",
        2,
        STAGES,
        companies.len() as u64,
        false,
        "companies",
    );
    refresh_company_exclusions_with_progress(crm, &companies, progress)?;
    let conflicts = duplicate_identities(&contacts);
    let mut active_collisions = HashSet::new();
    enqueue_collisions(crm, &conflicts, &mut active_collisions)?;

    let seen: HashSet<_> = contacts.iter().map(|contact| contact.id.as_str()).collect();
    let mut imported = 0;
    progress.stage(
        "Reconciling iCloud contacts",
        3,
        STAGES,
        contacts.len() as u64,
        false,
        "contacts",
    );
    for (index, contact) in contacts.iter().enumerate() {
        progress.focus([crate::contact_label::apple(contact)]);
        if reconcile_contact(
            crm,
            contact,
            &conflicts,
            &mut active_collisions,
            config.self_identity.apple_contact_id.as_deref(),
        )? {
            imported += 1;
        }
        progress.progress(
            "Reconciling iCloud contacts",
            (index + 1) as u64,
            contacts.len() as u64,
            false,
            "contacts",
        );
    }
    progress.finish_stage(
        "Reconciled iCloud contacts",
        contacts.len() as u64,
        contacts.len() as u64,
        false,
        "contacts",
    );
    review::resolve_absent(crm, "identity_collision", &active_collisions)?;
    retire_missing_with_progress(crm, &seen, progress, 4, STAGES)?;
    progress.stage("Finalizing contact links", 5, STAGES, 4, false, "steps");
    enqueue_migration_reviews(crm, &contacts)?;
    progress.progress("Finalizing contact links", 1, 4, false, "steps");
    rebind_unresolved_participants(crm)?;
    progress.progress("Finalizing contact links", 2, 4, false, "steps");
    review::enqueue_unresolved_candidates(crm)?;
    progress.progress("Finalizing contact links", 3, 4, false, "steps");
    let self_emails_after = repository::active_self_emails(crm)?;
    if self_emails_before != self_emails_after {
        super::gmail_backfill::reset_all(crm)?;
        progress.event("Requeued Gmail history after self-contact email addresses changed");
    }
    crm.execute(
        "INSERT INTO sources(id, kind, account, schema_fingerprint, content_fingerprint, cursor,
                             status, last_sync_at, last_reconcile_at)
         VALUES ('contacts', 'contacts', ?1, ?2, ?3, ?4, 'ok', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
         ON CONFLICT(id) DO UPDATE SET account=excluded.account,
         schema_fingerprint=excluded.schema_fingerprint,
         content_fingerprint=excluded.content_fingerprint, cursor=excluded.cursor,
         status='ok', error=NULL,
         last_sync_at=CURRENT_TIMESTAMP, last_reconcile_at=CURRENT_TIMESTAMP",
        params![container, fingerprint, content_fingerprint, change_token],
    )?;
    progress.finish_stage("Finalized contact links", 4, 4, false, "steps");
    Ok(SyncReport {
        source: "contacts".into(),
        imported,
        deleted: 0,
        schema_fingerprint: fingerprint,
        changed: true,
    })
}

fn content_fingerprint(contacts: &[AppleContact]) -> Result<String> {
    let mut contacts = contacts.to_vec();
    contacts.sort_by(|left, right| left.id.cmp(&right.id));
    for contact in &mut contacts {
        contact
            .emails
            .sort_by(|left, right| (&left.value, &left.label).cmp(&(&right.value, &right.label)));
        contact
            .phones
            .sort_by(|left, right| (&left.value, &left.label).cmp(&(&right.value, &right.label)));
    }
    let encoded = serde_json::to_vec(&contacts)
        .map_err(|error| CrmError::Serialization(error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

fn source_matches_content_fingerprint(crm: &Connection, fingerprint: &str) -> Result<bool> {
    Ok(crm
        .query_row(
            "SELECT content_fingerprint IS ?2 FROM sources WHERE id=?1",
            params!["contacts", fingerprint],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(false))
}

fn source_matches_change_token(
    crm: &Connection,
    change_token: &str,
    schema_fingerprint: &str,
) -> Result<bool> {
    Ok(crm
        .query_row(
            "SELECT cursor IS ?2 AND schema_fingerprint=?3 FROM sources WHERE id=?1",
            params!["contacts", change_token, schema_fingerprint],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(false))
}

fn reconcile_contact(
    crm: &Connection,
    contact: &AppleContact,
    conflicts: &HashMap<String, Vec<String>>,
    active_collisions: &mut HashSet<String>,
    self_apple_id: Option<&str>,
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
            &format!(
                "Link an existing CRM person to {}?",
                crate::contact_label::apple(contact)
            ),
            serde_json::json!({"suggested_apple_contact_id": contact.id, "display_name": crate::contact_label::apple_name(contact)}),
        )?;
        return Ok(false);
    } else {
        let id = Uuid::new_v4().to_string();
        crm.execute(
            "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state, last_contact_sync_at)
             VALUES (?1, ?2, ?3, 'active', CURRENT_TIMESTAMP)",
            params![id, crate::contact_label::apple_name(contact), contact.id],
        )?;
        id
    };
    crm.execute(
        "UPDATE people SET display_name=?2, lifecycle_state='active', retired_at=NULL,
         last_contact_sync_at=CURRENT_TIMESTAMP, updated_at=CURRENT_TIMESTAMP WHERE id=?1",
        params![person_id, crate::contact_label::apple_name(contact)],
    )?;
    crm.execute(
        "UPDATE identities SET active=0 WHERE person_id=?1",
        [&person_id],
    )?;
    let is_self = self_apple_id == Some(contact.id.as_str());
    for (kind, value) in contact_identities(contact) {
        let normalized = repository::normalize_identity(kind, value);
        if conflicts.contains_key(&normalized) {
            continue;
        }
        if let Err(error) = repository::upsert_identity(crm, &person_id, kind, value, is_self) {
            active_collisions.insert(normalized.clone());
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
             AND person_id IN (
                 SELECT p.id FROM people p WHERE lifecycle_state='migration_pending'
                 AND NOT EXISTS (
                     SELECT 1 FROM person_merges m WHERE m.source_person_id=p.id
                 )
             )",
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

fn retire_missing_with_progress(
    crm: &Connection,
    seen: &HashSet<&str>,
    progress: &mut ProgressTracker,
    stage_current: u64,
    stage_total: u64,
) -> Result<()> {
    let mut statement = crm.prepare(
        "SELECT id, apple_contact_id, display_name FROM people
         WHERE lifecycle_state='active' AND apple_contact_id IS NOT NULL",
    )?;
    let rows: Vec<(String, String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    let total = rows.len() as u64;
    progress.stage(
        "Checking for removed iCloud contacts",
        stage_current,
        stage_total,
        total,
        false,
        "contacts",
    );
    for (index, (person_id, apple_id, display_name)) in rows.into_iter().enumerate() {
        progress.focus([display_name]);
        if seen.contains(apple_id.as_str()) {
            progress.progress(
                "Checking for removed iCloud contacts",
                (index + 1) as u64,
                total,
                false,
                "contacts",
            );
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
        progress.progress(
            "Checking for removed iCloud contacts",
            (index + 1) as u64,
            total,
            false,
            "contacts",
        );
    }
    progress.finish_stage(
        "Checked for removed iCloud contacts",
        total,
        total,
        false,
        "contacts",
    );
    Ok(())
}

#[cfg(test)]
fn retire_missing(crm: &Connection, seen: &HashSet<&str>) -> Result<()> {
    let mut progress = ProgressTracker::disabled();
    retire_missing_with_progress(crm, seen, &mut progress, 1, 1)
}

fn enqueue_migration_reviews(crm: &Connection, contacts: &[AppleContact]) -> Result<()> {
    let available: Vec<_> = contacts
        .iter()
        .map(|contact| serde_json::json!({"id": contact.id, "name": crate::contact_label::apple(contact)}))
        .collect();
    let mut statement = crm.prepare(
        "SELECT id, display_name FROM people p WHERE lifecycle_state='migration_pending'
         AND NOT EXISTS (SELECT 1 FROM person_merges m WHERE m.source_person_id=p.id)",
    )?;
    let people: Vec<(String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    for (person_id, name) in people {
        let label = crate::contact_label::person(crm, &person_id, &name)?;
        review::enqueue(
            crm,
            "migration_person",
            &person_id,
            &format!("Link CRM person {label} to an iCloud contact"),
            serde_json::json!({"display_name": name, "available_icloud_contacts": available}),
        )?;
    }
    Ok(())
}

fn enqueue_collisions(
    crm: &Connection,
    conflicts: &HashMap<String, Vec<String>>,
    active_collisions: &mut HashSet<String>,
) -> Result<()> {
    for (normalized, apple_ids) in conflicts {
        active_collisions.insert(normalized.clone());
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

fn partition_contacts(contacts: Vec<AppleContact>) -> (Vec<AppleContact>, Vec<AppleContact>) {
    contacts
        .into_iter()
        .partition(|contact| !contact.is_company)
}

fn refresh_company_exclusions_with_progress(
    crm: &Connection,
    companies: &[AppleContact],
    progress: &mut ProgressTracker,
) -> Result<()> {
    crm.execute("DELETE FROM excluded_icloud_identities", [])?;
    for (index, contact) in companies.iter().enumerate() {
        for (kind, value) in contact_identities(contact) {
            crm.execute(
                "INSERT OR IGNORE INTO excluded_icloud_identities(
                     apple_contact_id, kind, normalized_value
                 ) VALUES (?1, ?2, ?3)",
                params![
                    contact.id,
                    kind,
                    repository::normalize_identity(kind, value)
                ],
            )?;
        }
        progress.progress(
            "Excluding iCloud company contacts",
            (index + 1) as u64,
            companies.len() as u64,
            false,
            "companies",
        );
    }
    progress.finish_stage(
        "Excluded iCloud company contacts",
        companies.len() as u64,
        companies.len() as u64,
        false,
        "companies",
    );
    Ok(())
}

#[cfg(test)]
fn refresh_company_exclusions(crm: &Connection, companies: &[AppleContact]) -> Result<()> {
    let mut progress = ProgressTracker::disabled();
    refresh_company_exclusions_with_progress(crm, companies, &mut progress)
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

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use rusqlite::Connection;
use serde::Serialize;

use super::model::{DesiredContact, owner_id, person_hash};
use super::plan::{self, ActionKind, PlannedAction};
use super::state;
use crate::error::{CrmError, Result};
use crate::gmail::Credentials;
use crate::google_contacts::{Client, Person};
use crate::review;

#[derive(Debug, Serialize)]
pub struct ActionSummary {
    pub action: ActionKind,
    pub account: String,
    pub apple_contact_id: String,
    pub google_resource_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PublishReport {
    pub applied: bool,
    pub create: usize,
    pub update: usize,
    pub delete: usize,
    pub recreate: usize,
    pub collision: usize,
    pub unchanged: usize,
    pub forget: usize,
    pub actions: Vec<ActionSummary>,
}

pub fn run(
    connection: &Connection,
    credentials: &HashMap<String, Credentials>,
    accounts: &[String],
    desired: Vec<DesiredContact>,
    apply: bool,
) -> Result<PublishReport> {
    let mirrors = state::list(connection)?;
    let mut clients = HashMap::new();
    let mut remote = HashMap::new();
    let mut duplicate_managed = HashMap::new();
    for account in accounts {
        let account_credentials = credentials.get(account).ok_or_else(|| {
            CrmError::Contacts(format!("missing OAuth credentials for {account}"))
        })?;
        let client = Client::for_account(account_credentials, account)?;
        let mut people = client.list()?;
        enqueue_unmanaged_candidates(connection, account, &people)?;
        duplicate_managed.insert(account.clone(), remove_duplicate_managed(&mut people));
        remote.insert(account.clone(), people);
        clients.insert(account.clone(), client);
    }
    let mut actions = plan::build(accounts, desired, mirrors, remote)?;
    for action in &mut actions {
        if duplicate_managed
            .get(&action.account)
            .is_some_and(|ids| ids.contains(&action.apple_id))
        {
            action.kind = ActionKind::Collision;
        }
    }
    let delete_subjects = action_subjects(&actions, ActionKind::Delete);
    let collision_subjects = action_subjects(&actions, ActionKind::Collision);
    review::resolve_absent(connection, "google_delete", &delete_subjects)?;
    review::resolve_absent(connection, "google_collision", &collision_subjects)?;
    if apply {
        apply_actions(connection, &clients, &actions)?;
    }
    Ok(report(actions, apply))
}

fn apply_actions(
    connection: &Connection,
    clients: &HashMap<String, Client>,
    actions: &[PlannedAction],
) -> Result<()> {
    for action in actions {
        let client = clients.get(&action.account).ok_or_else(|| {
            CrmError::Contacts(format!("missing Google client for {}", action.account))
        })?;
        match action.kind {
            ActionKind::Create | ActionKind::Recreate => {
                let desired = action.desired.as_ref().unwrap();
                let created = client.create(&desired.person)?;
                save_response(connection, desired, &created)?;
            }
            ActionKind::Update => update(connection, client, action)?,
            ActionKind::Delete => enqueue_delete(connection, action)?,
            ActionKind::Unchanged => {
                save_response(
                    connection,
                    action.desired.as_ref().unwrap(),
                    action.remote.as_ref().unwrap(),
                )?;
            }
            ActionKind::Forget => {
                state::remove(connection, &action.apple_id, &action.account)?;
            }
            ActionKind::Collision => enqueue_collision(connection, action)?,
        }
    }
    Ok(())
}

fn enqueue_delete(connection: &Connection, action: &PlannedAction) -> Result<()> {
    let resource = action
        .remote
        .as_ref()
        .and_then(|person| person.resource_name.as_deref());
    review::enqueue(
        connection,
        "google_delete",
        &format!("{}:{}", action.account, action.apple_id),
        &format!(
            "Delete managed Google contact {} from {}?",
            action.apple_id, action.account
        ),
        serde_json::json!({
            "account": action.account,
            "apple_contact_id": action.apple_id,
            "google_resource_name": resource,
        }),
    )?;
    Ok(())
}

fn enqueue_collision(connection: &Connection, action: &PlannedAction) -> Result<()> {
    review::enqueue(
        connection,
        "google_collision",
        &format!("{}:{}", action.account, action.apple_id),
        &format!(
            "Resolve Google collision for {} in {}",
            action.apple_id, action.account
        ),
        serde_json::json!({"account": action.account, "apple_contact_id": action.apple_id}),
    )?;
    Ok(())
}

fn enqueue_unmanaged_candidates(
    connection: &Connection,
    account: &str,
    people: &[Person],
) -> Result<()> {
    for person in people.iter().filter(|person| owner_id(person).is_none()) {
        let Some(resource) = person.resource_name.as_deref() else {
            continue;
        };
        let subject = format!("google:{account}:{resource}");
        if matches_existing_icloud_contact(connection, person)? {
            crate::review_candidates::resolve_pending_subject(
                connection,
                "contact_candidate",
                &subject,
            )?;
            continue;
        }
        let name = person.names.first().map(display_name).unwrap_or_default();
        let identity = person
            .email_addresses
            .first()
            .map(|item| item.value.as_str())
            .or_else(|| person.phone_numbers.first().map(|item| item.value.as_str()));
        if name.is_empty() && identity.is_none() {
            continue;
        }
        let label = crate::contact_label::format(
            (!name.is_empty()).then_some(name.as_str()),
            person.phone_numbers.first().map(|item| item.value.as_str()),
            person
                .email_addresses
                .first()
                .map(|item| item.value.as_str()),
        );
        review::enqueue(
            connection,
            "contact_candidate",
            &subject,
            &format!("Create an iCloud contact for {label}?"),
            serde_json::json!({
                "source": "google", "account": account, "resource_name": resource,
                "name": name, "emails": person.email_addresses, "phones": person.phone_numbers,
                "organizations": person.organizations,
            }),
        )?;
    }
    Ok(())
}

fn matches_existing_icloud_contact(connection: &Connection, person: &Person) -> Result<bool> {
    for identity in person
        .email_addresses
        .iter()
        .chain(person.phone_numbers.iter())
    {
        if crate::review_candidates::identity_belongs_to_icloud_contact(
            connection,
            &identity.value,
        )? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn display_name(name: &crate::google_contacts::Name) -> String {
    format!("{} {}", name.given_name.trim(), name.family_name.trim())
        .trim()
        .to_owned()
}

fn remove_duplicate_managed(people: &mut Vec<Person>) -> std::collections::HashSet<String> {
    let mut seen = std::collections::HashSet::new();
    let mut duplicates = std::collections::HashSet::new();
    let mut kept = Vec::with_capacity(people.len());
    for person in people.drain(..) {
        let Some(apple_id) = owner_id(&person) else {
            kept.push(person);
            continue;
        };
        if seen.insert(apple_id.to_owned()) {
            kept.push(person);
        } else {
            duplicates.insert(apple_id.to_owned());
        }
    }
    *people = kept;
    duplicates
}

fn action_subjects(
    actions: &[PlannedAction],
    kind: ActionKind,
) -> std::collections::HashSet<String> {
    actions
        .iter()
        .filter(|action| action.kind == kind)
        .map(|action| format!("{}:{}", action.account, action.apple_id))
        .collect()
}

pub fn delete_managed(
    connection: &Connection,
    credentials: &Credentials,
    account: &str,
    apple_id: &str,
    resource: &str,
) -> Result<()> {
    Client::for_account(credentials, account)?.delete(resource)?;
    state::remove(connection, apple_id, account)
}

fn update(connection: &Connection, client: &Client, action: &PlannedAction) -> Result<()> {
    let desired = action.desired.as_ref().unwrap();
    let resource = action
        .remote
        .as_ref()
        .and_then(|person| person.resource_name.as_deref())
        .ok_or_else(|| CrmError::Contacts("managed contact has no resource name".into()))?;
    let latest = client.get(resource)?;
    if person_hash(&latest)? == desired.content_hash {
        return save_response(connection, desired, &latest);
    }
    let mut request = desired.person.clone();
    request.resource_name = latest.resource_name;
    request.etag = latest.etag;
    request.metadata = latest.metadata;
    let updated = client.update(&request)?;
    save_response(connection, desired, &updated)
}

fn save_response(
    connection: &Connection,
    desired: &DesiredContact,
    response: &Person,
) -> Result<()> {
    let resource = response.resource_name.as_deref().ok_or_else(|| {
        CrmError::Contacts("Google response omitted the contact resource name".into())
    })?;
    state::upsert(
        connection,
        &desired.apple_id,
        &desired.account,
        resource,
        response.etag.as_deref(),
        &desired.content_hash,
    )
}

fn report(actions: Vec<PlannedAction>, applied: bool) -> PublishReport {
    let summaries = actions
        .iter()
        .map(|action| ActionSummary {
            action: action.kind,
            account: action.account.clone(),
            apple_contact_id: action.apple_id.clone(),
            google_resource_name: action
                .remote
                .as_ref()
                .and_then(|person| person.resource_name.clone()),
        })
        .collect();
    PublishReport {
        applied,
        create: count(&actions, ActionKind::Create),
        update: count(&actions, ActionKind::Update),
        delete: count(&actions, ActionKind::Delete),
        recreate: count(&actions, ActionKind::Recreate),
        collision: count(&actions, ActionKind::Collision),
        unchanged: count(&actions, ActionKind::Unchanged),
        forget: count(&actions, ActionKind::Forget),
        actions: summaries,
    }
}

fn count(actions: &[PlannedAction], kind: ActionKind) -> usize {
    actions.iter().filter(|action| action.kind == kind).count()
}

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use rusqlite::Connection;
use serde::Serialize;

use super::model::{DesiredContact, owner_id, person_hash};
use super::plan::{self, ActionKind, PlannedAction};
use super::state;
use crate::error::{CrmError, Result};
use crate::gmail::Credentials;
use crate::google_contacts::{Client, Person};

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
    allow_large_delete: bool,
) -> Result<PublishReport> {
    let mirrors = state::list(connection)?;
    let mut clients = HashMap::new();
    let mut remote = HashMap::new();
    for account in accounts {
        let account_credentials = credentials.get(account).ok_or_else(|| {
            CrmError::Contacts(format!("missing OAuth credentials for {account}"))
        })?;
        let client = Client::for_account(account_credentials, account)?;
        remote.insert(account.clone(), client.list()?);
        clients.insert(account.clone(), client);
    }
    let managed_count = remote
        .values()
        .flatten()
        .filter(|person| owner_id(person).is_some())
        .count();
    let actions = plan::build(accounts, desired, mirrors, remote)?;
    let delete_count = count(&actions, ActionKind::Delete);
    if apply && !allow_large_delete && large_delete(delete_count, managed_count) {
        return Err(CrmError::Contacts(format!(
            "refusing to delete {delete_count} of {managed_count} managed Google contacts; rerun with --allow-large-delete after reviewing the preview"
        )));
    }
    if apply {
        apply_actions(connection, &clients, &actions)?;
    }
    Ok(report(actions, apply))
}

fn large_delete(delete_count: usize, managed_count: usize) -> bool {
    delete_count >= 5 && delete_count.saturating_mul(10) > managed_count
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
            ActionKind::Delete => {
                if let Some(resource) = action
                    .remote
                    .as_ref()
                    .and_then(|person| person.resource_name.as_deref())
                {
                    client.delete(resource)?;
                }
                state::remove(connection, &action.apple_id, &action.account)?;
            }
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
            ActionKind::Collision => {}
        }
    }
    Ok(())
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
mod tests {
    use super::*;

    #[test]
    fn deletion_guard_requires_five_and_more_than_ten_percent() {
        assert!(!large_delete(4, 10));
        assert!(!large_delete(5, 50));
        assert!(large_delete(5, 49));
        assert!(large_delete(5, 0));
    }
}

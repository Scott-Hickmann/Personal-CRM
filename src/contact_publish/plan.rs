use std::collections::{HashMap, HashSet};

use serde::Serialize;

use super::model::{DesiredContact, owner_id, person_hash};
use super::state::Mirror;
use crate::error::{CrmError, Result};
use crate::google_contacts::Person;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Create,
    Update,
    Delete,
    Recreate,
    Collision,
    Unchanged,
    Forget,
}

pub(super) struct PlannedAction {
    pub kind: ActionKind,
    pub desired: Option<DesiredContact>,
    pub remote: Option<Person>,
    pub apple_id: String,
    pub account: String,
}

pub(super) fn build(
    accounts: &[String],
    desired: Vec<DesiredContact>,
    mirrors: Vec<Mirror>,
    remote: HashMap<String, Vec<Person>>,
) -> Result<Vec<PlannedAction>> {
    let desired_keys: HashSet<_> = desired
        .iter()
        .map(|item| (item.account.clone(), item.apple_id.clone()))
        .collect();
    let mirror_by_key: HashMap<_, _> = mirrors
        .iter()
        .map(|item| ((item.account.clone(), item.apple_id.clone()), item.clone()))
        .collect();
    let mut actions = Vec::new();

    for account in accounts {
        let people = remote.get(account).cloned().unwrap_or_default();
        let by_resource: HashMap<_, _> = people
            .iter()
            .filter_map(|person| {
                person
                    .resource_name
                    .clone()
                    .map(|name| (name, person.clone()))
            })
            .collect();
        let mut managed: HashMap<String, Person> = HashMap::new();
        for person in &people {
            if let Some(apple_id) = owner_id(person)
                && managed.insert(apple_id.into(), person.clone()).is_some()
            {
                return Err(CrmError::Contacts(format!(
                    "Google account {account} has duplicate managed contacts for Apple contact {apple_id}"
                )));
            }
        }
        for mirror in mirrors.iter().filter(|item| &item.account == account) {
            if let Some(person) = by_resource.get(&mirror.resource_name) {
                if let Some(existing) = managed.get(&mirror.apple_id)
                    && existing.resource_name != person.resource_name
                {
                    return Err(CrmError::Contacts(format!(
                        "Google account {account} has conflicting managed contacts for Apple contact {}",
                        mirror.apple_id
                    )));
                }
                managed.insert(mirror.apple_id.clone(), person.clone());
            }
        }
        let managed_resources: HashSet<_> = managed
            .values()
            .filter_map(|person| person.resource_name.clone())
            .collect();
        let unmanaged_emails: HashSet<_> = people
            .iter()
            .filter(|person| {
                person
                    .resource_name
                    .as_ref()
                    .is_none_or(|name| !managed_resources.contains(name))
            })
            .flat_map(|person| person.email_addresses.iter())
            .map(|email| email.value.trim().to_lowercase())
            .collect();

        for item in desired.iter().filter(|item| &item.account == account) {
            let key = (account.clone(), item.apple_id.clone());
            let current = managed.get(&item.apple_id).cloned();
            let kind = if let Some(person) = &current {
                if person_hash(person)? == item.content_hash {
                    ActionKind::Unchanged
                } else {
                    ActionKind::Update
                }
            } else if item
                .person
                .email_addresses
                .iter()
                .any(|email| unmanaged_emails.contains(&email.value.to_lowercase()))
            {
                ActionKind::Collision
            } else if mirror_by_key.contains_key(&key) {
                ActionKind::Recreate
            } else {
                ActionKind::Create
            };
            actions.push(PlannedAction {
                kind,
                desired: Some(item.clone()),
                remote: current,
                apple_id: item.apple_id.clone(),
                account: account.clone(),
            });
        }

        for (apple_id, person) in managed {
            if !desired_keys.contains(&(account.clone(), apple_id.clone())) {
                actions.push(PlannedAction {
                    kind: ActionKind::Delete,
                    desired: None,
                    remote: Some(person),
                    apple_id,
                    account: account.clone(),
                });
            }
        }
        for mirror in mirrors.iter().filter(|item| &item.account == account) {
            if !desired_keys.contains(&(account.clone(), mirror.apple_id.clone()))
                && !by_resource.contains_key(&mirror.resource_name)
            {
                actions.push(PlannedAction {
                    kind: ActionKind::Forget,
                    desired: None,
                    remote: None,
                    apple_id: mirror.apple_id.clone(),
                    account: account.clone(),
                });
            }
        }
    }
    actions.sort_by(|left, right| {
        left.account
            .cmp(&right.account)
            .then(left.apple_id.cmp(&right.apple_id))
    });
    Ok(actions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contact_publish::model::{OWNER_KEY, person_hash};
    use crate::google_contacts::{ClientData, TypedValue};

    fn desired(account: &str, apple_id: &str, email: &str) -> DesiredContact {
        let person = Person {
            email_addresses: vec![TypedValue {
                value: email.into(),
                kind: Some("work".into()),
            }],
            client_data: vec![ClientData {
                key: OWNER_KEY.into(),
                value: apple_id.into(),
            }],
            ..Person::default()
        };
        DesiredContact {
            apple_id: apple_id.into(),
            account: account.into(),
            content_hash: person_hash(&person).unwrap(),
            person,
        }
    }

    fn remote(item: &DesiredContact) -> Person {
        let mut person = item.person.clone();
        person.resource_name = Some(format!("people/{}", item.apple_id));
        person
    }

    #[test]
    fn plans_idempotent_managed_contact() {
        let item = desired("me@example.com", "apple-1", "a@example.com");
        let actions = build(
            &["me@example.com".into()],
            vec![item.clone()],
            Vec::new(),
            HashMap::from([("me@example.com".into(), vec![remote(&item)])]),
        )
        .unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].kind, ActionKind::Unchanged);
    }

    #[test]
    fn reports_unmanaged_email_collision() {
        let item = desired("me@example.com", "apple-1", "a@example.com");
        let unmanaged = Person {
            resource_name: Some("people/existing".into()),
            email_addresses: item.person.email_addresses.clone(),
            ..Person::default()
        };
        let actions = build(
            &["me@example.com".into()],
            vec![item],
            Vec::new(),
            HashMap::from([("me@example.com".into(), vec![unmanaged])]),
        )
        .unwrap();
        assert_eq!(actions[0].kind, ActionKind::Collision);
    }

    #[test]
    fn plans_delete_only_for_owned_remote() {
        let item = desired("me@example.com", "apple-1", "a@example.com");
        let actions = build(
            &["me@example.com".into()],
            Vec::new(),
            Vec::new(),
            HashMap::from([("me@example.com".into(), vec![remote(&item)])]),
        )
        .unwrap();
        assert_eq!(actions[0].kind, ActionKind::Delete);
    }
}

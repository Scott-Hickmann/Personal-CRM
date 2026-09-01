use sha2::{Digest, Sha256};

use super::apple::{AppleContact, LabeledValue};
use crate::config::ContactPublishConfig;
use crate::error::{CrmError, Result};
use crate::google_contacts::{ClientData, Name, Nickname, Organization, Person, TypedValue};

pub const OWNER_KEY: &str = "personal-crm.apple-contact-id";

#[derive(Debug, Clone)]
pub struct DesiredContact {
    pub apple_id: String,
    pub account: String,
    pub person: Person,
    pub content_hash: String,
}

pub fn project(
    contacts: Vec<AppleContact>,
    config: &ContactPublishConfig,
) -> Result<Vec<DesiredContact>> {
    let personal = config
        .personal_account
        .as_ref()
        .ok_or_else(not_configured)?;
    let workspace = config
        .workspace_account
        .as_ref()
        .ok_or_else(not_configured)?;
    let mut desired = Vec::new();
    for contact in contacts {
        let emails = normalized_values(&contact.emails, ValueKind::Email);
        if emails.is_empty() {
            continue;
        }
        desired.push(build(
            &contact,
            personal,
            emails.clone(),
            normalized_values(&contact.phones, ValueKind::Phone),
        )?);

        let work_emails: Vec<_> = emails
            .iter()
            .filter(|email| {
                email.kind.as_deref() == Some("work")
                    || email_domain(&email.value).is_some_and(|domain| {
                        config
                            .work_domains
                            .iter()
                            .any(|item| domain == item || domain.ends_with(&format!(".{item}")))
                    })
            })
            .cloned()
            .collect();
        if !work_emails.is_empty() {
            let work_phones = normalized_values(&contact.phones, ValueKind::Phone)
                .into_iter()
                .filter(|phone| phone.kind.as_deref() == Some("work"))
                .collect();
            desired.push(build(&contact, workspace, work_emails, work_phones)?);
        }
    }
    desired.sort_by(|left, right| {
        left.account
            .cmp(&right.account)
            .then(left.apple_id.cmp(&right.apple_id))
    });
    Ok(desired)
}

fn build(
    contact: &AppleContact,
    account: &str,
    email_addresses: Vec<TypedValue>,
    phone_numbers: Vec<TypedValue>,
) -> Result<DesiredContact> {
    let name = Name {
        honorific_prefix: contact.name_prefix.trim().into(),
        given_name: contact.given_name.trim().into(),
        middle_name: contact.middle_name.trim().into(),
        family_name: contact.family_name.trim().into(),
        honorific_suffix: contact.name_suffix.trim().into(),
    };
    let organization = Organization {
        name: contact.organization.trim().into(),
        department: contact.department.trim().into(),
        title: contact.job_title.trim().into(),
    };
    let mut person = Person {
        names: if name_is_empty(&name) {
            Vec::new()
        } else {
            vec![name]
        },
        nicknames: nonempty(&contact.nickname)
            .map(|value| vec![Nickname { value }])
            .unwrap_or_default(),
        email_addresses,
        phone_numbers,
        organizations: if organization_is_empty(&organization) {
            Vec::new()
        } else {
            vec![organization]
        },
        client_data: vec![ClientData {
            key: OWNER_KEY.into(),
            value: contact.id.clone(),
        }],
        ..Person::default()
    };
    normalize_person(&mut person);
    let content_hash = person_hash(&person)?;
    Ok(DesiredContact {
        apple_id: contact.id.clone(),
        account: account.into(),
        person,
        content_hash,
    })
}

pub fn owner_id(person: &Person) -> Option<&str> {
    person
        .client_data
        .iter()
        .find(|item| item.key == OWNER_KEY)
        .map(|item| item.value.as_str())
}

pub fn person_hash(person: &Person) -> Result<String> {
    let mut owned = person.clone();
    owned.resource_name = None;
    owned.etag = None;
    owned.metadata = None;
    normalize_person(&mut owned);
    let bytes =
        serde_json::to_vec(&owned).map_err(|error| CrmError::Serialization(error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn normalize_person(person: &mut Person) {
    person.email_addresses.sort_by(|left, right| {
        left.value
            .to_lowercase()
            .cmp(&right.value.to_lowercase())
            .then(left.kind.cmp(&right.kind))
    });
    person.phone_numbers.sort_by(|left, right| {
        left.value
            .cmp(&right.value)
            .then(left.kind.cmp(&right.kind))
    });
    person
        .client_data
        .sort_by(|left, right| left.key.cmp(&right.key).then(left.value.cmp(&right.value)));
}

enum ValueKind {
    Email,
    Phone,
}

fn normalized_values(values: &[LabeledValue], kind: ValueKind) -> Vec<TypedValue> {
    let mut output: Vec<_> = values
        .iter()
        .filter_map(|item| {
            nonempty(&item.value).map(|value| TypedValue {
                value: if matches!(kind, ValueKind::Email) {
                    value.to_lowercase()
                } else {
                    value
                },
                kind: Some(normalize_label(item.label.as_deref(), &kind)),
            })
        })
        .collect();
    output.sort_by(|left, right| left.value.cmp(&right.value));
    output.dedup_by(|left, right| left.value.eq_ignore_ascii_case(&right.value));
    output
}

fn normalize_label(label: Option<&str>, kind: &ValueKind) -> String {
    let label = label.unwrap_or_default().to_lowercase();
    if label.contains("work") {
        "work".into()
    } else if label.contains("home") {
        "home".into()
    } else if matches!(kind, ValueKind::Phone)
        && (label.contains("mobile") || label.contains("iphone"))
    {
        "mobile".into()
    } else {
        "other".into()
    }
}

fn email_domain(value: &str) -> Option<&str> {
    value.rsplit_once('@').map(|(_, domain)| domain)
}

fn nonempty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn name_is_empty(name: &Name) -> bool {
    name.honorific_prefix.is_empty()
        && name.given_name.is_empty()
        && name.middle_name.is_empty()
        && name.family_name.is_empty()
        && name.honorific_suffix.is_empty()
}

fn organization_is_empty(organization: &Organization) -> bool {
    organization.name.is_empty()
        && organization.department.is_empty()
        && organization.title.is_empty()
}

fn not_configured() -> CrmError {
    CrmError::Contacts("contact publishing is not configured".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact() -> AppleContact {
        AppleContact {
            id: "apple-1".into(),
            name_prefix: String::new(),
            given_name: "Alex".into(),
            middle_name: String::new(),
            family_name: "Example".into(),
            name_suffix: String::new(),
            nickname: String::new(),
            emails: vec![
                LabeledValue {
                    label: Some("$!<Home>!$".into()),
                    value: "alex@gmail.com".into(),
                },
                LabeledValue {
                    label: Some("$!<Work>!$".into()),
                    value: "alex@example.com".into(),
                },
            ],
            phones: vec![
                LabeledValue {
                    label: Some("$!<Mobile>!$".into()),
                    value: "555-0100".into(),
                },
                LabeledValue {
                    label: Some("$!<Work>!$".into()),
                    value: "555-0101".into(),
                },
            ],
            organization: "Example".into(),
            department: String::new(),
            job_title: "Engineer".into(),
        }
    }

    #[test]
    fn creates_full_personal_and_filtered_workspace_projections() {
        let config = ContactPublishConfig {
            personal_account: Some("personal@example.net".into()),
            workspace_account: Some("me@example.com".into()),
            work_domains: vec!["example.com".into()],
            ..ContactPublishConfig::default()
        };
        let result = project(vec![contact()], &config).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].person.email_addresses.len(), 1);
        assert_eq!(result[0].person.phone_numbers.len(), 1);
        assert_eq!(result[1].person.email_addresses.len(), 2);
        assert_eq!(result[1].person.phone_numbers.len(), 2);
    }

    #[test]
    fn ignores_contacts_without_email() {
        let mut item = contact();
        item.emails.clear();
        let config = ContactPublishConfig {
            personal_account: Some("personal@example.net".into()),
            workspace_account: Some("me@example.com".into()),
            work_domains: vec!["example.com".into()],
            ..ContactPublishConfig::default()
        };
        assert!(project(vec![item], &config).unwrap().is_empty());
    }

    #[test]
    fn work_domain_classifies_an_unlabeled_subdomain_email() {
        let mut item = contact();
        item.emails = vec![LabeledValue {
            label: None,
            value: "alex@team.example.com".into(),
        }];
        let config = ContactPublishConfig {
            personal_account: Some("personal@example.net".into()),
            workspace_account: Some("me@example.com".into()),
            work_domains: vec!["example.com".into()],
            ..ContactPublishConfig::default()
        };
        assert_eq!(project(vec![item], &config).unwrap().len(), 2);
    }
}

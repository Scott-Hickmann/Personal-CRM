use std::path::PathBuf;

use crate::cli::ReviewArgs;
use crate::config::Config;
use crate::contact_publish::apple;
use crate::contact_publish::apple::{LabeledValue, NewContact};
use crate::error::{CrmError, Result};
use crate::output::{self, Format};
use crate::sync::SyncTarget;
use crate::{commands, review, sync};

pub fn run(format: Format, config_path: PathBuf, args: ReviewArgs) -> Result<()> {
    let connection = commands::open_database(&config_path)?;
    let Some(id) = args.id.as_deref() else {
        if args.approve || args.reject || args.delete_person || args.link_icloud.is_some() {
            return Err(CrmError::InvalidConfig("a review id is required".into()));
        }
        refresh_whatsapp_reviews(&Config::load(&config_path)?, &connection)?;
        let items = review::pending(&connection)?;
        let table = items
            .iter()
            .map(|item| {
                format!(
                    "{}  {:<20} {:<24} {}",
                    item.id,
                    item.kind,
                    item.source.as_deref().unwrap_or("-"),
                    item.summary
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        return output::emit(format, "review", &items, table);
    };
    if args.delete_person {
        let person_id = crate::person_cleanup::delete_review_person(&connection, id)?;
        return output::emit(
            format,
            "review.delete_person",
            serde_json::json!({"review_id": id, "person_id": person_id}),
            format!("deleted migration person {person_id}"),
        );
    }
    if let Some(apple_id) = args.link_icloud.as_deref() {
        let mut config = Config::load(&config_path)?;
        validate_icloud_contact(&config, apple_id)?;
        let linked_self: bool = connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM identities i
                 JOIN review_items r ON r.subject_key=i.person_id
                 WHERE r.id=?1 AND i.is_self=1
             )",
            [id],
            |row| row.get(0),
        )?;
        let person_id = review::link_migration_person(&connection, id, apple_id)?;
        let is_self: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM identities WHERE person_id=?1 AND is_self=1)",
            [&person_id],
            |row| row.get(0),
        )?;
        if linked_self || is_self {
            config.self_identity.apple_contact_id = Some(apple_id.into());
            config.save(&config_path)?;
        }
        sync::run(SyncTarget::Contacts, &config, &connection)?;
        return output::emit(
            format,
            "review.link",
            serde_json::json!({"review_id": id, "person_id": person_id, "apple_contact_id": apple_id}),
            format!("linked {person_id} to iCloud contact {apple_id}"),
        );
    }
    if args.reject {
        review::reject(&connection, id)?;
        return output::emit(
            format,
            "review.reject",
            serde_json::json!({"review_id": id}),
            format!("rejected {id}"),
        );
    }
    if args.approve {
        let item = review::get_pending(&connection, id)?;
        return match item.kind.as_str() {
            "migration_person" | "contact_candidate" => {
                approve_contact(format, &config_path, &connection, &item)
            }
            "google_delete" => approve_google_delete(format, &config_path, &connection, &item),
            "google_collision" | "identity_collision" => Err(CrmError::InvalidConfig(
                "fix collisions at the source; the daemon will resolve the review after reconciliation".into(),
            )),
            _ => Err(CrmError::InvalidConfig(format!("cannot approve {}", item.kind))),
        };
    }
    Err(CrmError::InvalidConfig(
        "choose --link-icloud, --approve, --reject, or --delete-person".into(),
    ))
}

fn refresh_whatsapp_reviews(config: &Config, connection: &rusqlite::Connection) -> Result<()> {
    if config.paths.whatsapp.is_some() {
        sync::run(SyncTarget::Whatsapp, config, connection)?;
        review::enqueue_unresolved_candidates(connection)?;
    }
    Ok(())
}

fn approve_contact(
    format: Format,
    config_path: &std::path::Path,
    connection: &rusqlite::Connection,
    item: &review::ReviewItem,
) -> Result<()> {
    let mut config = Config::load(config_path)?;
    let container = config
        .contact_publish
        .source_container
        .clone()
        .ok_or_else(|| {
            CrmError::Contacts("authoritative iCloud container is not configured".into())
        })?;
    let contact = if item.kind == "migration_person" {
        contact_from_person(connection, &item.subject_key, container)?
    } else {
        contact_from_candidate(item, container)
    };
    let apple_id = apple::create(&contact)?;
    if item.kind == "migration_person" {
        let person_id = review::link_migration_person(connection, &item.id, &apple_id)?;
        let is_self: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM identities WHERE person_id=?1 AND is_self=1)",
            [&person_id],
            |row| row.get(0),
        )?;
        if is_self {
            config.self_identity.apple_contact_id = Some(apple_id.clone());
            config.save(config_path)?;
        }
    } else {
        review::resolve(connection, &item.id)?;
    }
    sync::run(SyncTarget::Contacts, &config, connection)?;
    output::emit(
        format,
        "review.approve",
        serde_json::json!({"review_id": item.id, "apple_contact_id": apple_id}),
        format!("created iCloud contact {apple_id}"),
    )
}

fn contact_from_person(
    connection: &rusqlite::Connection,
    person_id: &str,
    container_id: String,
) -> Result<NewContact> {
    let name: String = connection.query_row(
        "SELECT display_name FROM people WHERE id=?1",
        [person_id],
        |row| row.get(0),
    )?;
    let mut statement = connection.prepare(
        "SELECT kind, value FROM identities WHERE person_id=?1 AND active=1 ORDER BY kind, value",
    )?;
    let identities: Vec<(String, String)> = statement
        .query_map([person_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(NewContact {
        container_id,
        display_name: name,
        emails: values(&identities, "email"),
        phones: values(&identities, "phone"),
        organization: String::new(),
    })
}

fn contact_from_candidate(item: &review::ReviewItem, container_id: String) -> NewContact {
    let identity = item
        .details
        .get("identity")
        .and_then(|value| value.as_str());
    let display_name = item
        .details
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let mut emails = json_values(item.details.get("emails"));
    let mut phones = json_values(item.details.get("phones"));
    if let Some(identity) = identity {
        let value = LabeledValue {
            label: None,
            value: identity.into(),
        };
        if identity.contains('@') {
            emails.push(value)
        } else {
            phones.push(value)
        }
    }
    NewContact {
        container_id,
        display_name: display_name.into(),
        emails,
        phones,
        organization: String::new(),
    }
}

fn values(identities: &[(String, String)], kind: &str) -> Vec<LabeledValue> {
    identities
        .iter()
        .filter(|(candidate, _)| candidate == kind)
        .map(|(_, value)| LabeledValue {
            label: None,
            value: value.clone(),
        })
        .collect()
}

fn json_values(value: Option<&serde_json::Value>) -> Vec<LabeledValue> {
    value
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item.get("value")
                .and_then(|value| value.as_str())
                .map(|value| LabeledValue {
                    label: item
                        .get("type")
                        .and_then(|label| label.as_str())
                        .map(str::to_owned),
                    value: value.into(),
                })
        })
        .collect()
}

fn approve_google_delete(
    format: Format,
    config_path: &std::path::Path,
    connection: &rusqlite::Connection,
    item: &review::ReviewItem,
) -> Result<()> {
    let config = Config::load(config_path)?;
    let account = detail(&item.details, "account")?;
    let apple_id = detail(&item.details, "apple_contact_id")?;
    let resource = detail(&item.details, "google_resource_name")?;
    let path = config
        .contact_publish
        .account_credentials
        .get(account)
        .or(config.contact_publish.credentials_path.as_ref())
        .ok_or_else(|| {
            CrmError::Contacts(format!(
                "Google credentials are not configured for {account}"
            ))
        })?;
    let credentials = crate::gmail::Credentials::load(path)?;
    crate::contact_publish::reconcile::delete_managed(
        connection,
        &credentials,
        account,
        apple_id,
        resource,
    )?;
    review::resolve(connection, &item.id)?;
    output::emit(
        format,
        "review.approve",
        serde_json::json!({"review_id": item.id, "deleted": resource}),
        format!("deleted managed Google contact {resource}"),
    )
}

fn detail<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .ok_or_else(|| CrmError::InvalidConfig(format!("review item is missing {key}")))
}

fn validate_icloud_contact(config: &Config, apple_id: &str) -> Result<()> {
    let configured = config
        .paths
        .contacts
        .as_deref()
        .ok_or_else(|| CrmError::Contacts("Apple Contacts path is not configured".into()))?;
    let container = config
        .contact_publish
        .source_container
        .as_deref()
        .ok_or_else(|| {
            CrmError::Contacts("authoritative iCloud container is not configured".into())
        })?;
    if apple::contacts(configured, container)?
        .iter()
        .any(|contact| contact.id == apple_id)
    {
        Ok(())
    } else {
        Err(CrmError::Contacts(format!(
            "iCloud contact {apple_id} is not in the authoritative container"
        )))
    }
}

#[cfg(test)]
mod tests;

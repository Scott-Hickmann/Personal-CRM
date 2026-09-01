use std::path::PathBuf;

use rusqlite::OptionalExtension;

use crate::cli::ReviewArgs;
use crate::config::Config;
use crate::contact_publish::apple;
use crate::error::{CrmError, Result};
use crate::output::{self, Format};
use crate::sync::SyncTarget;
use crate::{commands, review, sync};

pub fn run(format: Format, config_path: PathBuf, args: ReviewArgs) -> Result<()> {
    let connection = commands::open_database(&config_path)?;
    let Some(id) = args.id.as_deref() else {
        if args.approve || args.reject || args.link_icloud.is_some() {
            return Err(CrmError::InvalidConfig("a review id is required".into()));
        }
        let items = review::pending(&connection)?;
        let table = items
            .iter()
            .map(|item| format!("{}  {:<20} {}", item.id, item.kind, item.summary))
            .collect::<Vec<_>>()
            .join("\n");
        return output::emit(format, "review", &items, table);
    };
    if let Some(apple_id) = args.link_icloud.as_deref() {
        let mut config = Config::load(&config_path)?;
        validate_icloud_contact(&config, apple_id)?;
        let person_id = review::link_migration_person(&connection, id, apple_id)?;
        let is_self: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM identities WHERE person_id=?1 AND is_self=1)",
            [&person_id],
            |row| row.get(0),
        )?;
        if is_self {
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
        let kind: Option<String> = connection
            .query_row(
                "SELECT kind FROM review_items WHERE id=?1 AND status='pending'",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        return Err(CrmError::InvalidConfig(format!(
            "approval for {} is not available until its external action is configured",
            kind.unwrap_or_else(|| "this review".into())
        )));
    }
    Err(CrmError::InvalidConfig(
        "choose --link-icloud, --approve, or --reject".into(),
    ))
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

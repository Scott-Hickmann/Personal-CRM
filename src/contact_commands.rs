use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::cli::{ContactsCommand, ContactsConfigureArgs, ContactsPublishArgs};
use crate::config::Config;
use crate::contact_publish::{self, apple};
use crate::error::{CrmError, Result};
use crate::gmail::Credentials;
use crate::output::{self, Format};

#[derive(Serialize)]
struct ContactStatus<'a> {
    configured: bool,
    source_container: Option<&'a str>,
    personal_account: Option<&'a str>,
    workspace_account: Option<&'a str>,
    work_domains: &'a [String],
    authorized_accounts: &'a [String],
}

pub(crate) fn run(format: Format, config_path: PathBuf, command: ContactsCommand) -> Result<()> {
    if !config_path.exists() {
        return Err(CrmError::ConfigMissing(config_path));
    }
    match command {
        ContactsCommand::Containers => list_containers(format, &config_path),
        ContactsCommand::Configure(args) => configure(format, config_path, args),
        ContactsCommand::Publish(args) => publish(format, config_path, args),
        ContactsCommand::Status => status(format, config_path, "contacts.status"),
    }
}

fn list_containers(format: Format, config_path: &std::path::Path) -> Result<()> {
    let config = Config::load(config_path)?;
    let contacts_path = contacts_path(&config)?;
    let containers = apple::containers(contacts_path)?;
    let table = containers
        .iter()
        .map(|item| format!("{}\t{}\t{}", item.id, item.name, item.kind))
        .collect::<Vec<_>>()
        .join("\n");
    output::emit(format, "contacts.containers", &containers, table)
}

fn configure(format: Format, config_path: PathBuf, args: ContactsConfigureArgs) -> Result<()> {
    let mut config = Config::load(&config_path)?;
    let personal = normalize_email(&args.personal_account)?;
    let workspace = normalize_email(&args.workspace_account)?;
    if personal == workspace {
        return Err(CrmError::Contacts(
            "personal and Workspace accounts must be different".into(),
        ));
    }
    for account in [&personal, &workspace] {
        if !config.contact_publish.accounts.contains(account) {
            return Err(CrmError::Contacts(format!(
                "{account} is not authorized; run `crm auth contacts add` while signed into that account"
            )));
        }
    }
    let containers = apple::containers(contacts_path(&config)?)?;
    if !containers
        .iter()
        .any(|item| item.id == args.source_container)
    {
        return Err(CrmError::Contacts(format!(
            "contact container {} was not found",
            args.source_container
        )));
    }
    let mut domains: Vec<_> = args
        .work_domains
        .into_iter()
        .map(|domain| domain.trim().trim_start_matches('@').to_lowercase())
        .filter(|domain| !domain.is_empty())
        .collect();
    domains.sort();
    domains.dedup();
    if domains
        .iter()
        .any(|domain| domain.contains('@') || !domain.contains('.'))
    {
        return Err(CrmError::Contacts(
            "work domains must look like example.com".into(),
        ));
    }
    config.contact_publish.source_container = Some(args.source_container);
    config.contact_publish.personal_account = Some(personal);
    config.contact_publish.workspace_account = Some(workspace);
    config.contact_publish.work_domains = domains;
    config.save(&config_path)?;
    status(format, config_path, "contacts.configure")
}

fn publish(format: Format, config_path: PathBuf, args: ContactsPublishArgs) -> Result<()> {
    let config = Config::load(&config_path)?;
    let publish = &config.contact_publish;
    let container = publish
        .source_container
        .as_deref()
        .ok_or_else(not_configured)?;
    let personal = publish
        .personal_account
        .as_ref()
        .ok_or_else(not_configured)?;
    let workspace = publish
        .workspace_account
        .as_ref()
        .ok_or_else(not_configured)?;
    let mut credentials = HashMap::new();
    for account in [personal, workspace] {
        let path = publish
            .account_credentials
            .get(account)
            .or(publish.credentials_path.as_ref())
            .ok_or_else(|| {
                CrmError::Contacts(format!(
                    "Google Contacts credentials are not configured for {account}"
                ))
            })?;
        credentials.insert(account.clone(), Credentials::load(path)?);
    }
    let desired = contact_publish::project(
        apple::contacts(contacts_path(&config)?, container)?,
        publish,
    )?;
    let connection = crate::commands::open_database(&config_path)?;
    let report = contact_publish::reconcile::run(
        &connection,
        &credentials,
        &[personal.clone(), workspace.clone()],
        desired,
        args.apply,
        args.allow_large_delete,
    )?;
    let mode = if report.applied { "applied" } else { "preview" };
    let table = format!(
        "{mode}\ncreate     {:>6}\nupdate     {:>6}\ndelete     {:>6}\nrecreate   {:>6}\ncollision  {:>6}\nunchanged  {:>6}\nforget     {:>6}",
        report.create,
        report.update,
        report.delete,
        report.recreate,
        report.collision,
        report.unchanged,
        report.forget
    );
    output::emit(format, "contacts.publish", &report, table)
}

fn status(format: Format, config_path: PathBuf, command: &str) -> Result<()> {
    let config = Config::load(&config_path)?;
    let publish = &config.contact_publish;
    let configured = publish.source_container.is_some()
        && publish.personal_account.is_some()
        && publish.workspace_account.is_some()
        && !publish.work_domains.is_empty();
    let result = ContactStatus {
        configured,
        source_container: publish.source_container.as_deref(),
        personal_account: publish.personal_account.as_deref(),
        workspace_account: publish.workspace_account.as_deref(),
        work_domains: &publish.work_domains,
        authorized_accounts: &publish.accounts,
    };
    let table = format!(
        "configured         {}\nsource container   {}\npersonal account   {}\nworkspace account  {}\nwork domains       {}\nauthorized         {}",
        configured,
        result.source_container.unwrap_or("not set"),
        result.personal_account.unwrap_or("not set"),
        result.workspace_account.unwrap_or("not set"),
        result.work_domains.join(", "),
        result.authorized_accounts.join(", ")
    );
    output::emit(format, command, &result, table)
}

fn normalize_email(value: &str) -> Result<String> {
    let value = value.trim().to_lowercase();
    if value.split_once('@').is_none_or(|(local, domain)| {
        local.is_empty() || domain.is_empty() || !domain.contains('.')
    }) {
        return Err(CrmError::Contacts(format!(
            "invalid Google account email: {value}"
        )));
    }
    Ok(value)
}

fn not_configured() -> CrmError {
    CrmError::Contacts("contact publishing is not configured; run `crm contacts configure`".into())
}

fn contacts_path(config: &Config) -> Result<&std::path::Path> {
    config
        .paths
        .contacts
        .as_deref()
        .ok_or_else(|| CrmError::Contacts("Apple Contacts path is not configured".into()))
}

use std::path::PathBuf;

use crate::cli::{AuthCommand, ContactsAuthCommand, GmailCommand};
use crate::config::Config;
use crate::error::{CrmError, Result};
use crate::gmail;
use crate::output::{self, Format};

pub(crate) fn run(format: Format, config_path: PathBuf, command: AuthCommand) -> Result<()> {
    if !config_path.exists() {
        return Err(CrmError::ConfigMissing(config_path));
    }
    let mut config = Config::load(&config_path)?;
    match command {
        AuthCommand::Gmail {
            command: GmailCommand::Add { credentials },
        } => {
            let account = gmail::authorize(&mut config, &config_path, &credentials)?;
            let table = format!("authorized  {}", account.email);
            output::emit(format, "auth.gmail.add", &account, table)
        }
        AuthCommand::Gmail {
            command: GmailCommand::List,
        } => {
            let accounts = gmail::list_accounts(&config);
            output::emit(format, "auth.gmail.list", accounts, accounts.join("\n"))
        }
        AuthCommand::Gmail {
            command: GmailCommand::Remove { account },
        } => {
            gmail::remove_account(&mut config, &config_path, &account)?;
            output::emit(
                format,
                "auth.gmail.remove",
                serde_json::json!({"account": account}),
                "Gmail account removed".into(),
            )
        }
        AuthCommand::Contacts {
            command: ContactsAuthCommand::Add { credentials },
        } => {
            let account = crate::google_contacts::authorize(
                &mut config,
                &config_path,
                credentials.as_deref(),
            )?;
            let table = format!("authorized  {}", account.email);
            output::emit(format, "auth.contacts.add", &account, table)
        }
        AuthCommand::Contacts {
            command: ContactsAuthCommand::List,
        } => {
            let accounts = crate::google_contacts::list_accounts(&config);
            output::emit(format, "auth.contacts.list", accounts, accounts.join("\n"))
        }
        AuthCommand::Contacts {
            command: ContactsAuthCommand::Remove { account },
        } => {
            crate::google_contacts::remove_account(&mut config, &config_path, &account)?;
            output::emit(
                format,
                "auth.contacts.remove",
                serde_json::json!({"account": account}),
                "Google Contacts account removed".into(),
            )
        }
    }
}

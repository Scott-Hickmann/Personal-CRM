use std::path::PathBuf;

use crate::cli::{AuthCommand, GmailCommand};
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
    }
}

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use serde::Serialize;

use crate::config::{self, Config};
use crate::db;
use crate::error::{CrmError, Result};
use crate::output::{self, Format};

#[derive(Parser)]
#[command(name = "crm", version, about)]
pub struct Cli {
    #[arg(long, value_enum, default_value = "table", global = true)]
    format: Format,
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Doctor,
    Status,
}

#[derive(Subcommand)]
enum ConfigCommand {
    Init(InitArgs),
    Show,
}

#[derive(Args)]
struct InitArgs {
    #[arg(long)]
    self_name: String,
    #[arg(long = "self-email")]
    self_emails: Vec<String>,
    #[arg(long = "self-phone")]
    self_phones: Vec<String>,
}

#[derive(Serialize)]
struct Status {
    config_path: PathBuf,
    database_path: PathBuf,
    schema_version: i64,
    source_count: i64,
}

pub fn run(cli: Cli) -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Err(CrmError::UnsupportedPlatform);
    }
    let config_path = cli.config.unwrap_or(config::default_config_path()?);
    match cli.command {
        Command::Config {
            command: ConfigCommand::Init(args),
        } => init(cli.format, config_path, args),
        Command::Config {
            command: ConfigCommand::Show,
        } => show_config(cli.format, config_path),
        Command::Doctor => doctor(cli.format, config_path),
        Command::Status => status(cli.format, config_path),
    }
}

fn init(format: Format, config_path: PathBuf, args: InitArgs) -> Result<()> {
    let config = Config::new(args.self_name, args.self_emails, args.self_phones)?;
    config.save_new(&config_path)?;
    let database_path = config_path
        .parent()
        .ok_or_else(|| CrmError::InvalidConfig("configuration path has no parent".into()))?
        .join("crm.sqlite3");
    let connection = db::open(&database_path)?;
    let status = Status {
        config_path,
        database_path,
        schema_version: db::schema_version(&connection)?,
        source_count: 0,
    };
    output::emit(format, "config.init", &status, "CRM initialized".into())
}

fn show_config(format: Format, config_path: PathBuf) -> Result<()> {
    ensure_config(&config_path)?;
    let config = Config::load(&config_path)?;
    let table = toml::to_string_pretty(&config)
        .map_err(|error| CrmError::Serialization(error.to_string()))?;
    output::emit(format, "config.show", &config, table)
}

fn doctor(format: Format, config_path: PathBuf) -> Result<()> {
    ensure_config(&config_path)?;
    let config = Config::load(&config_path)?;
    let database_path = config_path.parent().unwrap().join("crm.sqlite3");
    let connection = db::open(&database_path)?;
    let checks = serde_json::json!({
        "platform": "ok",
        "config": "ok",
        "database": "ok",
        "schema_version": db::schema_version(&connection)?,
        "self_name": config.self_identity.name,
    });
    output::emit(
        format,
        "doctor",
        &checks,
        "platform  ok\nconfig    ok\ndatabase  ok".into(),
    )
}

fn status(format: Format, config_path: PathBuf) -> Result<()> {
    ensure_config(&config_path)?;
    let database_path = config_path.parent().unwrap().join("crm.sqlite3");
    let connection = db::open(&database_path)?;
    let source_count =
        connection.query_row("SELECT COUNT(*) FROM sources", [], |row| row.get(0))?;
    let status = Status {
        config_path,
        database_path,
        schema_version: db::schema_version(&connection)?,
        source_count,
    };
    let table = format!(
        "schema version  {}\nsources         {}",
        status.schema_version, status.source_count
    );
    output::emit(format, "status", &status, table)
}

fn ensure_config(path: &PathBuf) -> Result<()> {
    if path.exists() {
        Ok(())
    } else {
        Err(CrmError::ConfigMissing(path.clone()))
    }
}

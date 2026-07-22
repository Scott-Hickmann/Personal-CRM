use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};
use serde::Serialize;

use crate::config::{self, Config};
use crate::db;
use crate::error::{CrmError, Result};
use crate::output::{self, Format};
use crate::repository;
use crate::source::ReadOnlySource;

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
    Person {
        #[command(subcommand)]
        command: PersonCommand,
    },
    Note {
        #[command(subcommand)]
        command: NoteCommand,
    },
    Fact {
        #[command(subcommand)]
        command: FactCommand,
    },
    Tag {
        #[command(subcommand)]
        command: TagCommand,
    },
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

#[derive(Subcommand)]
enum PersonCommand {
    Add(PersonAddArgs),
    Show(PersonReference),
}

#[derive(Subcommand)]
enum NoteCommand {
    Add(TextMutationArgs),
}

#[derive(Subcommand)]
enum FactCommand {
    Set(FactSetArgs),
}

#[derive(Subcommand)]
enum TagCommand {
    Add(TagAddArgs),
}

#[derive(Args)]
struct PersonAddArgs {
    #[arg(long)]
    name: String,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct PersonReference {
    person: String,
}

#[derive(Args)]
struct TextMutationArgs {
    #[arg(long)]
    person: String,
    #[arg(long)]
    text: String,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct FactSetArgs {
    #[arg(long)]
    person: String,
    #[arg(long)]
    key: String,
    #[arg(long)]
    value: String,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct TagAddArgs {
    #[arg(long)]
    person: String,
    #[arg(long)]
    tag: String,
    #[arg(long)]
    dry_run: bool,
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
        Command::Person { command } => person(cli.format, config_path, command),
        Command::Note { command } => note(cli.format, config_path, command),
        Command::Fact { command } => fact(cli.format, config_path, command),
        Command::Tag { command } => tag(cli.format, config_path, command),
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
    repository::ensure_self(&connection, &config.self_identity)?;
    let status = Status {
        config_path,
        database_path,
        schema_version: db::schema_version(&connection)?,
        source_count: 0,
    };
    output::emit(format, "config.init", &status, "CRM initialized".into())
}

fn person(format: Format, config_path: PathBuf, command: PersonCommand) -> Result<()> {
    let connection = open_configured_database(&config_path)?;
    match command {
        PersonCommand::Add(args) => {
            let result = repository::create_person(&connection, &args.name, args.dry_run)?;
            output::emit(
                format,
                "person.add",
                &result,
                format!("{}  {}", result.person_id, args.name),
            )
        }
        PersonCommand::Show(args) => {
            let result = repository::get_person(&connection, &args.person)?;
            let table = format!(
                "{}  {}\nnotes  {}\nfacts  {}\ntags   {}",
                result.id,
                result.display_name,
                result.notes.len(),
                result.facts.len(),
                result.tags.len()
            );
            output::emit(format, "person.show", &result, table)
        }
    }
}

fn note(format: Format, config_path: PathBuf, command: NoteCommand) -> Result<()> {
    let connection = open_configured_database(&config_path)?;
    let NoteCommand::Add(args) = command;
    let result = repository::add_note(&connection, &args.person, &args.text, args.dry_run)?;
    output::emit(
        format,
        "note.add",
        &result,
        format!("{}  note added", result.person_id),
    )
}

fn fact(format: Format, config_path: PathBuf, command: FactCommand) -> Result<()> {
    let connection = open_configured_database(&config_path)?;
    let FactCommand::Set(args) = command;
    let result = repository::set_fact(
        &connection,
        &args.person,
        &args.key,
        &args.value,
        args.dry_run,
    )?;
    output::emit(
        format,
        "fact.set",
        &result,
        format!("{}  {}={}", result.person_id, args.key, args.value),
    )
}

fn tag(format: Format, config_path: PathBuf, command: TagCommand) -> Result<()> {
    let connection = open_configured_database(&config_path)?;
    let TagCommand::Add(args) = command;
    let result = repository::add_tag(&connection, &args.person, &args.tag, args.dry_run)?;
    output::emit(
        format,
        "tag.add",
        &result,
        format!("{}  #{}", result.person_id, args.tag),
    )
}

fn open_configured_database(config_path: &Path) -> Result<rusqlite::Connection> {
    ensure_config(config_path)?;
    Config::load(config_path)?;
    db::open(&config_path.parent().unwrap().join("crm.sqlite3"))
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
    let source_checks = probe_sources(&config)?;
    let checks = serde_json::json!({
        "platform": "ok",
        "config": "ok",
        "database": "ok",
        "schema_version": db::schema_version(&connection)?,
        "self_name": config.self_identity.name,
        "sources": source_checks,
    });
    output::emit(
        format,
        "doctor",
        &checks,
        format!(
            "platform  ok\nconfig    ok\ndatabase  ok\nsources   {}",
            source_checks.len()
        ),
    )
}

fn probe_sources(config: &Config) -> Result<Vec<serde_json::Value>> {
    let specifications = [
        (
            "contacts",
            config.paths.contacts.as_ref(),
            "ZABCDRECORD",
            &["ZUNIQUEID"][..],
        ),
        (
            "imessage",
            config.paths.imessage.as_ref(),
            "message",
            &["guid", "date"][..],
        ),
        (
            "whatsapp",
            config.paths.whatsapp.as_ref(),
            "ZWAMESSAGE",
            &["Z_PK", "ZMESSAGEDATE"][..],
        ),
        (
            "apple_calls",
            config.paths.apple_calls.as_ref(),
            "ZCALLRECORD",
            &["Z_PK"][..],
        ),
        (
            "whatsapp_calls",
            config.paths.whatsapp_calls.as_ref(),
            "ZWACDCALLEVENT",
            &["Z_PK"][..],
        ),
    ];
    specifications
        .into_iter()
        .map(|(kind, path, table, columns)| {
            let path =
                path.ok_or_else(|| CrmError::InvalidConfig(format!("missing path for {kind}")))?;
            if !path.exists() {
                return Ok(serde_json::json!({"kind": kind, "status": "missing", "path": path}));
            }
            let source = ReadOnlySource::open(path)?;
            source.require_columns(table, columns)?;
            Ok(serde_json::json!({
                "kind": kind,
                "status": "ok",
                "path": path,
                "schema_fingerprint": source.schema_fingerprint()?,
            }))
        })
        .collect()
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

fn ensure_config(path: &Path) -> Result<()> {
    if path.exists() {
        Ok(())
    } else {
        Err(CrmError::ConfigMissing(path.to_owned()))
    }
}

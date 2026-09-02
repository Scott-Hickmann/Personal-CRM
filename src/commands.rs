use std::path::{Path, PathBuf};

use crate::cli::{
    Command, ConfigCommand, FactCommand, FollowupCommand, InitArgs, NoteCommand, PersonCommand,
    QueryArgs, TagCommand, UiDataCommand,
};
use crate::config::{self, Config};
use crate::db;
use crate::error::{CrmError, Result};
use crate::output::{self, Format};
use crate::query::{self, QueryOptions};
use crate::repository;
use crate::source::ReadOnlySource;

pub(crate) fn run(format: Format, config: Option<PathBuf>, command: Command) -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Err(CrmError::UnsupportedPlatform);
    }
    let config_path = config.unwrap_or(config::default_config_path()?);
    match command {
        Command::Ui(args) => crate::ui_commands::run(args.port, &config_path),
        Command::UiData { command } => ui_data(format, config_path, command),
        Command::Config {
            command: ConfigCommand::Init(args),
        } => init(format, config_path, args),
        Command::Config {
            command: ConfigCommand::Show,
        } => show_config(format, config_path),
        Command::Doctor => doctor(format, config_path),
        Command::Start => crate::daemon_commands::start(format, config_path),
        Command::Stop => crate::daemon_commands::stop(format, config_path),
        Command::Status(args) => crate::status_commands::run(format, config_path, args),
        Command::Review(args) => crate::review_commands::run(format, config_path, args),
        Command::Run(args) => crate::daemon_commands::run_job(format, config_path, args.job),
        Command::Daemon => crate::daemon::run(config_path),
        Command::Person { command } => person(format, config_path, command),
        Command::Note { command } => note(format, config_path, command),
        Command::Fact { command } => fact(format, config_path, command),
        Command::Tag { command } => tag(format, config_path, command),
        Command::Followup { command } => followup(format, config_path, command),
        Command::Affinity { command } => {
            crate::affinity_commands::run(format, config_path, command)
        }
        Command::Query(args) => query_entities(format, config_path, args),
        Command::History(args) => crate::history_commands::run(format, config_path, args),
        Command::Explain(args) => crate::analytics_commands::explain(format, config_path, args),
        Command::Graph(args) => crate::analytics_commands::graph(format, config_path, args),
        Command::Face { command } => crate::face_commands::run(format, command),
        Command::Photos { command } => crate::photos_commands::run(format, config_path, command),
        Command::Contacts { command } => crate::contact_commands::run(format, config_path, command),
        Command::Auth { command } => crate::auth_commands::run(format, config_path, command),
    }
}

fn ui_data(format: Format, config_path: PathBuf, command: UiDataCommand) -> Result<()> {
    let connection = open_database(&config_path)?;
    match command {
        UiDataCommand::Overview => {
            let result = crate::ui_data::overview(&connection)?;
            output::emit(format, "ui-data.overview", &result, "CRM overview".into())
        }
        UiDataCommand::Person(args) => {
            let result = crate::ui_data::person(&connection, &args.person, args.history_limit)?;
            output::emit(
                format,
                "ui-data.person",
                &result,
                result.person.display_name.clone(),
            )
        }
        UiDataCommand::Interaction(args) => {
            let result = crate::ui_data::interaction(&connection, &args.interaction)?;
            output::emit(
                format,
                "ui-data.interaction",
                &result,
                result.body.clone().unwrap_or_default(),
            )
        }
    }
}

fn query_entities(format: Format, config_path: PathBuf, args: QueryArgs) -> Result<()> {
    let connection = open_database(&config_path)?;
    let result = query::execute(
        &connection,
        args.entity,
        QueryOptions {
            include_retired: args.include_retired,
            select: args.select.as_deref(),
            filter: args.filter.as_deref(),
            sort: args.sort.as_deref(),
            group: args.group.as_deref(),
            limit: args.limit,
        },
    )?;
    let table = result
        .rows
        .iter()
        .map(|row| {
            result
                .columns
                .iter()
                .map(|column| row.get(column).map(display_json).unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n");
    output::emit(format, "query", &result, table)
}

fn display_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(value) => value.clone(),
        other => other.to_string(),
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
    let status = crate::status_commands::initialized(
        config_path,
        database_path,
        db::schema_version(&connection)?,
    );
    output::emit(format, "config.init", &status, "CRM initialized".into())
}

fn person(format: Format, config_path: PathBuf, command: PersonCommand) -> Result<()> {
    let connection = open_database(&config_path)?;
    match command {
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
        PersonCommand::Delete(args) => {
            let person_id =
                crate::person_cleanup::delete_retired_person(&connection, &args.person)?;
            output::emit(
                format,
                "person.delete",
                serde_json::json!({"person_id": person_id}),
                format!("deleted retired person {person_id}"),
            )
        }
    }
}

fn note(format: Format, config_path: PathBuf, command: NoteCommand) -> Result<()> {
    let connection = open_database(&config_path)?;
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
    let connection = open_database(&config_path)?;
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
    let connection = open_database(&config_path)?;
    let TagCommand::Add(args) = command;
    let result = repository::add_tag(&connection, &args.person, &args.tag, args.dry_run)?;
    output::emit(
        format,
        "tag.add",
        &result,
        format!("{}  #{}", result.person_id, args.tag),
    )
}

fn followup(format: Format, config_path: PathBuf, command: FollowupCommand) -> Result<()> {
    let connection = open_database(&config_path)?;
    let FollowupCommand::Add(args) = command;
    let result = repository::add_followup(
        &connection,
        &args.person,
        &args.text,
        args.due.as_deref(),
        args.dry_run,
    )?;
    output::emit(
        format,
        "followup.add",
        &result,
        format!("{}  follow-up added", result.person_id),
    )
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
    let connection = db::open(&config_path.parent().unwrap().join("crm.sqlite3"))?;
    let source_checks = probe_sources(&config)?;
    let checks = serde_json::json!({
        "platform": "ok", "config": "ok", "database": "ok",
        "schema_version": db::schema_version(&connection)?,
        "self_name": config.self_identity.name, "sources": source_checks,
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
    specifications.into_iter().map(|(kind, path, table, columns)| {
        let path = path.ok_or_else(|| CrmError::InvalidConfig(format!("missing path for {kind}")))?;
        if !path.exists() { return Ok(serde_json::json!({"kind": kind, "status": "missing", "path": path})); }
        let source = ReadOnlySource::open(path)?;
        source.require_columns(table, columns)?;
        Ok(serde_json::json!({"kind": kind, "status": "ok", "path": path, "schema_fingerprint": source.schema_fingerprint()?}))
    }).collect()
}

pub(crate) fn open_database(config_path: &Path) -> Result<rusqlite::Connection> {
    ensure_config(config_path)?;
    Config::load(config_path)?;
    db::open(&config_path.parent().unwrap().join("crm.sqlite3"))
}

fn ensure_config(path: &Path) -> Result<()> {
    if path.exists() {
        Ok(())
    } else {
        Err(CrmError::ConfigMissing(path.to_owned()))
    }
}

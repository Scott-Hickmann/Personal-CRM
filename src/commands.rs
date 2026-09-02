use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::cli::{
    Command, ConfigCommand, FactCommand, InitArgs, NoteCommand, PersonCommand, QueryArgs,
    TagCommand,
};
use crate::config::{self, Config};
use crate::db;
use crate::error::{CrmError, Result};
use crate::output::{self, Format};
use crate::query::{self, QueryOptions};
use crate::repository;
use crate::source::ReadOnlySource;

#[derive(Serialize)]
struct Status {
    config_path: PathBuf,
    database_path: PathBuf,
    schema_version: i64,
    source_count: i64,
    daemon_running: bool,
    daemon_pid: Option<i64>,
    queued_jobs: i64,
    running_jobs: i64,
    failed_jobs: i64,
    pending_reviews: i64,
    active_people: i64,
    retired_people: i64,
    migration_people: i64,
}

pub(crate) fn run(format: Format, config: Option<PathBuf>, command: Command) -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Err(CrmError::UnsupportedPlatform);
    }
    let config_path = config.unwrap_or(config::default_config_path()?);
    match command {
        Command::Config {
            command: ConfigCommand::Init(args),
        } => init(format, config_path, args),
        Command::Config {
            command: ConfigCommand::Show,
        } => show_config(format, config_path),
        Command::Doctor => doctor(format, config_path),
        Command::Start => crate::daemon_commands::start(format, config_path),
        Command::Stop => crate::daemon_commands::stop(format, config_path),
        Command::Status => status(format, config_path),
        Command::Review(args) => crate::review_commands::run(format, config_path, args),
        Command::Run(args) => crate::daemon_commands::run_job(format, config_path, args.job),
        Command::Daemon => crate::daemon::run(config_path),
        Command::Person { command } => person(format, config_path, command),
        Command::Note { command } => note(format, config_path, command),
        Command::Fact { command } => fact(format, config_path, command),
        Command::Tag { command } => tag(format, config_path, command),
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
    let status = Status {
        config_path,
        database_path,
        schema_version: db::schema_version(&connection)?,
        source_count: 0,
        daemon_running: false,
        daemon_pid: None,
        queued_jobs: 0,
        running_jobs: 0,
        failed_jobs: 0,
        pending_reviews: 0,
        active_people: 0,
        retired_people: 0,
        migration_people: 1,
    };
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

fn status(format: Format, config_path: PathBuf) -> Result<()> {
    ensure_config(&config_path)?;
    let database_path = config_path.parent().unwrap().join("crm.sqlite3");
    let connection = db::open(&database_path)?;
    let source_count =
        connection.query_row("SELECT COUNT(*) FROM sources", [], |row| row.get(0))?;
    let daemon_pid: Option<i64> = connection.query_row(
        "SELECT pid FROM daemon_state WHERE id=1
         UNION ALL SELECT NULL LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    let daemon_running = daemon_pid.is_some_and(crate::daemon::process_is_running);
    let count = |sql| connection.query_row(sql, [], |row| row.get::<_, i64>(0));
    let status = Status {
        config_path,
        database_path,
        schema_version: db::schema_version(&connection)?,
        source_count,
        daemon_running,
        daemon_pid,
        queued_jobs: count("SELECT COUNT(*) FROM jobs WHERE state='queued'")?,
        running_jobs: count("SELECT COUNT(*) FROM jobs WHERE state='running'")?,
        failed_jobs: crate::jobs::unresolved_failed_count(&connection)?,
        pending_reviews: count("SELECT COUNT(*) FROM review_items WHERE status='pending'")?,
        active_people: count("SELECT COUNT(*) FROM people WHERE lifecycle_state='active'")?,
        retired_people: count("SELECT COUNT(*) FROM people WHERE lifecycle_state='retired'")?,
        migration_people: count(
            "SELECT COUNT(*) FROM people p WHERE lifecycle_state='migration_pending'
             AND NOT EXISTS (SELECT 1 FROM person_merges m WHERE m.source_person_id=p.id)",
        )?,
    };
    output::emit(
        format,
        "status",
        &status,
        format!(
            "daemon         {}{}\nschema version  {}\nsources         {}\npeople          {} active, {} retired, {} migration\njobs            {} queued, {} running, {} failed\nreview          {} pending",
            if status.daemon_running {
                "running"
            } else {
                "stopped"
            },
            status
                .daemon_pid
                .map(|pid| format!(" (PID {pid})"))
                .unwrap_or_default(),
            status.schema_version,
            status.source_count,
            status.active_people,
            status.retired_people,
            status.migration_people,
            status.queued_jobs,
            status.running_jobs,
            status.failed_jobs,
            status.pending_reviews
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

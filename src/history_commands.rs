use std::path::PathBuf;

use rusqlite::{Connection, params};
use serde::Serialize;

use crate::cli::HistoryArgs;
use crate::config::Config;
use crate::db;
use crate::error::{CrmError, Result};
use crate::output::{self, Format};
use crate::repository;

#[derive(Serialize)]
struct History {
    person_id: String,
    channel: Option<String>,
    interactions: Vec<HistoryItem>,
}

#[derive(Serialize)]
struct HistoryItem {
    id: String,
    channel: String,
    kind: String,
    occurred_at: String,
    direction: Option<String>,
    subject: Option<String>,
    body: Option<String>,
}

pub fn run(format: Format, config_path: PathBuf, args: HistoryArgs) -> Result<()> {
    if args.limit == 0 || args.limit > 10_000 {
        return Err(CrmError::InvalidQuery(
            "limit must be between 1 and 10000".into(),
        ));
    }
    let connection = open(&config_path)?;
    let person_id = repository::resolve_person_id(&connection, &args.person)?;
    let interactions = collect(&connection, &person_id, args.channel.as_deref(), args.limit)?;
    let history = History {
        person_id,
        channel: args.channel,
        interactions,
    };
    let table = history
        .interactions
        .iter()
        .map(|item| {
            let text = item
                .subject
                .as_deref()
                .or(item.body.as_deref())
                .unwrap_or("");
            format!(
                "{}  {:<10} {:<8} {}",
                item.occurred_at,
                item.channel,
                item.direction.as_deref().unwrap_or(""),
                text.chars().take(100).collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    output::emit(format, "history", &history, table)
}

fn collect(
    connection: &Connection,
    person_id: &str,
    channel: Option<&str>,
    limit: u32,
) -> Result<Vec<HistoryItem>> {
    let mut statement = connection.prepare(
        "SELECT DISTINCT i.id, i.channel, i.kind, i.occurred_at, i.direction, i.subject, i.body
         FROM interactions i JOIN interaction_participants ip ON ip.interaction_id=i.id
         WHERE ip.person_id=?1 AND i.deleted_at IS NULL AND (?2 IS NULL OR i.channel=?2 COLLATE NOCASE)
         ORDER BY i.occurred_at DESC LIMIT ?3",
    )?;
    Ok(statement
        .query_map(params![person_id, channel, limit], |row| {
            Ok(HistoryItem {
                id: row.get(0)?,
                channel: row.get(1)?,
                kind: row.get(2)?,
                occurred_at: row.get(3)?,
                direction: row.get(4)?,
                subject: row.get(5)?,
                body: row.get(6)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?)
}

fn open(config_path: &std::path::Path) -> Result<Connection> {
    if !config_path.exists() {
        return Err(CrmError::ConfigMissing(config_path.to_path_buf()));
    }
    Config::load(config_path)?;
    let database = config_path
        .parent()
        .ok_or_else(|| CrmError::InvalidConfig("configuration path has no parent".into()))?
        .join("crm.sqlite3");
    db::open(&database)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_filters_channels_case_insensitively() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        let person = repository::create_person(&connection, "Alex", false).unwrap();
        connection
            .execute("INSERT INTO sources(id, kind) VALUES ('test', 'test')", [])
            .unwrap();
        connection.execute(
            "INSERT INTO interactions(id, source_id, native_id, channel, kind, occurred_at, body)
             VALUES ('message', 'test', 'native', 'iMessage', 'message', '2026-01-01', 'hello')",
            [],
        ).unwrap();
        connection
            .execute(
                "INSERT INTO interaction_participants(interaction_id, person_id, role)
             VALUES ('message', ?1, 'sender')",
                [&person.person_id],
            )
            .unwrap();
        let items = collect(&connection, &person.person_id, Some("imessage"), 10).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].body.as_deref(), Some("hello"));
    }
}

mod model;
mod store;

use rusqlite::Connection;
use serde::Serialize;

use self::model::{AnalysisOutput, InputInteraction, InputParticipant, model_input, restore_ids};
use self::store::persist;
use crate::config::Config;
use crate::error::{CrmError, Result};
use crate::ollama::OllamaClient;
use crate::progress::ProgressTracker;

const BATCH_SIZE: usize = 10;

#[derive(Debug, Serialize)]
pub struct AnalysisReport {
    pub selected: usize,
    pub analyzed: usize,
    pub mentions: usize,
    pub relationships: usize,
    pub relationship_signals: usize,
}

pub fn run(
    config: &Config,
    connection: &Connection,
    limit: u32,
    progress: &mut ProgressTracker,
) -> Result<AnalysisReport> {
    if !(1..=100).contains(&limit) {
        return Err(CrmError::InvalidConfig(
            "analysis limit must be between 1 and 100".into(),
        ));
    }
    progress.stage(
        "Selecting interactions for analysis",
        1,
        2,
        1,
        false,
        "query",
    );
    let inputs = pending(connection, limit)?;
    progress.finish_stage("Selected interactions for analysis", 1, 1, false, "query");
    if inputs.is_empty() {
        progress.stage("Analyzing interactions", 2, 2, 0, false, "interactions");
        progress.finish_stage("Analyzed interactions", 0, 0, false, "interactions");
        return Ok(AnalysisReport {
            selected: 0,
            analyzed: 0,
            mentions: 0,
            relationships: 0,
            relationship_signals: 0,
        });
    }
    analyze_batches(config, connection, &inputs, progress)
}

fn analyze_batches(
    config: &Config,
    connection: &Connection,
    inputs: &[InputInteraction],
    progress: &mut ProgressTracker,
) -> Result<AnalysisReport> {
    let client = OllamaClient::new(&config.ollama)?;
    let total = inputs.len() as u64;
    let batch_count = inputs.len().div_ceil(BATCH_SIZE);
    let mut report = AnalysisReport {
        selected: inputs.len(),
        analyzed: 0,
        mentions: 0,
        relationships: 0,
        relationship_signals: 0,
    };
    progress.stage("Analyzing interactions", 2, 2, total, false, "interactions");
    for (batch_index, batch) in inputs.chunks(BATCH_SIZE).enumerate() {
        progress.progress_now(
            format!(
                "Analyzing interactions (batch {} of {batch_count})",
                batch_index + 1
            ),
            report.analyzed as u64,
            total,
            false,
            "interactions",
        );
        let mut output: AnalysisOutput = client.analyze(&model_input(batch))?;
        restore_ids(batch, &mut output)?;
        let summaries: Vec<_> = output
            .items
            .iter()
            .map(|item| item.summary.clone())
            .collect();
        let embeddings = client.embed(&summaries)?;
        let batch_report = persist(config, connection, batch, output, embeddings)?;
        report.analyzed += batch_report.analyzed;
        report.mentions += batch_report.mentions;
        report.relationships += batch_report.relationships;
        report.relationship_signals += batch_report.relationship_signals;
        progress.progress(
            "Analyzing interactions",
            report.analyzed as u64,
            total,
            false,
            "interactions",
        );
    }
    progress.finish_stage("Analyzed interactions", total, total, false, "interactions");
    Ok(report)
}

fn pending(connection: &Connection, limit: u32) -> Result<Vec<InputInteraction>> {
    let mut statement = connection.prepare(
        "SELECT id, channel, occurred_at, direction, subject, body
         FROM interactions
         WHERE analysis_state='pending' AND deleted_at IS NULL AND body IS NOT NULL AND trim(body) != ''
           AND EXISTS (
             SELECT 1 FROM interaction_participants ip JOIN people p ON p.id=ip.person_id
             WHERE ip.interaction_id=interactions.id AND p.lifecycle_state='active'
               AND p.apple_contact_id IS NOT NULL
               AND NOT EXISTS (
                 SELECT 1 FROM identities own
                 WHERE own.person_id=p.id AND own.is_self=1 AND own.active=1
               )
           )
         ORDER BY occurred_at DESC LIMIT ?1",
    )?;
    let mut inputs = statement
        .query_map([limit], |row| {
            let body: String = row.get(5)?;
            Ok(InputInteraction {
                interaction_id: row.get(0)?,
                channel: row.get(1)?,
                occurred_at: row.get(2)?,
                direction: row.get(3)?,
                subject: row.get(4)?,
                body: body.chars().take(6_000).collect(),
                participants: Vec::new(),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for input in &mut inputs {
        input.participants = participants(connection, &input.interaction_id)?;
    }
    Ok(inputs)
}

pub(crate) fn has_pending(connection: &Connection) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM interactions
               WHERE analysis_state='pending' AND deleted_at IS NULL
                 AND body IS NOT NULL AND trim(body) != ''
                 AND EXISTS (
                   SELECT 1 FROM interaction_participants ip JOIN people p ON p.id=ip.person_id
                   WHERE ip.interaction_id=interactions.id AND p.lifecycle_state='active'
                     AND p.apple_contact_id IS NOT NULL
                     AND NOT EXISTS (
                       SELECT 1 FROM identities own
                       WHERE own.person_id=p.id AND own.is_self=1 AND own.active=1
                     )
                 )
             )",
            [],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

fn participants(connection: &Connection, interaction_id: &str) -> Result<Vec<InputParticipant>> {
    let mut statement = connection.prepare(
        "SELECT p.id, p.display_name, GROUP_CONCAT(DISTINCT ip.role)
         FROM interaction_participants ip JOIN people p ON p.id=ip.person_id
         WHERE ip.interaction_id=?1 AND p.lifecycle_state='active'
           AND p.apple_contact_id IS NOT NULL
           AND NOT EXISTS (
             SELECT 1 FROM identities own
             WHERE own.person_id=p.id AND own.is_self=1 AND own.active=1
           )
         GROUP BY p.id, p.display_name
         ORDER BY p.display_name COLLATE NOCASE, p.id",
    )?;
    Ok(statement
        .query_map([interaction_id], |row| {
            Ok(InputParticipant {
                participant_id: row.get(0)?,
                display_name: row.get(1)?,
                role: row.get(2)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_only_interactions_linked_to_active_icloud_people() {
        let directory = tempfile::tempdir().unwrap();
        let connection = crate::db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute_batch(
                "INSERT INTO sources(id, kind) VALUES ('gmail', 'gmail');
                 INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
                 VALUES ('person', 'Alex', 'apple-1', 'active');
                 INSERT INTO interactions(id, source_id, native_id, channel, kind, occurred_at, body)
                 VALUES ('linked', 'gmail', 'linked', 'gmail', 'email', '2026-01-01', 'hello'),
                        ('unknown', 'gmail', 'unknown', 'gmail', 'email', '2026-01-01', 'hello');
                 INSERT INTO interaction_participants(interaction_id, person_id, identity_value, role)
                 VALUES ('linked', 'person', 'alex@example.com', 'sender'),
                        ('unknown', NULL, 'unknown@example.com', 'sender');",
            )
            .unwrap();

        let selected = pending(&connection, 100).unwrap();

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].participants[0].display_name, "Alex");
        assert!(has_pending(&connection).unwrap());
        connection
            .execute(
                "UPDATE interactions SET analysis_state='complete' WHERE id='linked'",
                [],
            )
            .unwrap();
        assert!(!has_pending(&connection).unwrap());
    }
}

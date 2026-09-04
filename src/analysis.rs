mod engine;
mod model;
mod store;

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use self::engine::Analyzer;
use self::model::{AnalysisOutput, InputInteraction, InputParticipant};
use self::store::persist;
use crate::config::Config;
use crate::error::Result;
use crate::progress::ProgressTracker;

const ANALYZABLE_FILTER: &str = "deleted_at IS NULL AND body IS NOT NULL AND trim(body) != ''
    AND EXISTS (
      SELECT 1 FROM interaction_participants ip JOIN people p ON p.id=ip.person_id
      WHERE ip.interaction_id=interactions.id AND p.lifecycle_state='active'
        AND p.apple_contact_id IS NOT NULL
        AND NOT EXISTS (
          SELECT 1 FROM identities own
          WHERE own.person_id=p.id AND own.is_self=1 AND own.active=1
        )
    )";

pub(crate) struct Counts {
    pub total: i64,
    pub analyzed: i64,
}

#[derive(Debug, Serialize)]
pub struct AnalysisReport {
    pub selected: usize,
    pub analyzed: usize,
    pub mentions: usize,
    pub relationship_signals: usize,
}

pub fn run(
    config: &Config,
    connection: &Connection,
    progress: &mut ProgressTracker,
) -> Result<AnalysisReport> {
    progress.stage(
        "Selecting interactions for analysis",
        1,
        2,
        1,
        false,
        "query",
    );
    let ids = pending_ids(connection)?;
    progress.finish_stage("Selected interactions for analysis", 1, 1, false, "query");
    if ids.is_empty() {
        progress.stage("Analyzing interactions", 2, 2, 0, false, "interactions");
        progress.finish_stage("Analyzed interactions", 0, 0, false, "interactions");
        return Ok(AnalysisReport {
            selected: 0,
            analyzed: 0,
            mentions: 0,
            relationship_signals: 0,
        });
    }
    analyze_interactions(config, connection, &ids, progress)
}

fn analyze_interactions(
    config: &Config,
    connection: &Connection,
    ids: &[String],
    progress: &mut ProgressTracker,
) -> Result<AnalysisReport> {
    let analyzer = Analyzer::new(&config.mlx)?;
    let total = ids.len() as u64;
    let mut report = AnalysisReport {
        selected: ids.len(),
        analyzed: 0,
        mentions: 0,
        relationship_signals: 0,
    };
    progress.stage("Analyzing interactions", 2, 2, total, false, "interactions");
    let mut first_error = None;
    let mut processed = 0;
    let mut ready = Vec::with_capacity(config.mlx.embedding_batch_size);
    for (batch_index, ids) in ids.chunks(config.mlx.batch_size).enumerate() {
        let mut inputs = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(input) = pending_interaction(connection, id)? {
                inputs.push(input);
            }
        }
        if inputs.is_empty() {
            continue;
        }
        let start = batch_index * config.mlx.batch_size + 1;
        let end = (start + inputs.len() - 1).min(total as usize);
        progress.progress_now(
            format!("Analyzing interactions {start}-{end} of {total}"),
            processed as u64,
            total,
            false,
            "interactions",
        );
        progress.focus_now(inputs.iter().map(interaction_focus));
        let outputs = analyzer.analyze(&inputs)?;
        for (input, output) in inputs.into_iter().zip(outputs) {
            match output {
                Ok(output) => {
                    ready.push((input, output));
                    if ready.len() == config.mlx.embedding_batch_size {
                        persist_ready(
                            config,
                            connection,
                            &analyzer,
                            &mut ready,
                            &mut report,
                            progress,
                        )?;
                    }
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        processed += ids.len();
        progress.progress(
            "Analyzing interactions",
            processed as u64,
            total,
            false,
            "interactions",
        );
    }
    persist_ready(
        config,
        connection,
        &analyzer,
        &mut ready,
        &mut report,
        progress,
    )?;
    progress.finish_stage(
        "Analyzed interactions",
        report.analyzed as u64,
        total,
        false,
        "interactions",
    );
    first_error.map_or(Ok(report), Err)
}

fn persist_ready(
    config: &Config,
    connection: &Connection,
    analyzer: &Analyzer,
    ready: &mut Vec<(InputInteraction, AnalysisOutput)>,
    report: &mut AnalysisReport,
    progress: &mut ProgressTracker,
) -> Result<()> {
    if ready.is_empty() {
        return Ok(());
    }
    progress.focus_now(ready.iter().map(|(input, _)| interaction_focus(input)));
    let summaries: Vec<_> = ready
        .iter()
        .map(|(_, output)| output.items[0].summary.clone())
        .collect();
    let embeddings = analyzer.embed(&summaries)?;
    for ((input, output), embedding) in ready.drain(..).zip(embeddings) {
        let interaction_report = persist(
            config,
            connection,
            std::slice::from_ref(&input),
            output,
            vec![embedding],
        )?;
        report.analyzed += interaction_report.analyzed;
        report.mentions += interaction_report.mentions;
        report.relationship_signals += interaction_report.relationship_signals;
    }
    Ok(())
}

fn interaction_focus(input: &InputInteraction) -> String {
    let people = input
        .participants
        .iter()
        .map(|participant| participant.display_name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let date = input.occurred_at.get(..10).unwrap_or(&input.occurred_at);
    let context = input
        .subject
        .as_deref()
        .filter(|subject| !subject.trim().is_empty())
        .map(|subject| format!("“{}”", subject.trim()))
        .or_else(|| input.direction.clone())
        .unwrap_or_else(|| "interaction".into());
    format!("{people} · {} · {context} · {date}", input.channel)
}

fn pending_ids(connection: &Connection) -> Result<Vec<String>> {
    let mut statement = connection.prepare(&format!(
        "SELECT id FROM interactions
         WHERE analysis_state='pending' AND {ANALYZABLE_FILTER}
         ORDER BY occurred_at DESC, id"
    ))?;
    statement
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()
        .map_err(Into::into)
}

pub(crate) fn counts(connection: &Connection) -> Result<Counts> {
    connection
        .query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(SUM(analysis_state='complete'), 0)
                 FROM interactions WHERE {ANALYZABLE_FILTER}"
            ),
            [],
            |row| {
                Ok(Counts {
                    total: row.get(0)?,
                    analyzed: row.get(1)?,
                })
            },
        )
        .map_err(Into::into)
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

fn pending_interaction(connection: &Connection, id: &str) -> Result<Option<InputInteraction>> {
    let input = connection
        .query_row(
            "SELECT id, channel, occurred_at, direction, subject, body
             FROM interactions WHERE id=?1 AND analysis_state='pending'
               AND deleted_at IS NULL AND body IS NOT NULL AND trim(body) != ''",
            [id],
            |row| {
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
            },
        )
        .optional()?;
    input
        .map(|mut input| {
            input.participants = participants(connection, &input.interaction_id)?;
            Ok(input)
        })
        .transpose()
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

        let selected = pending_ids(&connection).unwrap();

        assert_eq!(selected.len(), 1);
        let input = pending_interaction(&connection, &selected[0])
            .unwrap()
            .unwrap();
        assert_eq!(input.participants[0].display_name, "Alex");
        assert_eq!(
            interaction_focus(&input),
            "Alex · gmail · interaction · 2026-01-01"
        );
        connection
            .execute(
                "UPDATE interactions SET analysis_state='complete' WHERE id='linked'",
                [],
            )
            .unwrap();
        assert!(pending_ids(&connection).unwrap().is_empty());
        let counts = counts(&connection).unwrap();
        assert_eq!(counts.total, 1);
        assert_eq!(counts.analyzed, 1);
    }

    #[test]
    fn selects_more_than_one_hundred_pending_interactions() {
        let directory = tempfile::tempdir().unwrap();
        let connection = crate::db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute(
                "INSERT INTO sources(id, kind) VALUES ('source', 'test')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
             VALUES ('person', 'Alex', 'apple-1', 'active')",
                [],
            )
            .unwrap();
        for index in 0..205 {
            connection.execute(
                "INSERT INTO interactions(id, source_id, native_id, channel, kind, occurred_at, body)
                 VALUES (?1, 'source', ?1, 'imessage', 'message', '2026-01-01', 'hello')",
                [format!("interaction-{index}")],
            ).unwrap();
            connection
                .execute(
                    "INSERT INTO interaction_participants(interaction_id, person_id, role)
                 VALUES (?1, 'person', 'sender')",
                    [format!("interaction-{index}")],
                )
                .unwrap();
        }

        assert_eq!(pending_ids(&connection).unwrap().len(), 205);
    }
}

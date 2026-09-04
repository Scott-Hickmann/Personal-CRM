use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use super::AnalysisReport;
use super::model::{AnalysisOutput, InputInteraction, OutputMention};
use crate::config::Config;
use crate::error::{CrmError, Result};
use crate::mlx;

pub(super) fn persist(
    config: &Config,
    connection: &Connection,
    inputs: &[InputInteraction],
    output: AnalysisOutput,
    embeddings: Vec<Vec<f64>>,
) -> Result<AnalysisReport> {
    let allowed: HashMap<_, _> = inputs
        .iter()
        .map(|input| (input.interaction_id.as_str(), input))
        .collect();
    let transaction = connection.unchecked_transaction()?;
    let mut report = AnalysisReport {
        selected: inputs.len(),
        analyzed: 0,
        mentions: 0,
        relationship_signals: 0,
    };
    if output.items.len() != embeddings.len() {
        return Err(CrmError::Serialization(format!(
            "embedding count {} does not match analysis count {}",
            embeddings.len(),
            output.items.len()
        )));
    }
    let prompt_hash = mlx::prompt_hash()?;
    for (item, embedding) in output.items.into_iter().zip(embeddings) {
        let input = allowed.get(item.interaction_id.as_str()).ok_or_else(|| {
            CrmError::Serialization(format!(
                "MLX returned unknown interaction id {}",
                item.interaction_id
            ))
        })?;
        transaction.execute(
            "DELETE FROM mentions WHERE interaction_id=?1",
            [&item.interaction_id],
        )?;
        transaction.execute(
            "DELETE FROM relationship_signals WHERE interaction_id=?1",
            [&item.interaction_id],
        )?;
        let is_email = matches!(input.channel.as_str(), "email" | "gmail");
        let is_personal = !is_email || item.is_personal;
        if is_personal {
            for signal in item.relationship_signals {
                transaction.execute(
                    "INSERT INTO relationship_signals(
                       interaction_id, person_id, intimacy, emotional_support, practical_support,
                       affection, shared_activity, conflict_repair, confidence, evidence,
                       model_version, prompt_hash
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        item.interaction_id,
                        signal.participant_id,
                        dimension(signal.intimacy),
                        dimension(signal.emotional_support),
                        dimension(signal.practical_support),
                        dimension(signal.affection),
                        dimension(signal.shared_activity),
                        dimension(signal.conflict_repair),
                        signal.confidence.clamp(0.0, 1.0),
                        signal.evidence.trim().chars().take(500).collect::<String>(),
                        config.mlx.generation_model,
                        prompt_hash,
                    ],
                )?;
                report.relationship_signals += 1;
            }
        }
        let source_people = source_people(&transaction, &item.interaction_id)?;
        for mention in item.mentions {
            report.mentions += persist_mention(&transaction, &item.interaction_id, mention)?;
        }
        persist_summary(
            &transaction,
            config,
            &prompt_hash,
            source_people.first(),
            &item.interaction_id,
            &item.summary,
            &embedding,
        )?;
        if is_email && !is_personal {
            transaction.execute(
                "UPDATE interactions SET body=NULL WHERE id=?1",
                [&item.interaction_id],
            )?;
        }
        transaction.execute(
            "UPDATE interactions SET analysis_state='complete' WHERE id=?1",
            [&item.interaction_id],
        )?;
        report.analyzed += 1;
    }
    transaction.commit()?;
    Ok(report)
}

fn persist_mention(
    connection: &Connection,
    interaction_id: &str,
    mention: OutputMention,
) -> Result<usize> {
    if mention.name.trim().is_empty() {
        return Ok(0);
    }
    let target = resolve_exact_person(connection, &mention.name)?;
    connection.execute(
        "INSERT INTO mentions(id, interaction_id, text, person_id, confidence, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            Uuid::new_v4().to_string(),
            interaction_id,
            mention.name.trim(),
            target,
            mention.confidence.clamp(0.0, 1.0),
            if target.is_some() {
                "resolved"
            } else {
                "unresolved"
            }
        ],
    )?;
    Ok(1)
}

fn persist_summary(
    connection: &Connection,
    config: &Config,
    prompt_hash: &str,
    person_id: Option<&String>,
    interaction_id: &str,
    summary: &str,
    embedding: &[f64],
) -> Result<()> {
    connection.execute(
        "INSERT INTO semantic_chunks(id, person_id, interaction_ids_json, summary, embedding_json, model_version, prompt_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET person_id=excluded.person_id, summary=excluded.summary,
         embedding_json=excluded.embedding_json, model_version=excluded.model_version,
         prompt_hash=excluded.prompt_hash, created_at=CURRENT_TIMESTAMP",
        params![
            format!("interaction:{interaction_id}"),
            person_id,
            serde_json::json!([interaction_id]).to_string(),
            summary,
            serde_json::to_string(embedding).map_err(serialization)?,
            config.mlx.embedding_model,
            prompt_hash
        ],
    )?;
    Ok(())
}

fn source_people(connection: &Connection, interaction_id: &str) -> Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT DISTINCT ip.person_id FROM interaction_participants ip JOIN people p ON p.id=ip.person_id
         WHERE ip.interaction_id=?1 AND ip.person_id IS NOT NULL AND p.lifecycle_state='active'
         AND NOT EXISTS (SELECT 1 FROM identities x WHERE x.person_id=ip.person_id AND x.is_self=1)",
    )?;
    Ok(statement
        .query_map([interaction_id], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?)
}

fn resolve_exact_person(connection: &Connection, name: &str) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT id FROM people WHERE lifecycle_state='active'
             AND display_name=?1 COLLATE NOCASE LIMIT 1",
            [name.trim()],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

fn serialization(error: serde_json::Error) -> CrmError {
    CrmError::Serialization(error.to_string())
}

fn dimension(value: f64) -> f64 {
    value.clamp(0.0, 3.0)
}

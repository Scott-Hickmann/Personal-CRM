use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::Config;
use crate::error::{CrmError, Result};
use crate::ollama::{self, OllamaClient};

#[derive(Debug, Serialize)]
pub struct AnalysisReport {
    pub selected: usize,
    pub analyzed: usize,
    pub mentions: usize,
    pub relationships: usize,
}

#[derive(Debug, Serialize)]
struct AnalysisInput {
    interactions: Vec<InputInteraction>,
}

#[derive(Debug, Clone, Serialize)]
struct InputInteraction {
    interaction_id: String,
    channel: String,
    occurred_at: String,
    direction: Option<String>,
    subject: Option<String>,
    body: String,
}

#[derive(Debug, Deserialize)]
struct AnalysisOutput {
    items: Vec<OutputItem>,
}

#[derive(Debug, Deserialize)]
struct OutputItem {
    interaction_id: String,
    summary: String,
    is_personal: bool,
    mentions: Vec<OutputMention>,
}

#[derive(Debug, Deserialize)]
struct OutputMention {
    name: String,
    confidence: f64,
    relationship_type: String,
}

pub fn run(config: &Config, connection: &Connection, limit: u32) -> Result<AnalysisReport> {
    if !(1..=100).contains(&limit) {
        return Err(CrmError::InvalidConfig(
            "analysis limit must be between 1 and 100".into(),
        ));
    }
    let inputs = pending(connection, limit)?;
    if inputs.is_empty() {
        return Ok(AnalysisReport {
            selected: 0,
            analyzed: 0,
            mentions: 0,
            relationships: 0,
        });
    }
    let client = OllamaClient::new(&config.ollama)?;
    let output: AnalysisOutput = client.analyze(&AnalysisInput {
        interactions: inputs.clone(),
    })?;
    let summaries: Vec<_> = output
        .items
        .iter()
        .map(|item| item.summary.clone())
        .collect();
    let embeddings = client.embed(&summaries)?;
    persist(config, connection, &inputs, output, embeddings)
}

fn pending(connection: &Connection, limit: u32) -> Result<Vec<InputInteraction>> {
    let mut statement = connection.prepare(
        "SELECT id, channel, occurred_at, direction, subject, body
         FROM interactions
         WHERE analysis_state='pending' AND deleted_at IS NULL AND body IS NOT NULL AND trim(body) != ''
         ORDER BY occurred_at DESC LIMIT ?1",
    )?;
    Ok(statement
        .query_map([limit], |row| {
            let body: String = row.get(5)?;
            Ok(InputInteraction {
                interaction_id: row.get(0)?,
                channel: row.get(1)?,
                occurred_at: row.get(2)?,
                direction: row.get(3)?,
                subject: row.get(4)?,
                body: body.chars().take(6_000).collect(),
            })
        })?
        .collect::<std::result::Result<_, _>>()?)
}

fn persist(
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
    let mut mention_count = 0;
    let mut relationship_count = 0;
    let mut analyzed = 0;
    let prompt_hash = ollama::prompt_hash()?;
    for (item, embedding) in output.items.into_iter().zip(embeddings) {
        let Some(input) = allowed.get(item.interaction_id.as_str()) else {
            return Err(CrmError::Serialization(format!(
                "Ollama returned unknown interaction id {}",
                item.interaction_id
            )));
        };
        transaction.execute(
            "DELETE FROM mentions WHERE interaction_id=?1",
            [&item.interaction_id],
        )?;
        let source_people = source_people(&transaction, &item.interaction_id)?;
        for mention in item.mentions {
            if mention.name.trim().is_empty() {
                continue;
            }
            let target = resolve_exact_person(&transaction, &mention.name)?;
            transaction.execute(
                "INSERT INTO mentions(id, interaction_id, text, person_id, confidence, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    Uuid::new_v4().to_string(),
                    item.interaction_id,
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
            mention_count += 1;
            if let Some(target) = target {
                for source in &source_people {
                    if source != &target {
                        upsert_relationship(
                            &transaction,
                            source,
                            &target,
                            &mention,
                            &item.interaction_id,
                            &input.occurred_at,
                            &config.ollama.generation_model,
                        )?;
                        relationship_count += 1;
                    }
                }
            }
        }
        let person_id = source_people.first();
        transaction.execute(
            "INSERT INTO semantic_chunks(id, person_id, interaction_ids_json, summary, embedding_json, model_version, prompt_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET person_id=excluded.person_id, summary=excluded.summary,
             embedding_json=excluded.embedding_json, model_version=excluded.model_version,
             prompt_hash=excluded.prompt_hash, created_at=CURRENT_TIMESTAMP",
            params![format!("interaction:{}", item.interaction_id), person_id,
                serde_json::json!([item.interaction_id]).to_string(), item.summary,
                serde_json::to_string(&embedding).map_err(serialization)?, config.ollama.embedding_model,
                prompt_hash],
        )?;
        if input.channel == "email" && !item.is_personal {
            transaction.execute(
                "UPDATE interactions SET body=NULL WHERE id=?1",
                [&item.interaction_id],
            )?;
        }
        transaction.execute(
            "UPDATE interactions SET analysis_state='complete' WHERE id=?1",
            [&item.interaction_id],
        )?;
        analyzed += 1;
    }
    transaction.commit()?;
    Ok(AnalysisReport {
        selected: inputs.len(),
        analyzed,
        mentions: mention_count,
        relationships: relationship_count,
    })
}

fn source_people(connection: &Connection, interaction_id: &str) -> Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT DISTINCT ip.person_id FROM interaction_participants ip
         WHERE ip.interaction_id=?1 AND ip.person_id IS NOT NULL
         AND NOT EXISTS (SELECT 1 FROM identities x WHERE x.person_id=ip.person_id AND x.is_self=1)",
    )?;
    Ok(statement
        .query_map([interaction_id], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?)
}

fn resolve_exact_person(connection: &Connection, name: &str) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT id FROM people WHERE display_name=?1 COLLATE NOCASE LIMIT 1",
            [name.trim()],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

fn upsert_relationship(
    connection: &Connection,
    source: &str,
    target: &str,
    mention: &OutputMention,
    interaction_id: &str,
    occurred_at: &str,
    model: &str,
) -> Result<()> {
    let relationship_type = relationship_type(&mention.relationship_type);
    let id = stable_id(&format!("{source}\0{target}\0{relationship_type}"));
    let evidence = serde_json::json!([{"interaction_id": interaction_id}]).to_string();
    connection.execute(
        "INSERT INTO relationships(id, source_person_id, target_person_id, relationship_type, confidence,
         status, evidence_json, model_version, first_observed_at, last_observed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'inferred', ?6, ?7, ?8, ?8)
         ON CONFLICT(id) DO UPDATE SET confidence=MAX(confidence, excluded.confidence),
         evidence_json=excluded.evidence_json, model_version=excluded.model_version,
         last_observed_at=MAX(last_observed_at, excluded.last_observed_at)",
        params![id, source, target, relationship_type, mention.confidence.clamp(0.0, 1.0), evidence, model, occurred_at],
    )?;
    Ok(())
}

fn relationship_type(value: &str) -> String {
    let value: String = value
        .trim()
        .to_lowercase()
        .chars()
        .filter(|character| character.is_ascii_alphabetic() || *character == '-')
        .take(40)
        .collect();
    if value.is_empty() {
        "unclear".into()
    } else {
        value
    }
}

fn stable_id(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn serialization(error: serde_json::Error) -> CrmError {
    CrmError::Serialization(error.to_string())
}

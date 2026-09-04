mod evidence;
mod markdown;

use std::collections::HashSet;

use evidence::{Candidate, Prepared, prepare};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::config::Config;
use crate::error::{CrmError, Result};
use crate::mlx::{self, MlxClient};
use crate::progress::ProgressTracker;

const MIN_CLASSIFICATION_CONFIDENCE: f64 = 0.65;
const TYPES: &[&str] = &[
    "partner",
    "parent-child",
    "sibling",
    "relative",
    "friend",
    "coworker",
    "professional",
    "neighbor",
    "classmate",
    "unclear",
];

#[derive(Clone, Serialize)]
pub(super) struct PromptMessage {
    pub role: &'static str,
    pub content: String,
}

#[derive(Debug, Deserialize)]
struct Classification {
    relationship_id: String,
    relationship_type: String,
    confidence: f64,
    evidence_message_ids: Vec<String>,
    evidence: String,
}

pub(crate) fn run(
    config: &Config,
    connection: &Connection,
    progress: &mut ProgressTracker,
) -> Result<usize> {
    let prompt_hash = classification_prompt_hash()?;
    let candidates = candidates(connection, &prompt_hash)?;
    if candidates.is_empty() {
        return Ok(0);
    }
    let template = mlx::prompt_file("classify-relationship.md")?;
    let schema_template: Value =
        serde_json::from_str(&mlx::prompt_file("classify-relationship.schema.json")?)
            .map_err(serialization)?;
    let repair = mlx::prompt_file("repair-relationship-classification.md")?;
    let client = MlxClient::shared(&config.mlx)?;
    let total = candidates.len() as u64;
    let mut completed = 0;
    progress.stage(
        "Classifying relationships",
        1,
        1,
        total,
        false,
        "relationships",
    );
    for batch in candidates.chunks(config.mlx.batch_size) {
        let prepared = batch
            .iter()
            .map(|candidate| prepare(connection, candidate, &template, &schema_template))
            .collect::<Result<Vec<_>>>()?;
        let mut requests = Vec::new();
        let mut active = Vec::new();
        for (candidate, prepared) in batch.iter().zip(prepared) {
            if let Some(prepared) = prepared {
                requests.push(prepared);
                active.push(candidate);
            } else {
                persist_unclear(
                    connection,
                    candidate,
                    &prompt_hash,
                    &config.mlx.generation_model,
                )?;
                completed += 1;
            }
        }
        if !requests.is_empty() {
            progress.focus_now(
                active
                    .iter()
                    .map(|item| format!("{} ↔ {}", item.source_name, item.target_name)),
            );
            let batches = requests
                .iter()
                .map(|item| &item.messages)
                .collect::<Vec<_>>();
            let raw = client.generate(&batches)?;
            for ((candidate, prepared), raw) in active.into_iter().zip(requests).zip(raw) {
                let classification = classify(&client, candidate, prepared, raw, &repair)?;
                persist(
                    connection,
                    candidate,
                    &classification,
                    &prompt_hash,
                    &config.mlx.generation_model,
                )?;
                completed += 1;
            }
        }
        progress.progress(
            "Classifying relationships",
            completed as u64,
            total,
            false,
            "relationships",
        );
    }
    progress.finish_stage(
        "Classified relationships",
        completed as u64,
        total,
        false,
        "relationships",
    );
    Ok(completed)
}

fn classify(
    client: &MlxClient,
    candidate: &Candidate,
    prepared: Prepared,
    raw: String,
    repair: &str,
) -> Result<Classification> {
    match decode(&raw, candidate, &prepared.message_ids) {
        Ok(output) => Ok(output),
        Err(error) => {
            let repair_text = repair
                .replace("{{validation_error}}", &error.to_string())
                .replace("{{relationship_id}}", &candidate.id);
            let mut request = prepared.messages;
            request.push(PromptMessage {
                role: "assistant",
                content: raw,
            });
            request.push(PromptMessage {
                role: "user",
                content: repair_text,
            });
            let repaired = client.generate(&[request])?.pop().unwrap();
            decode(&repaired, candidate, &prepared.message_ids).map_err(|error| {
                CrmError::Serialization(format!(
                    "relationship classification failed after one repair: {error}"
                ))
            })
        }
    }
}

fn candidates(connection: &Connection, prompt_hash: &str) -> Result<Vec<Candidate>> {
    let mut statement = connection.prepare(
        "SELECT r.id, r.source_person_id, r.target_person_id,
                source.display_name, target.display_name, r.structure_revision
         FROM relationships r
         JOIN people source ON source.id=r.source_person_id
         JOIN people target ON target.id=r.target_person_id
         WHERE r.classification_state='pending' OR r.prompt_hash IS NOT ?1
         ORDER BY r.last_observed_at DESC, r.id",
    )?;
    statement
        .query_map([prompt_hash], |row| {
            Ok(Candidate {
                id: row.get(0)?,
                source_id: row.get(1)?,
                target_id: row.get(2)?,
                source_name: row.get(3)?,
                target_name: row.get(4)?,
                structure_revision: row.get(5)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()
        .map_err(Into::into)
}

fn decode(
    raw: &str,
    candidate: &Candidate,
    allowed_ids: &HashSet<String>,
) -> Result<Classification> {
    let output: Classification = serde_json::from_str(raw).map_err(serialization)?;
    if output.relationship_id != candidate.id {
        return Err(CrmError::Serialization(
            "classification changed relationship ID".into(),
        ));
    }
    if !TYPES.contains(&output.relationship_type.as_str()) {
        return Err(CrmError::Serialization(
            "classification returned an unknown type".into(),
        ));
    }
    if !output.confidence.is_finite() || !(0.0..=1.0).contains(&output.confidence) {
        return Err(CrmError::Serialization(
            "classification confidence is invalid".into(),
        ));
    }
    if output.relationship_type != "unclear" && output.confidence < MIN_CLASSIFICATION_CONFIDENCE {
        return Err(CrmError::Serialization(
            "classification confidence is below 0.65".into(),
        ));
    }
    if output.evidence.chars().count() > 300 || output.evidence_message_ids.len() > 3 {
        return Err(CrmError::Serialization(
            "classification evidence is too long".into(),
        ));
    }
    let unique: HashSet<_> = output.evidence_message_ids.iter().collect();
    if unique.len() != output.evidence_message_ids.len()
        || output
            .evidence_message_ids
            .iter()
            .any(|id| !allowed_ids.contains(id))
    {
        return Err(CrmError::Serialization(
            "classification cited an invalid message".into(),
        ));
    }
    Ok(output)
}

fn persist(
    connection: &Connection,
    candidate: &Candidate,
    output: &Classification,
    prompt_hash: &str,
    model: &str,
) -> Result<()> {
    connection.execute(
        "UPDATE relationships SET relationship_type=?2, classification_confidence=?3,
         classification_state='complete', classification_evidence=?4,
         evidence_message_ids_json=?5, model_version=?6, prompt_hash=?7
         WHERE id=?1 AND structure_revision=?8",
        params![
            candidate.id,
            output.relationship_type,
            output.confidence,
            output.evidence.trim(),
            serde_json::to_string(&output.evidence_message_ids).map_err(serialization)?,
            model,
            prompt_hash,
            candidate.structure_revision,
        ],
    )?;
    Ok(())
}

fn persist_unclear(
    connection: &Connection,
    candidate: &Candidate,
    prompt_hash: &str,
    model: &str,
) -> Result<()> {
    persist(
        connection,
        candidate,
        &Classification {
            relationship_id: candidate.id.clone(),
            relationship_type: "unclear".into(),
            confidence: 0.0,
            evidence_message_ids: Vec::new(),
            evidence: String::new(),
        },
        prompt_hash,
        model,
    )
}

fn classification_prompt_hash() -> Result<String> {
    let mut hash = Sha256::new();
    for name in [
        "classify-relationship.md",
        "classify-relationship.schema.json",
        "repair-relationship-classification.md",
    ] {
        hash.update(mlx::prompt_file(name)?);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn serialization(error: serde_json::Error) -> CrmError {
    CrmError::Serialization(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_low_confidence_specific_classification() {
        let candidate = Candidate {
            id: "a:b".into(),
            source_id: "a".into(),
            target_id: "b".into(),
            source_name: "Alex".into(),
            target_name: "Blair".into(),
            structure_revision: 1,
        };
        let allowed = HashSet::from(["message".into()]);
        let raw = r#"{"relationship_id":"a:b","relationship_type":"friend","confidence":0.4,"evidence_message_ids":["message"],"evidence":"They spoke."}"#;
        assert!(decode(raw, &candidate, &allowed).is_err());
    }
}

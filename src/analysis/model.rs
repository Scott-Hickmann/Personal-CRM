use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::error::{CrmError, Result};

#[derive(Debug, Serialize)]
pub(super) struct AnalysisInput {
    interactions: Vec<InputInteraction>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct InputInteraction {
    pub interaction_id: String,
    pub channel: String,
    pub occurred_at: String,
    pub direction: Option<String>,
    pub subject: Option<String>,
    pub body: String,
    pub participants: Vec<InputParticipant>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct InputParticipant {
    pub participant_id: String,
    pub display_name: String,
    pub role: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct AnalysisOutput {
    pub items: Vec<OutputItem>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OutputItem {
    pub interaction_id: String,
    pub summary: String,
    pub is_personal: bool,
    pub mentions: Vec<OutputMention>,
    pub relationship_signals: Vec<OutputRelationshipSignal>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OutputMention {
    pub name: String,
    pub confidence: f64,
    pub relationship_type: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct OutputRelationshipSignal {
    pub participant_id: String,
    pub intimacy: f64,
    pub emotional_support: f64,
    pub practical_support: f64,
    pub affection: f64,
    pub shared_activity: f64,
    pub conflict_repair: f64,
    pub confidence: f64,
    pub evidence: String,
}

pub(super) fn model_input(inputs: &[InputInteraction]) -> AnalysisInput {
    AnalysisInput {
        interactions: inputs
            .iter()
            .enumerate()
            .map(|(index, input)| InputInteraction {
                interaction_id: format!("item-{index}"),
                participants: input
                    .participants
                    .iter()
                    .enumerate()
                    .map(|(participant_index, participant)| InputParticipant {
                        participant_id: format!("participant-{participant_index}"),
                        display_name: participant.display_name.clone(),
                        role: participant.role.clone(),
                    })
                    .collect(),
                ..input.clone()
            })
            .collect(),
    }
}

pub(super) fn restore_ids(inputs: &[InputInteraction], output: &mut AnalysisOutput) -> Result<()> {
    if output.items.len() != inputs.len() {
        return Err(CrmError::Serialization(format!(
            "Ollama returned {} items for {} interactions",
            output.items.len(),
            inputs.len()
        )));
    }
    let mut seen = HashSet::new();
    for item in &mut output.items {
        let index = short_index(&item.interaction_id, "item-", inputs.len()).ok_or_else(|| {
            CrmError::Serialization(format!(
                "Ollama returned unknown interaction id {}",
                item.interaction_id
            ))
        })?;
        if !seen.insert(index) {
            return Err(CrmError::Serialization(format!(
                "Ollama returned duplicate interaction id {}",
                item.interaction_id
            )));
        }
        restore_participant_ids(&inputs[index], item)?;
        item.interaction_id = inputs[index].interaction_id.clone();
    }
    Ok(())
}

fn restore_participant_ids(input: &InputInteraction, item: &mut OutputItem) -> Result<()> {
    let mut seen = HashSet::new();
    for signal in &mut item.relationship_signals {
        let index = short_index(
            &signal.participant_id,
            "participant-",
            input.participants.len(),
        )
        .ok_or_else(|| {
            CrmError::Serialization(format!(
                "Ollama returned unknown participant id {} for {}",
                signal.participant_id, item.interaction_id
            ))
        })?;
        if !seen.insert(index) {
            return Err(CrmError::Serialization(format!(
                "Ollama returned duplicate participant id {} for {}",
                signal.participant_id, item.interaction_id
            )));
        }
        signal.participant_id = input.participants[index].participant_id.clone();
    }
    if seen.len() != input.participants.len() {
        return Err(CrmError::Serialization(format!(
            "Ollama returned {} relationship assessments for {} participants in {}",
            seen.len(),
            input.participants.len(),
            item.interaction_id
        )));
    }
    Ok(())
}

fn short_index(value: &str, prefix: &str, len: usize) -> Option<usize> {
    value
        .strip_prefix(prefix)
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|index| *index < len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(id: &str) -> InputInteraction {
        InputInteraction {
            interaction_id: id.into(),
            channel: "whatsapp".into(),
            occurred_at: "2026-01-01 00:00:00".into(),
            direction: None,
            subject: None,
            body: "hello".into(),
            participants: vec![InputParticipant {
                participant_id: "person-id".into(),
                display_name: "Alex".into(),
                role: "sender".into(),
            }],
        }
    }

    fn output(id: &str) -> OutputItem {
        OutputItem {
            interaction_id: id.into(),
            summary: "hello".into(),
            is_personal: true,
            mentions: Vec::new(),
            relationship_signals: vec![OutputRelationshipSignal {
                participant_id: "participant-0".into(),
                intimacy: 0.0,
                emotional_support: 0.0,
                practical_support: 0.0,
                affection: 0.0,
                shared_activity: 0.0,
                conflict_repair: 0.0,
                confidence: 1.0,
                evidence: String::new(),
            }],
        }
    }

    #[test]
    fn restores_short_model_ids_to_exact_database_ids() {
        let inputs = vec![input("long-uuid-one"), input("long-uuid-two")];
        let model = model_input(&inputs);
        assert_eq!(model.interactions[0].interaction_id, "item-0");

        let mut result = AnalysisOutput {
            items: vec![output("item-1"), output("item-0")],
        };
        restore_ids(&inputs, &mut result).unwrap();

        assert_eq!(result.items[0].interaction_id, "long-uuid-two");
        assert_eq!(
            result.items[0].relationship_signals[0].participant_id,
            "person-id"
        );
    }

    #[test]
    fn rejects_duplicate_short_model_ids() {
        let inputs = vec![input("one"), input("two")];
        let mut result = AnalysisOutput {
            items: vec![output("item-0"), output("item-0")],
        };

        assert!(restore_ids(&inputs, &mut result).is_err());
    }
}

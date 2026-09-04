use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{CrmError, Result};

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

#[derive(Debug, Serialize)]
pub(super) struct ModelInteraction {
    pub interaction_id: String,
    pub channel: String,
    pub occurred_at: String,
    pub direction: Option<String>,
    pub subject: Option<String>,
    pub body: String,
    pub participants: Vec<InputParticipant>,
}

#[derive(Debug)]
pub(super) struct AnalysisOutput {
    pub items: Vec<OutputItem>,
}

#[derive(Debug)]
pub(super) struct OutputItem {
    pub interaction_id: String,
    pub summary: String,
    pub is_personal: bool,
    pub mentions: Vec<OutputMention>,
    pub relationship_signals: Vec<OutputRelationshipSignal>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ContentOutput {
    pub interaction_id: String,
    pub summary: String,
    pub is_personal: bool,
    pub mentions: Vec<OutputMention>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OutputMention {
    pub name: String,
    pub confidence: f64,
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

pub(super) fn model_interaction(input: &InputInteraction) -> ModelInteraction {
    ModelInteraction {
        interaction_id: "item-0".into(),
        channel: input.channel.clone(),
        occurred_at: input.occurred_at.clone(),
        direction: input.direction.clone(),
        subject: input.subject.clone(),
        body: input.body.clone(),
        participants: input
            .participants
            .iter()
            .enumerate()
            .map(|(index, participant)| InputParticipant {
                participant_id: format!("participant-{index}"),
                display_name: participant.display_name.clone(),
                role: participant.role.clone(),
            })
            .collect(),
    }
}

pub(super) fn relationship_input(model: &ModelInteraction, index: usize) -> Value {
    json!({
        "record": {
            "interaction_id": model.interaction_id,
            "channel": model.channel,
            "occurred_at": model.occurred_at,
            "direction": model.direction,
            "subject": model.subject,
            "body": model.body,
        },
        "participant": model.participants[index],
    })
}

pub(super) fn validate_content(model: &ModelInteraction, output: &ContentOutput) -> Result<()> {
    if output.interaction_id != model.interaction_id {
        return Err(serialization(format!(
            "MLX returned content for {} instead of {}",
            output.interaction_id, model.interaction_id
        )));
    }
    for mention in &output.mentions {
        if !in_range(mention.confidence, 0.0, 1.0) {
            return Err(serialization("MLX returned invalid mention confidence"));
        }
    }
    Ok(())
}

pub(super) fn validate_relationship(
    model: &ModelInteraction,
    index: usize,
    output: &OutputRelationshipSignal,
) -> Result<()> {
    let expected = &model.participants[index].participant_id;
    if output.participant_id != *expected {
        return Err(serialization(format!(
            "MLX returned relationship for {} instead of {expected}",
            output.participant_id
        )));
    }
    for (name, value) in [
        ("intimacy", output.intimacy),
        ("emotional_support", output.emotional_support),
        ("practical_support", output.practical_support),
        ("affection", output.affection),
        ("shared_activity", output.shared_activity),
        ("conflict_repair", output.conflict_repair),
    ] {
        if !in_range(value, 0.0, 3.0) || value.fract() != 0.0 {
            return Err(serialization(format!(
                "MLX returned invalid {name} score for {expected}"
            )));
        }
    }
    if !in_range(output.confidence, 0.0, 1.0) {
        return Err(serialization(format!(
            "MLX returned invalid confidence for {expected}"
        )));
    }
    Ok(())
}

pub(super) fn normalize_relationship(output: &mut OutputRelationshipSignal) {
    let all_zero = [
        output.intimacy,
        output.emotional_support,
        output.practical_support,
        output.affection,
        output.shared_activity,
        output.conflict_repair,
    ]
    .into_iter()
    .all(|value| value == 0.0);
    if all_zero {
        output.evidence.clear();
    }
}

pub(super) fn combine(
    input: &InputInteraction,
    content: ContentOutput,
    mut relationships: Vec<OutputRelationshipSignal>,
) -> Result<AnalysisOutput> {
    if relationships.len() != input.participants.len() {
        return Err(serialization(
            "relationship result count changed during analysis",
        ));
    }
    for (index, signal) in relationships.iter_mut().enumerate() {
        signal
            .participant_id
            .clone_from(&input.participants[index].participant_id);
    }
    Ok(AnalysisOutput {
        items: vec![OutputItem {
            interaction_id: input.interaction_id.clone(),
            summary: content.summary,
            is_personal: content.is_personal,
            mentions: content.mentions,
            relationship_signals: relationships,
        }],
    })
}

fn in_range(value: f64, minimum: f64, maximum: f64) -> bool {
    value.is_finite() && value >= minimum && value <= maximum
}

fn serialization(message: impl Into<String>) -> CrmError {
    CrmError::Serialization(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> InputInteraction {
        InputInteraction {
            interaction_id: "database-id".into(),
            channel: "gmail".into(),
            occurred_at: "2026-01-01".into(),
            direction: Some("incoming".into()),
            subject: Some("hello".into()),
            body: "hello".into(),
            participants: vec![InputParticipant {
                participant_id: "person-id".into(),
                display_name: "Alex".into(),
                role: "sender".into(),
            }],
        }
    }

    #[test]
    fn shortens_ids_for_model_calls() {
        let model = model_interaction(&input());

        assert_eq!(model.interaction_id, "item-0");
        assert_eq!(model.participants[0].participant_id, "participant-0");
    }

    #[test]
    fn rejects_relationship_for_the_wrong_participant() {
        let model = model_interaction(&input());
        let output = OutputRelationshipSignal {
            participant_id: "participant-1".into(),
            intimacy: 0.0,
            emotional_support: 0.0,
            practical_support: 0.0,
            affection: 0.0,
            shared_activity: 0.0,
            conflict_repair: 0.0,
            confidence: 1.0,
            evidence: String::new(),
        };

        assert!(validate_relationship(&model, 0, &output).is_err());
    }

    #[test]
    fn clears_evidence_when_every_score_is_zero() {
        let mut output = OutputRelationshipSignal {
            participant_id: "participant-0".into(),
            intimacy: 0.0,
            emotional_support: 0.0,
            practical_support: 0.0,
            affection: 0.0,
            shared_activity: 0.0,
            conflict_repair: 0.0,
            confidence: 1.0,
            evidence: "No scored evidence".into(),
        };

        normalize_relationship(&mut output);

        assert!(output.evidence.is_empty());
    }
}

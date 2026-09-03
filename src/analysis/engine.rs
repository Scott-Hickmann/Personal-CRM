use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::model::{
    AnalysisOutput, ContentOutput, InputInteraction, ModelInteraction, OutputRelationshipSignal,
    combine, model_interaction, normalize_relationship, relationship_input, validate_content,
    validate_relationship,
};
use crate::config::MlxConfig;
use crate::error::{CrmError, Result};
use crate::mlx::{self, MlxClient};

pub(super) struct Analyzer<C = Arc<MlxClient>> {
    client: C,
    assets: Assets,
}

struct Assets {
    content_prompt: String,
    content_repair: String,
    relationship_prompt: String,
    relationship_schema: Value,
    relationship_repair: String,
}

#[derive(Clone, Serialize)]
pub(super) struct Message {
    role: &'static str,
    content: String,
}

struct RelationshipTask {
    model_index: usize,
    participant_index: usize,
    prompt: String,
    input: Value,
}

pub(super) trait ModelClient: Sync {
    fn generate(&self, inputs: &[Vec<Message>]) -> Result<Vec<String>>;
}

impl ModelClient for Arc<MlxClient> {
    fn generate(&self, inputs: &[Vec<Message>]) -> Result<Vec<String>> {
        MlxClient::generate(self, inputs)
    }
}

impl Analyzer<Arc<MlxClient>> {
    pub(super) fn new(config: &MlxConfig) -> Result<Self> {
        Self::from_client(MlxClient::shared(config)?)
    }

    pub(super) fn embed(&self, input: &[String]) -> Result<Vec<Vec<f64>>> {
        self.client.embed(input)
    }
}

impl<C: ModelClient> Analyzer<C> {
    fn from_client(client: C) -> Result<Self> {
        Ok(Self {
            client,
            assets: Assets::load()?,
        })
    }

    pub(super) fn analyze(
        &self,
        inputs: &[InputInteraction],
    ) -> Result<Vec<Result<AnalysisOutput>>> {
        let models: Vec<_> = inputs.iter().map(model_interaction).collect();
        let contents = self.content(&models)?;
        let relationships = self.relationships(&models)?;
        Ok(inputs
            .iter()
            .zip(contents)
            .zip(relationships)
            .map(|((input, content), relationships)| combine(input, content?, relationships?))
            .collect())
    }

    fn content(&self, models: &[ModelInteraction]) -> Result<Vec<Result<ContentOutput>>> {
        let requests: Vec<_> = models
            .iter()
            .map(|model| messages(&self.assets.content_prompt, model))
            .collect::<Result<_>>()?;
        let raw = self.generate(&requests)?;
        let mut results: Vec<Option<Result<ContentOutput>>> =
            (0..models.len()).map(|_| None).collect();
        let mut repairs = Vec::new();
        for (index, (model, raw)) in models.iter().zip(raw).enumerate() {
            match decode("content analysis", &raw, |output| {
                validate_content(model, output)
            }) {
                Ok(output) => results[index] = Some(Ok(output)),
                Err(error) => {
                    let repair =
                        render_repair(&self.assets.content_repair, &error, &model.interaction_id)?;
                    repairs.push((index, repair_messages(&requests[index], raw, repair)));
                }
            }
        }
        if !repairs.is_empty() {
            let requests: Vec<_> = repairs.iter().map(|(_, request)| request.clone()).collect();
            let raw = self.generate(&requests)?;
            for ((index, _), raw) in repairs.into_iter().zip(raw) {
                results[index] = Some(
                    decode("content analysis", &raw, |output| {
                        validate_content(&models[index], output)
                    })
                    .map_err(repair_error("content analysis")),
                );
            }
        }
        Ok(results.into_iter().map(Option::unwrap).collect())
    }

    fn relationships(
        &self,
        models: &[ModelInteraction],
    ) -> Result<Vec<Result<Vec<OutputRelationshipSignal>>>> {
        let mut tasks = Vec::new();
        for (model_index, model) in models.iter().enumerate() {
            for participant_index in 0..model.participants.len() {
                let participant_id = &model.participants[participant_index].participant_id;
                let schema = relationship_schema(&self.assets.relationship_schema, participant_id)?;
                tasks.push(RelationshipTask {
                    model_index,
                    participant_index,
                    prompt: render_prompt(&self.assets.relationship_prompt, &schema)?,
                    input: relationship_input(model, participant_index),
                });
            }
        }
        let requests: Vec<_> = tasks
            .iter()
            .map(|task| messages(&task.prompt, &task.input))
            .collect::<Result<_>>()?;
        if requests.is_empty() {
            return Ok(models.iter().map(|_| Ok(Vec::new())).collect());
        }
        let raw = self.generate(&requests)?;
        let mut results: Vec<Vec<Option<Result<OutputRelationshipSignal>>>> = models
            .iter()
            .map(|model| (0..model.participants.len()).map(|_| None).collect())
            .collect();
        let mut repairs = Vec::new();
        for (task_index, (task, raw)) in tasks.iter().zip(raw).enumerate() {
            let model = &models[task.model_index];
            match decode("relationship analysis", &raw, |output| {
                validate_relationship(model, task.participant_index, output)
            }) {
                Ok(mut output) => {
                    normalize_relationship(&mut output);
                    results[task.model_index][task.participant_index] = Some(Ok(output));
                }
                Err(error) => {
                    let required_id = &model.participants[task.participant_index].participant_id;
                    let repair =
                        render_repair(&self.assets.relationship_repair, &error, required_id)?;
                    repairs.push((
                        task_index,
                        repair_messages(&requests[task_index], raw, repair),
                    ));
                }
            }
        }
        if !repairs.is_empty() {
            let requests: Vec<_> = repairs.iter().map(|(_, request)| request.clone()).collect();
            let raw = self.generate(&requests)?;
            for ((task_index, _), raw) in repairs.into_iter().zip(raw) {
                let task = &tasks[task_index];
                let mut output = decode("relationship analysis", &raw, |output| {
                    validate_relationship(&models[task.model_index], task.participant_index, output)
                })
                .map_err(repair_error("relationship analysis"));
                if let Ok(output) = &mut output {
                    normalize_relationship(output);
                }
                results[task.model_index][task.participant_index] = Some(output);
            }
        }
        Ok(results
            .into_iter()
            .map(|items| items.into_iter().map(Option::unwrap).collect())
            .collect())
    }

    fn generate(&self, requests: &[Vec<Message>]) -> Result<Vec<String>> {
        let output = self.client.generate(requests)?;
        if output.len() != requests.len() {
            return Err(CrmError::Serialization(format!(
                "MLX returned {} generations for {} requests",
                output.len(),
                requests.len()
            )));
        }
        Ok(output)
    }
}

impl Assets {
    fn load() -> Result<Self> {
        let content_schema = schema("analyze-content.schema.json")?;
        Ok(Self {
            content_prompt: render_prompt(
                &mlx::prompt_file("analyze-content.md")?,
                &content_schema,
            )?,
            content_repair: mlx::prompt_file("repair-content.md")?,
            relationship_prompt: mlx::prompt_file("analyze-relationship.md")?,
            relationship_schema: schema("analyze-relationship.schema.json")?,
            relationship_repair: mlx::prompt_file("repair-relationship.md")?,
        })
    }
}

fn messages<T: Serialize>(prompt: &str, input: &T) -> Result<Vec<Message>> {
    Ok(vec![
        Message {
            role: "system",
            content: prompt.into(),
        },
        Message {
            role: "user",
            content: serde_json::to_string(input).map_err(serialization)?,
        },
    ])
}

fn repair_messages(initial: &[Message], invalid: String, repair: String) -> Vec<Message> {
    let mut messages = initial.to_vec();
    messages.push(Message {
        role: "assistant",
        content: invalid,
    });
    messages.push(Message {
        role: "user",
        content: repair,
    });
    messages
}

fn schema(name: &str) -> Result<Value> {
    serde_json::from_str(&mlx::prompt_file(name)?).map_err(serialization)
}

fn relationship_schema(template: &Value, participant_id: &str) -> Result<Value> {
    let mut schema = template.clone();
    let value = schema
        .pointer_mut("/properties/participant_id/const")
        .ok_or_else(|| CrmError::Serialization("relationship schema has no ID const".into()))?;
    *value = json!(participant_id);
    Ok(schema)
}

fn render_prompt(template: &str, schema: &Value) -> Result<String> {
    replace_once(
        template,
        "{{json_schema}}",
        &serde_json::to_string(schema).map_err(serialization)?,
    )
}

fn render_repair(template: &str, error: &CrmError, required_id: &str) -> Result<String> {
    let repair = replace_once(template, "{{validation_error}}", &error.to_string())?;
    replace_once(
        &repair,
        "{{required_manifest}}",
        &json!({"required_id": required_id}).to_string(),
    )
}

fn replace_once(template: &str, marker: &str, value: &str) -> Result<String> {
    if template.matches(marker).count() != 1 {
        return Err(CrmError::Serialization(format!(
            "prompt must contain exactly one {marker} marker"
        )));
    }
    Ok(template.replace(marker, value))
}

fn decode<T, V>(label: &str, raw: &str, validate: V) -> Result<T>
where
    T: DeserializeOwned,
    V: Fn(&T) -> Result<()>,
{
    let output = serde_json::from_str(raw).map_err(|error| {
        CrmError::Serialization(format!("invalid MLX {label} response: {error}"))
    })?;
    validate(&output)?;
    Ok(output)
}

fn repair_error(label: &'static str) -> impl FnOnce(CrmError) -> CrmError {
    move |error| CrmError::Serialization(format!("{label} failed after one repair: {error}"))
}

fn serialization(error: serde_json::Error) -> CrmError {
    CrmError::Serialization(error.to_string())
}

#[cfg(test)]
mod tests;

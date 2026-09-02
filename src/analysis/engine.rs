use std::thread;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::model::{
    AnalysisOutput, ContentOutput, InputInteraction, ModelInteraction, OutputRelationshipSignal,
    combine, model_interaction, normalize_relationship, relationship_input, validate_content,
    validate_relationship,
};
use crate::config::OllamaConfig;
use crate::error::{CrmError, Result};
use crate::ollama::{self, OllamaClient};

const PARTICIPANT_WORKERS: usize = 3;

pub(super) struct Analyzer<C = OllamaClient> {
    client: C,
    assets: Assets,
}

struct Assets {
    content_prompt: String,
    content_schema: Value,
    content_repair: String,
    relationship_prompt: String,
    relationship_schema: Value,
    relationship_repair: String,
}

pub(super) trait ModelClient: Sync {
    fn generate<T: Serialize>(&self, prompt: &str, schema: &Value, input: &T) -> Result<String>;

    fn repair<T: Serialize>(
        &self,
        prompt: &str,
        schema: &Value,
        input: &T,
        invalid: &str,
        repair: &str,
    ) -> Result<String>;
}

impl ModelClient for OllamaClient {
    fn generate<T: Serialize>(&self, prompt: &str, schema: &Value, input: &T) -> Result<String> {
        self.generate_json(prompt, schema, input)
    }

    fn repair<T: Serialize>(
        &self,
        prompt: &str,
        schema: &Value,
        input: &T,
        invalid: &str,
        repair: &str,
    ) -> Result<String> {
        self.repair_json(prompt, schema, input, invalid, repair)
    }
}

impl Analyzer<OllamaClient> {
    pub(super) fn new(config: &OllamaConfig) -> Result<Self> {
        Self::from_client(OllamaClient::new(config)?)
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

    pub(super) fn analyze(&self, input: &InputInteraction) -> Result<AnalysisOutput> {
        let model = model_interaction(input);
        let content = self.content(&model)?;
        let relationships = self.relationships(&model)?;
        combine(input, content, relationships)
    }

    fn content(&self, model: &ModelInteraction) -> Result<ContentOutput> {
        self.stage(
            "content analysis",
            &self.assets.content_prompt,
            &self.assets.content_schema,
            model,
            &self.assets.content_repair,
            &model.interaction_id,
            |output| validate_content(model, output),
        )
    }

    fn relationships(&self, model: &ModelInteraction) -> Result<Vec<OutputRelationshipSignal>> {
        let mut outputs = Vec::with_capacity(model.participants.len());
        for start in (0..model.participants.len()).step_by(PARTICIPANT_WORKERS) {
            let end = (start + PARTICIPANT_WORKERS).min(model.participants.len());
            let batch = thread::scope(|scope| -> Result<Vec<_>> {
                let handles: Vec<_> = (start..end)
                    .map(|index| scope.spawn(move || self.relationship(model, index)))
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle.join().map_err(|_| {
                            CrmError::Serialization("relationship analysis worker panicked".into())
                        })?
                    })
                    .collect()
            })?;
            outputs.extend(batch);
        }
        Ok(outputs)
    }

    fn relationship(
        &self,
        model: &ModelInteraction,
        index: usize,
    ) -> Result<OutputRelationshipSignal> {
        let participant_id = &model.participants[index].participant_id;
        let schema = relationship_schema(&self.assets.relationship_schema, participant_id)?;
        let prompt = render_prompt(&self.assets.relationship_prompt, &schema)?;
        let input = relationship_input(model, index);
        let mut output = self.stage(
            "relationship analysis",
            &prompt,
            &schema,
            &input,
            &self.assets.relationship_repair,
            participant_id,
            |output| validate_relationship(model, index, output),
        )?;
        normalize_relationship(&mut output);
        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    fn stage<T, I, V>(
        &self,
        label: &str,
        prompt: &str,
        schema: &Value,
        input: &I,
        repair_template: &str,
        required_id: &str,
        validate: V,
    ) -> Result<T>
    where
        T: DeserializeOwned,
        I: Serialize,
        V: Fn(&T) -> Result<()>,
    {
        let raw = self.client.generate(prompt, schema, input)?;
        match decode(label, &raw, &validate) {
            Ok(output) => Ok(output),
            Err(first_error) => {
                let repair = render_repair(repair_template, &first_error, required_id)?;
                let repaired = self.client.repair(prompt, schema, input, &raw, &repair)?;
                decode(label, &repaired, &validate).map_err(|error| {
                    CrmError::Serialization(format!("{label} failed after one repair: {error}"))
                })
            }
        }
    }
}

impl Assets {
    fn load() -> Result<Self> {
        let content_schema = schema("analyze-content.schema.json")?;
        Ok(Self {
            content_prompt: render_prompt(
                &ollama::prompt_file("analyze-content.md")?,
                &content_schema,
            )?,
            content_schema,
            content_repair: ollama::prompt_file("repair-content.md")?,
            relationship_prompt: ollama::prompt_file("analyze-relationship.md")?,
            relationship_schema: schema("analyze-relationship.schema.json")?,
            relationship_repair: ollama::prompt_file("repair-relationship.md")?,
        })
    }
}

fn schema(name: &str) -> Result<Value> {
    serde_json::from_str(&ollama::prompt_file(name)?)
        .map_err(|error| CrmError::Serialization(error.to_string()))
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

fn decode<T, V>(label: &str, raw: &str, validate: &V) -> Result<T>
where
    T: DeserializeOwned,
    V: Fn(&T) -> Result<()>,
{
    let output = serde_json::from_str(raw).map_err(|error| {
        CrmError::Serialization(format!("invalid Ollama {label} response: {error}"))
    })?;
    validate(&output)?;
    Ok(output)
}

fn serialization(error: serde_json::Error) -> CrmError {
    CrmError::Serialization(error.to_string())
}

#[cfg(test)]
mod tests;

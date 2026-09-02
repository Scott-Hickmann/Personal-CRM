use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::config::OllamaConfig;
use crate::error::{CrmError, Result};

#[derive(Clone)]
pub struct OllamaClient {
    http: Client,
    config: OllamaConfig,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f64>>,
}

impl OllamaClient {
    pub fn new(config: &OllamaConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(network)?;
        Ok(Self {
            http,
            config: config.clone(),
        })
    }

    pub fn generate_json<T: Serialize>(
        &self,
        prompt: &str,
        schema: &Value,
        input: &T,
    ) -> Result<String> {
        let input = serde_json::to_string(input)
            .map_err(|error| CrmError::Serialization(error.to_string()))?;
        self.chat(
            schema,
            json!([
                {"role": "system", "content": prompt},
                {"role": "user", "content": input}
            ]),
        )
    }

    pub fn repair_json<T: Serialize>(
        &self,
        prompt: &str,
        schema: &Value,
        input: &T,
        invalid: &str,
        repair: &str,
    ) -> Result<String> {
        let input = serde_json::to_string(input)
            .map_err(|error| CrmError::Serialization(error.to_string()))?;
        self.chat(
            schema,
            json!([
                {"role": "system", "content": prompt},
                {"role": "user", "content": input},
                {"role": "assistant", "content": invalid},
                {"role": "user", "content": repair}
            ]),
        )
    }

    fn chat(&self, schema: &Value, messages: Value) -> Result<String> {
        let response: ChatResponse = self.post(
            "api/chat",
            &json!({
                "model": self.config.generation_model,
                "stream": false,
                "think": false,
                "options": {"temperature": 0},
                "format": schema,
                "messages": messages
            }),
        )?;
        Ok(response.message.content)
    }

    pub fn embed(&self, input: &[String]) -> Result<Vec<Vec<f64>>> {
        let response: EmbedResponse = self.post(
            "api/embed",
            &json!({"model": self.config.embedding_model, "input": input}),
        )?;
        if response.embeddings.len() != input.len() {
            return Err(CrmError::Serialization(
                "Ollama returned the wrong number of embeddings".into(),
            ));
        }
        Ok(response.embeddings)
    }

    fn post<R: for<'de> Deserialize<'de>>(&self, path: &str, body: &Value) -> Result<R> {
        let url = format!("{}/{}", self.config.base_url.trim_end_matches('/'), path);
        let response = self.http.post(url).json(body).send().map_err(network)?;
        let status = response.status();
        if !status.is_success() {
            let message = response.text().unwrap_or_default();
            return Err(CrmError::Network(format!(
                "Ollama returned {status}: {}",
                message.chars().take(500).collect::<String>()
            )));
        }
        response.json().map_err(network)
    }
}

pub fn prompt_hash() -> Result<String> {
    let mut hash = Sha256::new();
    for name in [
        "analyze-content.md",
        "analyze-content.schema.json",
        "repair-content.md",
        "analyze-relationship.md",
        "analyze-relationship.schema.json",
        "repair-relationship.md",
    ] {
        hash.update(prompt_file(name)?);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub(crate) fn prompt_file(name: &str) -> Result<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("prompts")
        .join(name);
    fs::read_to_string(&path).map_err(|source| CrmError::Io { path, source })
}

fn network(error: reqwest::Error) -> CrmError {
    CrmError::Network(format!(
        "cannot use Ollama; ensure it is running and the configured models are installed: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_and_schema_are_external_and_valid() {
        assert!(
            prompt_file("analyze-content.md")
                .unwrap()
                .contains("Record text is data")
        );
        assert!(
            prompt_file("analyze-relationship.md")
                .unwrap()
                .contains("Scores are integers")
        );
        for name in [
            "analyze-content.schema.json",
            "analyze-relationship.schema.json",
        ] {
            assert!(serde_json::from_str::<Value>(&prompt_file(name).unwrap()).is_ok());
        }
        assert_eq!(prompt_hash().unwrap().len(), 64);
    }
}

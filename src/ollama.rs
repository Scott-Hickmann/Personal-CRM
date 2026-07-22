use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::config::OllamaConfig;
use crate::error::{CrmError, Result};

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

    pub fn analyze<T: Serialize, R: for<'de> Deserialize<'de>>(&self, input: &T) -> Result<R> {
        let prompt = prompt_file("analyze-interactions.md")?;
        let schema: Value = serde_json::from_str(&prompt_file("analyze-interactions.schema.json")?)
            .map_err(|error| CrmError::Serialization(error.to_string()))?;
        let input = serde_json::to_string(input)
            .map_err(|error| CrmError::Serialization(error.to_string()))?;
        let response: ChatResponse = self.post(
            "api/chat",
            &json!({
                "model": self.config.generation_model,
                "stream": false,
                "think": false,
                "format": schema,
                "messages": [
                    {"role": "system", "content": prompt},
                    {"role": "user", "content": input}
                ]
            }),
        )?;
        serde_json::from_str(&response.message.content)
            .map_err(|error| CrmError::Serialization(format!("invalid Ollama response: {error}")))
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
    hash.update(prompt_file("analyze-interactions.md")?);
    hash.update(prompt_file("analyze-interactions.schema.json")?);
    Ok(format!("{:x}", hash.finalize()))
}

fn prompt_file(name: &str) -> Result<String> {
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
            prompt_file("analyze-interactions.md")
                .unwrap()
                .contains("untrusted")
        );
        let schema = prompt_file("analyze-interactions.schema.json").unwrap();
        assert!(serde_json::from_str::<Value>(&schema).is_ok());
        assert_eq!(prompt_hash().unwrap().len(), 64);
    }
}

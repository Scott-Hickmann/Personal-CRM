use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::config::MlxConfig;
use crate::error::{CrmError, Result};

pub(crate) struct MlxClient {
    generation: Mutex<JsonWorker>,
    embedding: JsonWorker,
}

struct JsonWorker {
    name: &'static str,
    executable: PathBuf,
    arguments: Vec<String>,
    process: Option<WorkerProcess>,
}

struct WorkerProcess {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

#[derive(Serialize)]
struct WorkerRequest<'a, T> {
    inputs: &'a [T],
}

#[derive(serde::Deserialize)]
struct WorkerResponse<T> {
    outputs: Option<T>,
    error: Option<String>,
}

impl MlxClient {
    pub(crate) fn shared(config: &MlxConfig) -> Result<Arc<Self>> {
        static CLIENT: OnceLock<Mutex<Option<(MlxConfig, Arc<MlxClient>)>>> = OnceLock::new();
        validate(config)?;
        let mut cached = CLIENT.get_or_init(|| Mutex::new(None)).lock().unwrap();
        if let Some((current, client)) = cached.as_ref()
            && current == config
        {
            return Ok(Arc::clone(client));
        }
        let client = Arc::new(Self::new(config)?);
        *cached = Some((config.clone(), Arc::clone(&client)));
        Ok(client)
    }

    fn new(config: &MlxConfig) -> Result<Self> {
        let support = support_directory()?;
        let python = support.join("venv/bin/python");
        Ok(Self {
            generation: Mutex::new(JsonWorker::new(
                "generation",
                python,
                vec![
                    support
                        .join("generation_worker.py")
                        .to_string_lossy()
                        .into(),
                    config.generation_model.clone(),
                    config.batch_size.to_string(),
                    config.max_batch_tokens.to_string(),
                    config.max_tokens.to_string(),
                ],
            )),
            embedding: JsonWorker::new(
                "embedding",
                support.join("venv/bin/python"),
                vec![
                    support.join("embedding_worker.py").to_string_lossy().into(),
                    config.embedding_model.clone(),
                ],
            ),
        })
    }

    pub(crate) fn generate<T: Serialize>(&self, inputs: &[T]) -> Result<Vec<String>> {
        let outputs: Vec<String> = self
            .generation
            .lock()
            .unwrap()
            .call(&WorkerRequest { inputs })?;
        ensure_count("generation", inputs.len(), outputs.len())?;
        Ok(outputs)
    }

    pub(crate) fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f64>>> {
        let outputs: Vec<Vec<f64>> = self.embedding.fresh().call(&WorkerRequest { inputs })?;
        ensure_count("embedding", inputs.len(), outputs.len())?;
        Ok(outputs)
    }
}

impl JsonWorker {
    fn new(name: &'static str, executable: PathBuf, arguments: Vec<String>) -> Self {
        Self {
            name,
            executable,
            arguments,
            process: None,
        }
    }

    fn fresh(&self) -> Self {
        Self::new(self.name, self.executable.clone(), self.arguments.clone())
    }

    fn call<I: Serialize, O: DeserializeOwned>(&mut self, request: &I) -> Result<O> {
        if self.process.is_none() {
            self.process = Some(self.spawn()?);
        }
        let result = exchange(
            self.name,
            &self.executable,
            self.process.as_mut().unwrap(),
            request,
        );
        if result.is_err() {
            self.process = None;
        }
        result
    }

    fn spawn(&self) -> Result<WorkerProcess> {
        let mut child = Command::new(&self.executable)
            .args(&self.arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|source| CrmError::Io {
                path: self.executable.clone(),
                source,
            })?;
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Ok(WorkerProcess {
            child,
            input,
            output,
        })
    }
}

fn exchange<I: Serialize, O: DeserializeOwned>(
    name: &str,
    executable: &PathBuf,
    process: &mut WorkerProcess,
    request: &I,
) -> Result<O> {
    serde_json::to_writer(&mut process.input, request).map_err(serialization)?;
    process
        .input
        .write_all(b"\n")
        .map_err(|source| CrmError::Io {
            path: executable.clone(),
            source,
        })?;
    process.input.flush().map_err(|source| CrmError::Io {
        path: executable.clone(),
        source,
    })?;
    let mut line = String::new();
    process
        .output
        .read_line(&mut line)
        .map_err(|source| CrmError::Io {
            path: executable.clone(),
            source,
        })?;
    if line.is_empty() {
        let status = process.child.try_wait().ok().flatten();
        return Err(CrmError::Serialization(format!(
            "MLX {} worker exited before returning a response{}",
            name,
            status
                .map(|value| format!(" ({value})"))
                .unwrap_or_default()
        )));
    }
    let response: WorkerResponse<O> = serde_json::from_str(&line).map_err(serialization)?;
    if let Some(error) = response.error {
        return Err(CrmError::Serialization(format!(
            "MLX {} worker failed: {error}",
            name
        )));
    }
    response
        .outputs
        .ok_or_else(|| CrmError::Serialization(format!("MLX {name} worker returned no output")))
}

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) fn prompt_hash() -> Result<String> {
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

fn support_directory() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("PERSONAL_CRM_MLX_SUPPORT") {
        return Ok(path.into());
    }
    let executable = std::env::current_exe().map_err(|source| CrmError::Io {
        path: PathBuf::from("current executable"),
        source,
    })?;
    let mut path = executable.into_os_string();
    path.push(".support");
    Ok(path.into())
}

fn validate(config: &MlxConfig) -> Result<()> {
    if config.generation_model.trim().is_empty() || config.embedding_model.trim().is_empty() {
        return Err(CrmError::InvalidConfig(
            "MLX model names cannot be empty".into(),
        ));
    }
    if config.batch_size == 0
        || config.max_batch_tokens == 0
        || config.embedding_batch_size == 0
        || config.max_tokens == 0
    {
        return Err(CrmError::InvalidConfig(
            "MLX batch and token limits must be positive".into(),
        ));
    }
    Ok(())
}

fn ensure_count(label: &str, expected: usize, actual: usize) -> Result<()> {
    if expected == actual {
        Ok(())
    } else {
        Err(CrmError::Serialization(format!(
            "MLX {label} worker returned {actual} outputs for {expected} inputs"
        )))
    }
}

fn serialization(error: serde_json::Error) -> CrmError {
    CrmError::Serialization(error.to_string())
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
            assert!(serde_json::from_str::<serde_json::Value>(&prompt_file(name).unwrap()).is_ok());
        }
        assert_eq!(prompt_hash().unwrap().len(), 64);
    }
}

use chrono::Utc;
use serde::Serialize;
use serde_json::{Value, json};

use crate::error::{CrmError, Result};

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum Format {
    Table,
    Json,
    Jsonl,
}

#[derive(Serialize)]
struct Envelope<'a, T> {
    schema_version: &'static str,
    command: &'a str,
    data: T,
    meta: Value,
    warnings: Vec<String>,
}

pub fn emit<T: Serialize>(format: Format, command: &str, data: T, table: String) -> Result<()> {
    match format {
        Format::Table => println!("{table}"),
        Format::Json => {
            let envelope = Envelope {
                schema_version: "1",
                command,
                data,
                meta: json!({"generated_at": Utc::now()}),
                warnings: Vec::new(),
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&envelope)
                    .map_err(|error| CrmError::Serialization(error.to_string()))?
            );
        }
        Format::Jsonl => {
            println!(
                "{}",
                serde_json::to_string(&data)
                    .map_err(|error| CrmError::Serialization(error.to_string()))?
            );
        }
    }
    Ok(())
}

pub fn print_error(error: &CrmError) {
    eprintln!(
        "{}",
        json!({
            "schema_version": "1",
            "error": {"code": error.code(), "message": error.to_string()}
        })
    );
}

use std::fs;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::{CrmError, Result};

const HELPER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/export-contacts.swift");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppleContainer {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppleContact {
    pub id: String,
    pub name_prefix: String,
    pub given_name: String,
    pub middle_name: String,
    pub family_name: String,
    pub name_suffix: String,
    pub nickname: String,
    pub emails: Vec<LabeledValue>,
    pub phones: Vec<LabeledValue>,
    pub organization: String,
    pub department: String,
    pub job_title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabeledValue {
    pub label: Option<String>,
    pub value: String,
}

pub fn containers() -> Result<Vec<AppleContainer>> {
    run_helper(&["containers"])
}

pub fn contacts(container_id: &str) -> Result<Vec<AppleContact>> {
    run_helper(&["export", container_id])
}

fn run_helper<T: for<'de> Deserialize<'de>>(arguments: &[&str]) -> Result<T> {
    let cache = std::env::temp_dir().join("personal-crm-swift-module-cache");
    fs::create_dir_all(&cache).map_err(|source| CrmError::Io {
        path: cache.clone(),
        source,
    })?;
    let response = Command::new("xcrun")
        .args(["swift", HELPER])
        .args(arguments)
        .env("CLANG_MODULE_CACHE_PATH", &cache)
        .env("SWIFT_MODULECACHE_PATH", &cache)
        .output()
        .map_err(|error| CrmError::Contacts(format!("could not start Contacts helper: {error}")))?;
    if !response.status.success() {
        let message = String::from_utf8_lossy(&response.stderr).trim().to_owned();
        return Err(CrmError::Contacts(if message.is_empty() {
            "Contacts helper failed without an error message".into()
        } else {
            message
        }));
    }
    serde_json::from_slice(&response.stdout)
        .map_err(|error| CrmError::Contacts(format!("invalid Contacts helper response: {error}")))
}

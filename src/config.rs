use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CrmError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub self_identity: SelfIdentity,
    #[serde(default)]
    pub mlx: MlxConfig,
    #[serde(default)]
    pub gmail: GmailConfig,
    #[serde(default)]
    pub contact_publish: ContactPublishConfig,
    #[serde(default)]
    pub paths: SourcePaths,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfIdentity {
    pub name: String,
    #[serde(default)]
    pub apple_contact_id: Option<String>,
    #[serde(default)]
    pub emails: Vec<String>,
    #[serde(default)]
    pub phones: Vec<String>,
    #[serde(default)]
    pub whatsapp_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MlxConfig {
    pub generation_model: String,
    pub embedding_model: String,
    pub batch_size: usize,
    pub max_tokens: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GmailConfig {
    pub credentials_path: Option<PathBuf>,
    #[serde(default)]
    pub accounts: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContactPublishConfig {
    pub credentials_path: Option<PathBuf>,
    #[serde(default)]
    pub account_credentials: BTreeMap<String, PathBuf>,
    #[serde(default)]
    pub accounts: Vec<String>,
    pub source_container: Option<String>,
    pub personal_account: Option<String>,
    pub workspace_account: Option<String>,
    #[serde(default)]
    pub work_domains: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourcePaths {
    pub contacts: Option<PathBuf>,
    pub imessage: Option<PathBuf>,
    pub whatsapp: Option<PathBuf>,
    pub apple_calls: Option<PathBuf>,
    pub whatsapp_calls: Option<PathBuf>,
}

impl Default for MlxConfig {
    fn default() -> Self {
        Self {
            generation_model: "mlx-community/gemma-4-E4B-it-qat-4bit".into(),
            embedding_model: "mlx-community/embeddinggemma-300m-bf16".into(),
            batch_size: 8,
            max_tokens: 512,
        }
    }
}

impl Config {
    pub fn new(name: String, emails: Vec<String>, phones: Vec<String>) -> Result<Self> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(CrmError::InvalidConfig("self name cannot be empty".into()));
        }
        Ok(Self {
            self_identity: SelfIdentity {
                name,
                apple_contact_id: None,
                emails: normalize_strings(emails),
                phones: normalize_strings(phones),
                whatsapp_ids: Vec::new(),
            },
            mlx: MlxConfig::default(),
            gmail: GmailConfig::default(),
            contact_publish: ContactPublishConfig::default(),
            paths: SourcePaths::discover()?,
        })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).map_err(|source| CrmError::Io {
            path: path.to_owned(),
            source,
        })?;
        toml::from_str(&text).map_err(|error| CrmError::InvalidConfig(error.to_string()))
    }

    pub fn save_new(&self, path: &Path) -> Result<()> {
        if path.exists() {
            return Err(CrmError::ConfigExists(path.to_owned()));
        }
        let parent = path.parent().ok_or_else(|| {
            CrmError::InvalidConfig("configuration path has no parent directory".into())
        })?;
        fs::create_dir_all(parent).map_err(|source| CrmError::Io {
            path: parent.to_owned(),
            source,
        })?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(|source| {
            CrmError::Io {
                path: parent.to_owned(),
                source,
            }
        })?;
        self.save(path)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self)
            .map_err(|error| CrmError::Serialization(error.to_string()))?;
        fs::write(path, text).map_err(|source| CrmError::Io {
            path: path.to_owned(),
            source,
        })?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|source| {
            CrmError::Io {
                path: path.to_owned(),
                source,
            }
        })
    }
}

impl SourcePaths {
    fn discover() -> Result<Self> {
        let home = dirs::home_dir()
            .ok_or_else(|| CrmError::InvalidConfig("cannot determine home directory".into()))?;
        let whatsapp = home.join("Library/Group Containers/group.net.whatsapp.WhatsApp.shared");
        Ok(Self {
            contacts: Some(
                home.join("Library/Application Support/AddressBook/AddressBook-v22.abcddb"),
            ),
            imessage: Some(home.join("Library/Messages/chat.db")),
            whatsapp: Some(whatsapp.join("ChatStorage.sqlite")),
            apple_calls: Some(
                home.join("Library/Application Support/CallHistoryDB/CallHistory.storedata"),
            ),
            whatsapp_calls: Some(whatsapp.join("CallHistory.sqlite")),
        })
    }
}

pub fn default_data_dir() -> Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| CrmError::InvalidConfig("cannot determine home directory".into()))?;
    Ok(home.join("Library/Application Support/crm"))
}

pub fn default_config_path() -> Result<PathBuf> {
    Ok(default_data_dir()?.join("config.toml"))
}

fn normalize_strings(values: Vec<String>) -> Vec<String> {
    let mut values: Vec<_> = values
        .into_iter()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
        .collect();
    values.sort();
    values.dedup();
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_self_identifiers() {
        let config = Config::new(
            " Scott Hickmann ".into(),
            vec!["Scott@Example.com".into(), "scott@example.com".into()],
            vec![" +1 555 123 4567 ".into()],
        )
        .unwrap();
        assert_eq!(config.self_identity.name, "Scott Hickmann");
        assert_eq!(config.self_identity.emails, ["scott@example.com"]);
        assert_eq!(config.self_identity.phones, ["+1 555 123 4567"]);
        assert_eq!(
            config.mlx.generation_model,
            "mlx-community/gemma-4-E4B-it-qat-4bit"
        );
        assert_eq!(config.mlx.batch_size, 8);
    }
}

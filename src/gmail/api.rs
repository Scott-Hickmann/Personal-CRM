use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::error::{CrmError, Result};

#[derive(Debug, Clone, Deserialize)]
pub struct CredentialsFile {
    pub installed: Credentials,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Credentials {
    pub client_id: String,
    pub client_secret: String,
    pub auth_uri: String,
    pub token_uri: String,
}

impl Credentials {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).map_err(|source| CrmError::Io {
            path: path.to_owned(),
            source,
        })?;
        let file: CredentialsFile = serde_json::from_str(&text).map_err(|error| {
            CrmError::Authentication(format!("invalid OAuth credentials: {error}"))
        })?;
        Ok(file.installed)
    }
}

pub enum ApiResponse<T> {
    Data(T),
    NotFound,
}

pub struct ApiClient {
    http: Client,
    access_token: String,
}

impl ApiClient {
    pub fn for_account(credentials: &Credentials, account: &str) -> Result<Self> {
        let refresh_token = keyring::Entry::new("personal-crm.gmail", account)
            .map_err(keyring_error)?
            .get_password()
            .map_err(keyring_error)?;
        let http = Client::builder().build().map_err(network_error)?;
        let token: TokenResponse = http
            .post(&credentials.token_uri)
            .form(&[
                ("client_id", credentials.client_id.as_str()),
                ("client_secret", credentials.client_secret.as_str()),
                ("refresh_token", refresh_token.as_str()),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .map_err(network_error)?
            .error_for_status()
            .map_err(network_error)?
            .json()
            .map_err(network_error)?;
        Ok(Self {
            http,
            access_token: token.access_token,
        })
    }

    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<ApiResponse<T>> {
        let url = if path.starts_with("http") {
            path.to_owned()
        } else {
            format!("https://gmail.googleapis.com/gmail/v1/users/me/{path}")
        };
        for attempt in 0..5 {
            let response = match self.http.get(&url).bearer_auth(&self.access_token).send() {
                Ok(response) => response,
                Err(_) if attempt < 4 => {
                    thread::sleep(retry_delay(attempt));
                    continue;
                }
                Err(error) => return Err(network_error(error)),
            };
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(ApiResponse::NotFound);
            }
            if (response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
                || response.status().is_server_error())
                && attempt < 4
            {
                thread::sleep(retry_delay(attempt));
                continue;
            }
            return Ok(ApiResponse::Data(
                response
                    .error_for_status()
                    .map_err(network_error)?
                    .json()
                    .map_err(network_error)?,
            ));
        }
        unreachable!("bounded Gmail retry loop always returns")
    }
}

fn retry_delay(attempt: u32) -> Duration {
    Duration::from_secs(1_u64 << attempt)
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageList {
    #[serde(default)]
    pub messages: Vec<MessageRef>,
    pub next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MessageRef {
    pub id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailMessage {
    pub id: String,
    pub thread_id: String,
    #[serde(default)]
    pub label_ids: Vec<String>,
    pub internal_date: String,
    pub payload: MessagePart,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePart {
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub headers: Vec<Header>,
    #[serde(default)]
    pub body: MessageBody,
    #[serde(default)]
    pub parts: Vec<MessagePart>,
}

#[derive(Debug, Deserialize)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageBody {
    #[serde(default)]
    pub size: i64,
    pub data: Option<String>,
    pub attachment_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    #[serde(default)]
    pub history: Vec<History>,
    pub next_page_token: Option<String>,
    pub history_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    #[serde(default)]
    pub messages_added: Vec<HistoryMessage>,
    #[serde(default)]
    pub messages_deleted: Vec<HistoryMessage>,
    #[serde(default)]
    pub labels_added: Vec<HistoryMessage>,
    #[serde(default)]
    pub labels_removed: Vec<HistoryMessage>,
}

#[derive(Debug, Deserialize)]
pub struct HistoryMessage {
    pub message: MessageRef,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Profile {
    #[serde(rename = "emailAddress")]
    pub email_address: String,
    #[serde(default, rename = "historyId")]
    pub history_id: String,
}

pub(crate) fn network_error(error: reqwest::Error) -> CrmError {
    CrmError::Network(error.to_string())
}
pub(crate) fn keyring_error(error: keyring::Error) -> CrmError {
    CrmError::Authentication(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_profile_history_checkpoint() {
        let profile: Profile =
            serde_json::from_str(r#"{"emailAddress":"me@example.com","historyId":"20280"}"#)
                .unwrap();
        assert_eq!(profile.history_id, "20280");
    }
}

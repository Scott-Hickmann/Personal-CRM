use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

use crate::config::Config;
use crate::error::{CrmError, Result};
use crate::gmail::Credentials;

const KEYRING_SERVICE: &str = "personal-crm.google-contacts";
const SCOPES: &str =
    "https://www.googleapis.com/auth/contacts https://www.googleapis.com/auth/userinfo.email";

#[derive(Debug, Serialize)]
pub struct AuthorizedAccount {
    pub email: String,
}

pub fn authorize(
    config: &mut Config,
    config_path: &Path,
    credentials_path: Option<&Path>,
) -> Result<AuthorizedAccount> {
    let path = resolve_credentials(config, credentials_path)?;
    let credentials = Credentials::load(&path)?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|source| CrmError::Io {
        path: path.clone(),
        source,
    })?;
    let redirect_uri = format!(
        "http://127.0.0.1:{}",
        listener
            .local_addr()
            .map_err(|error| CrmError::Authentication(error.to_string()))?
            .port()
    );
    let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let state = Uuid::new_v4().to_string();
    let mut url = Url::parse(&credentials.auth_uri)
        .map_err(|error| CrmError::Authentication(error.to_string()))?;
    url.query_pairs_mut()
        .append_pair("client_id", &credentials.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPES)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state);
    eprintln!("Authorize Google Contacts in your browser: {url}");
    let _ = Command::new("open").arg(url.as_str()).status();

    let (mut stream, _) = listener
        .accept()
        .map_err(|error| CrmError::Authentication(error.to_string()))?;
    let mut buffer = [0_u8; 8192];
    let read = stream
        .read(&mut buffer)
        .map_err(|error| CrmError::Authentication(error.to_string()))?;
    let callback = callback_url(&String::from_utf8_lossy(&buffer[..read]), &redirect_uri)?;
    let values: std::collections::HashMap<_, _> = callback.query_pairs().into_owned().collect();
    if values.get("state") != Some(&state) {
        return Err(CrmError::Authentication("OAuth state mismatch".into()));
    }
    let code = values
        .get("code")
        .ok_or_else(|| CrmError::Authentication("OAuth callback omitted code".into()))?;
    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 55\r\n\r\nGoogle Contacts authorized. You can close this window.\n");

    let http = Client::builder().build().map_err(network_error)?;
    let token: TokenResponse = http
        .post(&credentials.token_uri)
        .form(&[
            ("client_id", credentials.client_id.as_str()),
            ("client_secret", credentials.client_secret.as_str()),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri.as_str()),
        ])
        .send()
        .map_err(network_error)?
        .error_for_status()
        .map_err(network_error)?
        .json()
        .map_err(network_error)?;
    let identity: UserInfo = http
        .get("https://www.googleapis.com/oauth2/v2/userinfo")
        .bearer_auth(&token.access_token)
        .send()
        .map_err(network_error)?
        .error_for_status()
        .map_err(network_error)?
        .json()
        .map_err(network_error)?;
    let email = identity.email.trim().to_lowercase();
    let refresh_token = token.refresh_token.ok_or_else(|| {
        CrmError::Authentication("Google did not return a Contacts refresh token".into())
    })?;
    keyring::Entry::new(KEYRING_SERVICE, &email)
        .map_err(keyring_error)?
        .set_password(&refresh_token)
        .map_err(keyring_error)?;
    config.contact_publish.credentials_path = Some(path.clone());
    config
        .contact_publish
        .account_credentials
        .insert(email.clone(), path);
    if !config.contact_publish.accounts.contains(&email) {
        config.contact_publish.accounts.push(email.clone());
        config.contact_publish.accounts.sort();
    }
    config.save(config_path)?;
    Ok(AuthorizedAccount { email })
}

pub fn list_accounts(config: &Config) -> &[String] {
    &config.contact_publish.accounts
}

pub fn remove_account(config: &mut Config, config_path: &Path, account: &str) -> Result<()> {
    let account = account.trim().to_lowercase();
    keyring::Entry::new(KEYRING_SERVICE, &account)
        .map_err(keyring_error)?
        .delete_credential()
        .map_err(keyring_error)?;
    config
        .contact_publish
        .accounts
        .retain(|candidate| candidate != &account);
    config.contact_publish.account_credentials.remove(&account);
    if config.contact_publish.personal_account.as_deref() == Some(&account) {
        config.contact_publish.personal_account = None;
    }
    if config.contact_publish.workspace_account.as_deref() == Some(&account) {
        config.contact_publish.workspace_account = None;
    }
    config.save(config_path)
}

fn resolve_credentials(config: &Config, explicit: Option<&Path>) -> Result<PathBuf> {
    explicit
        .map(Path::to_owned)
        .or_else(|| config.contact_publish.credentials_path.clone())
        .or_else(|| config.gmail.credentials_path.clone())
        .ok_or_else(|| {
            CrmError::Authentication(
                "OAuth credentials are required; pass --credentials or authorize Gmail first"
                    .into(),
            )
        })
}

fn callback_url(request: &str, redirect_uri: &str) -> Result<Url> {
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| CrmError::Authentication("invalid OAuth callback".into()))?;
    Url::parse(&format!("{redirect_uri}{path}"))
        .map_err(|error| CrmError::Authentication(error.to_string()))
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct UserInfo {
    email: String,
}

fn network_error(error: reqwest::Error) -> CrmError {
    CrmError::Network(error.to_string())
}

fn keyring_error(error: keyring::Error) -> CrmError {
    CrmError::Authentication(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_oauth_callback() {
        let url = callback_url(
            "GET /?code=abc&state=xyz HTTP/1.1\r\n",
            "http://127.0.0.1:1234",
        )
        .unwrap();
        assert_eq!(
            url.query_pairs().find(|(key, _)| key == "code").unwrap().1,
            "abc"
        );
    }
}

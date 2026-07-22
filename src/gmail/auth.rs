use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;

use base64::Engine;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

use super::api::{Credentials, Profile, keyring_error, network_error};
use crate::config::Config;
use crate::error::{CrmError, Result};

#[derive(Debug, Serialize)]
pub struct AuthorizedAccount {
    pub email: String,
}

pub fn authorize(
    config: &mut Config,
    config_path: &Path,
    credentials_path: &Path,
) -> Result<AuthorizedAccount> {
    let credentials = Credentials::load(credentials_path)?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|source| CrmError::Io {
        path: credentials_path.to_owned(),
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
        .append_pair("scope", "https://www.googleapis.com/auth/gmail.readonly")
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state);
    eprintln!("Authorize Gmail in your browser: {url}");
    let _ = Command::new("open").arg(url.as_str()).status();
    let (mut stream, _) = listener
        .accept()
        .map_err(|error| CrmError::Authentication(error.to_string()))?;
    let mut buffer = [0_u8; 8192];
    let read = stream
        .read(&mut buffer)
        .map_err(|error| CrmError::Authentication(error.to_string()))?;
    let request = String::from_utf8_lossy(&buffer[..read]);
    let callback = callback_url(&request, &redirect_uri)?;
    let values: std::collections::HashMap<_, _> = callback.query_pairs().into_owned().collect();
    if values.get("state") != Some(&state) {
        return Err(CrmError::Authentication("OAuth state mismatch".into()));
    }
    let code = values
        .get("code")
        .ok_or_else(|| CrmError::Authentication("OAuth callback omitted code".into()))?;
    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 44\r\n\r\nGmail authorized. You can close this window.\n");
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
    let profile: Profile = http
        .get("https://gmail.googleapis.com/gmail/v1/users/me/profile")
        .bearer_auth(&token.access_token)
        .send()
        .map_err(network_error)?
        .error_for_status()
        .map_err(network_error)?
        .json()
        .map_err(network_error)?;
    keyring::Entry::new("personal-crm.gmail", &profile.email_address)
        .map_err(keyring_error)?
        .set_password(&token.refresh_token.ok_or_else(|| {
            CrmError::Authentication("Google did not return a refresh token".into())
        })?)
        .map_err(keyring_error)?;
    config.gmail.credentials_path = Some(credentials_path.to_owned());
    if !config.gmail.accounts.contains(&profile.email_address) {
        config.gmail.accounts.push(profile.email_address.clone());
        config.gmail.accounts.sort();
    }
    config.save(config_path)?;
    Ok(AuthorizedAccount {
        email: profile.email_address,
    })
}

pub fn list_accounts(config: &Config) -> &[String] {
    &config.gmail.accounts
}

pub fn remove_account(config: &mut Config, config_path: &Path, account: &str) -> Result<()> {
    keyring::Entry::new("personal-crm.gmail", account)
        .map_err(keyring_error)?
        .delete_credential()
        .map_err(keyring_error)?;
    config
        .gmail
        .accounts
        .retain(|candidate| candidate != account);
    config.save(config_path)
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
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

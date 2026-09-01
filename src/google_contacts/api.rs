use reqwest::blocking::{Client as HttpClient, RequestBuilder};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use url::Url;

use crate::error::{CrmError, Result};
use crate::gmail::Credentials;

const API_ROOT: &str = "https://people.googleapis.com/v1";
const KEYRING_SERVICE: &str = "personal-crm.google-contacts";
pub const PERSON_FIELDS: &str =
    "names,nicknames,emailAddresses,phoneNumbers,organizations,clientData,metadata";
pub const UPDATE_FIELDS: &str =
    "names,nicknames,emailAddresses,phoneNumbers,organizations,clientData";

pub struct Client {
    http: HttpClient,
    access_token: String,
    root: String,
}

impl Client {
    pub fn for_account(credentials: &Credentials, account: &str) -> Result<Self> {
        let refresh_token = keyring::Entry::new(KEYRING_SERVICE, account)
            .map_err(keyring_error)?
            .get_password()
            .map_err(keyring_error)?;
        let http = HttpClient::builder().build().map_err(network_error)?;
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
            root: API_ROOT.into(),
        })
    }

    pub fn list(&self) -> Result<Vec<Person>> {
        let mut people = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut url = Url::parse(&format!("{}/people/me/connections", self.root))
                .map_err(|error| CrmError::Network(error.to_string()))?;
            url.query_pairs_mut()
                .append_pair("pageSize", "1000")
                .append_pair("personFields", PERSON_FIELDS)
                .append_pair("sources", "READ_SOURCE_TYPE_CONTACT");
            if let Some(token) = &page_token {
                url.query_pairs_mut().append_pair("pageToken", token);
            }
            let page: Connections = self.send(self.http.get(url))?;
            people.extend(page.connections);
            if page.next_page_token.is_none() {
                break;
            }
            page_token = page.next_page_token;
        }
        Ok(people)
    }

    pub fn create(&self, person: &Person) -> Result<Person> {
        let mut url = Url::parse(&format!("{}/people:createContact", self.root))
            .map_err(|error| CrmError::Network(error.to_string()))?;
        url.query_pairs_mut()
            .append_pair("personFields", PERSON_FIELDS);
        self.send(self.http.post(url).json(person))
    }

    pub fn get(&self, resource_name: &str) -> Result<Person> {
        let mut url = Url::parse(&format!("{}/{resource_name}", self.root))
            .map_err(|error| CrmError::Network(error.to_string()))?;
        url.query_pairs_mut()
            .append_pair("personFields", PERSON_FIELDS)
            .append_pair("sources", "READ_SOURCE_TYPE_CONTACT");
        self.send(self.http.get(url))
    }

    pub fn update(&self, person: &Person) -> Result<Person> {
        let resource = person.resource_name.as_deref().ok_or_else(|| {
            CrmError::Contacts("Google contact is missing its resource name".into())
        })?;
        let mut url = Url::parse(&format!("{}/{resource}:updateContact", self.root))
            .map_err(|error| CrmError::Network(error.to_string()))?;
        url.query_pairs_mut()
            .append_pair("updatePersonFields", UPDATE_FIELDS)
            .append_pair("personFields", PERSON_FIELDS);
        self.send(self.http.patch(url).json(person))
    }

    pub fn delete(&self, resource_name: &str) -> Result<()> {
        let url = format!("{}/{resource_name}:deleteContact", self.root);
        let response = self
            .http
            .delete(url)
            .bearer_auth(&self.access_token)
            .send()
            .map_err(network_error)?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        ensure_success(response)?;
        Ok(())
    }

    fn send<T: DeserializeOwned>(&self, request: RequestBuilder) -> Result<T> {
        let response = request
            .bearer_auth(&self.access_token)
            .send()
            .map_err(network_error)?;
        ensure_success(response)?.json().map_err(network_error)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<PersonMetadata>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<Name>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nicknames: Vec<Nickname>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub email_addresses: Vec<TypedValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phone_numbers: Vec<TypedValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub organizations: Vec<Organization>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub client_data: Vec<ClientData>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersonMetadata {
    #[serde(default)]
    pub sources: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Name {
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub honorific_prefix: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub given_name: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub middle_name: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub family_name: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub honorific_suffix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Nickname {
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypedValue {
    pub value: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Organization {
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub department: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientData {
    pub key: String,
    pub value: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Connections {
    #[serde(default)]
    connections: Vec<Person>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

fn network_error(error: reqwest::Error) -> CrmError {
    CrmError::Network(error.to_string())
}

fn ensure_success(response: reqwest::blocking::Response) -> Result<reqwest::blocking::Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let message = response.text().unwrap_or_default();
    Err(CrmError::Network(format!(
        "Google People API returned {status}: {message}"
    )))
}

fn keyring_error(error: keyring::Error) -> CrmError {
    CrmError::Authentication(error.to_string())
}

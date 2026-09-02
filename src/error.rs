use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum CrmError {
    #[error("configuration already exists at {0}")]
    ConfigExists(PathBuf),
    #[error("configuration not found at {0}; run `crm config init`")]
    ConfigMissing(PathBuf),
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("person not found: {0}")]
    PersonNotFound(String),
    #[error("person reference is ambiguous: {0}")]
    AmbiguousPerson(String),
    #[error("source database is incompatible: {0}")]
    IncompatibleSource(String),
    #[error("invalid query: {0}")]
    InvalidQuery(String),
    #[error("authentication error: {0}")]
    Authentication(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("Photos face matching error: {0}")]
    PhotoFaceMatching(String),
    #[error("Photos integration error: {0}")]
    Photos(String),
    #[error("Contacts publishing error: {0}")]
    Contacts(String),
    #[error("UI error: {0}")]
    Ui(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("unsupported platform: this CRM supports macOS only")]
    UnsupportedPlatform,
}

impl CrmError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::ConfigExists(_)
            | Self::InvalidConfig(_)
            | Self::InvalidQuery(_)
            | Self::Serialization(_) => 2,
            Self::ConfigMissing(_) | Self::Io { .. } => 4,
            Self::Authentication(_) | Self::Network(_) => 4,
            Self::PhotoFaceMatching(_) | Self::Photos(_) => 5,
            Self::Contacts(_) => 5,
            Self::Ui(_) => 5,
            Self::UnsupportedPlatform => 5,
            Self::PersonNotFound(_) | Self::AmbiguousPerson(_) => 3,
            Self::IncompatibleSource(_) => 5,
            Self::Database(_) => 1,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::ConfigExists(_) => "config_exists",
            Self::ConfigMissing(_) => "config_missing",
            Self::InvalidConfig(_) => "invalid_config",
            Self::Io { .. } => "io_error",
            Self::Database(_) => "database_error",
            Self::PersonNotFound(_) => "person_not_found",
            Self::AmbiguousPerson(_) => "ambiguous_person",
            Self::IncompatibleSource(_) => "incompatible_source",
            Self::InvalidQuery(_) => "invalid_query",
            Self::Authentication(_) => "authentication_error",
            Self::Network(_) => "network_error",
            Self::PhotoFaceMatching(_) => "photo_face_matching_error",
            Self::Photos(_) => "photos_error",
            Self::Contacts(_) => "contacts_error",
            Self::Ui(_) => "ui_error",
            Self::Serialization(_) => "serialization_error",
            Self::UnsupportedPlatform => "unsupported_platform",
        }
    }
}

pub type Result<T> = std::result::Result<T, CrmError>;

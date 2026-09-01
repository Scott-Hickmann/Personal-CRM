use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::error::{CrmError, Result};

const PHOTO_PICKER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/select-photo.swift");
const PHOTO_IMPORTER: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/scripts/photos-import.applescript"
);
const IMPORT_ALBUM: &str = "CRM Imports";

pub(crate) fn select_photo(person_name: &str) -> Result<Option<PathBuf>> {
    let cache = std::env::temp_dir().join("personal-crm-swift-module-cache");
    std::fs::create_dir_all(&cache).map_err(|source| CrmError::Io {
        path: cache.clone(),
        source,
    })?;
    let result = Command::new("xcrun")
        .args(["swift", PHOTO_PICKER])
        .arg(person_name)
        .env("CLANG_MODULE_CACHE_PATH", &cache)
        .env("SWIFT_MODULECACHE_PATH", &cache)
        .output()
        .map_err(|error| CrmError::Photos(format!("could not open the photo picker: {error}")))?;
    if !result.status.success() {
        let message = String::from_utf8_lossy(&result.stderr).trim().to_owned();
        if message.contains("-128") || message.to_ascii_lowercase().contains("cancel") {
            return Ok(None);
        }
        return Err(CrmError::Photos(if message.is_empty() {
            "the photo picker failed without an error message".into()
        } else {
            message
        }));
    }
    let path = PathBuf::from(String::from_utf8_lossy(&result.stdout).trim());
    if path.is_file() {
        Ok(Some(path))
    } else {
        Err(CrmError::Photos(format!(
            "selected photo not found at {}",
            path.display()
        )))
    }
}

pub(crate) fn import_photo(path: &Path, person_id: &str) -> Result<String> {
    let result = Command::new("osascript")
        .arg(PHOTO_IMPORTER)
        .arg(path)
        .arg(IMPORT_ALBUM)
        .arg(person_id)
        .output()
        .map_err(|error| CrmError::Photos(format!("could not start Photos import: {error}")))?;
    if !result.status.success() {
        let message = String::from_utf8_lossy(&result.stderr).trim().to_owned();
        return Err(CrmError::Photos(if message.is_empty() {
            "Photos import failed without an error message".into()
        } else {
            message
        }));
    }
    let identifier = String::from_utf8_lossy(&result.stdout).trim().to_owned();
    if identifier.is_empty() {
        Err(CrmError::Photos(
            "Photos imported the image but returned no asset identifier".into(),
        ))
    } else {
        Ok(identifier)
    }
}

pub(crate) fn sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|source| CrmError::Io {
        path: path.to_owned(),
        source,
    })?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| CrmError::Io {
            path: path.to_owned(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

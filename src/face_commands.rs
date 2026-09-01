use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;

use crate::cli::{FaceCommand, FaceMatchArgs};
use crate::error::{CrmError, Result};
use crate::face_matching::{self, BoundingBox, FaceMatchResult, QueryFaceprint};
use crate::output::{self, Format};
use crate::photos_faces;

const HELPER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/face-match.swift");

#[derive(Deserialize)]
struct HelperResponse {
    faces: Vec<HelperFace>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelperFace {
    face_index: usize,
    bounding_box: BoundingBox,
    faceprint: String,
}

pub(crate) fn run(format: Format, command: FaceCommand) -> Result<()> {
    let FaceCommand::Match(args) = command;
    match_photo(format, args)
}

fn match_photo(format: Format, args: FaceMatchArgs) -> Result<()> {
    require_file(&args.photo, "query photo")?;
    let library = args.library.map_or_else(discover_library, Ok)?;
    let database_path = library.join("database/Photos.sqlite");
    require_file(&database_path, "Photos database")?;

    let stored = photos_faces::load(&database_path)?;
    let queries = query_faceprints(&args.photo)?;
    let result = face_matching::rank(&stored, queries, args.limit as usize)?;
    emit(format, &result)
}

fn query_faceprints(photo: &Path) -> Result<Vec<QueryFaceprint>> {
    let cache = std::env::temp_dir().join("personal-crm-swift-module-cache");
    fs::create_dir_all(&cache).map_err(|source| CrmError::Io {
        path: cache.clone(),
        source,
    })?;
    let response = Command::new("xcrun")
        .args(["swift", HELPER])
        .arg(photo)
        .env("CLANG_MODULE_CACHE_PATH", &cache)
        .env("SWIFT_MODULECACHE_PATH", &cache)
        .output()
        .map_err(|error| {
            CrmError::PhotoFaceMatching(format!(
                "could not start the local macOS Vision helper: {error}"
            ))
        })?;
    if !response.status.success() {
        let message = String::from_utf8_lossy(&response.stderr).trim().to_owned();
        return Err(CrmError::PhotoFaceMatching(if message.is_empty() {
            "the local macOS Vision helper failed without an error message".into()
        } else {
            message
        }));
    }
    let response: HelperResponse = serde_json::from_slice(&response.stdout).map_err(|error| {
        CrmError::PhotoFaceMatching(format!("invalid response from Vision helper: {error}"))
    })?;
    if response.faces.is_empty() {
        return Err(CrmError::PhotoFaceMatching(
            "Vision helper returned no faces".into(),
        ));
    }
    response
        .faces
        .into_iter()
        .map(|face| {
            let bytes = STANDARD.decode(face.faceprint).map_err(|error| {
                CrmError::PhotoFaceMatching(format!(
                    "invalid faceprint from Vision helper: {error}"
                ))
            })?;
            let vector = face_matching::float_vector(&bytes).ok_or_else(|| {
                CrmError::PhotoFaceMatching("invalid faceprint from Vision helper".into())
            })?;
            Ok(QueryFaceprint {
                face_index: face.face_index,
                bounding_box: face.bounding_box,
                vector,
            })
        })
        .collect()
}

fn emit(format: Format, result: &FaceMatchResult) -> Result<()> {
    let mut rows = vec!["face\tperson\tdistance\tfaceprints\tbounding box".to_owned()];
    for face in &result.faces {
        rows.extend(face.matches.iter().map(|item| {
            format!(
                "{}\t{}\t{:.4}\t{}\t{:.3},{:.3},{:.3},{:.3}",
                face.face_index,
                item.name,
                item.distance,
                item.faceprints_compared,
                face.bounding_box.x,
                face.bounding_box.y,
                face.bounding_box.width,
                face.bounding_box.height
            )
        }));
    }
    rows.push(format!(
        "\nMatched {} query faces against {} of {} faceprints across {} named people; {} invalid. Lower distance is closer.",
        result.faces.len(),
        result.reference_faceprints_usable,
        result.reference_faceprints_considered,
        result.named_people,
        result.invalid_reference_faceprints
    ));
    output::emit(format, "face.match", result, rows.join("\n"))
}

fn require_file(path: &Path, label: &str) -> Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        Err(CrmError::PhotoFaceMatching(format!(
            "{label} not found at {}",
            path.display()
        )))
    }
}

fn discover_library() -> Result<PathBuf> {
    let pictures = dirs::picture_dir().ok_or_else(|| {
        CrmError::PhotoFaceMatching("cannot determine the Pictures directory".into())
    })?;
    let conventional = pictures.join("Photos Library.photoslibrary");
    if conventional.join("database/Photos.sqlite").is_file() {
        return Ok(conventional);
    }
    let entries = fs::read_dir(&pictures).map_err(|source| CrmError::Io {
        path: pictures.clone(),
        source,
    })?;
    let mut libraries = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|value| value == "photoslibrary")
                && path.join("database/Photos.sqlite").is_file()
        })
        .collect::<Vec<_>>();
    libraries.sort();
    match libraries.as_slice() {
        [library] => Ok(library.clone()),
        [] => Err(CrmError::PhotoFaceMatching(format!(
            "no Photos library found in {}; pass --library PATH",
            pictures.display()
        ))),
        _ => Err(CrmError::PhotoFaceMatching(format!(
            "multiple Photos libraries found in {}; pass --library PATH",
            pictures.display()
        ))),
    }
}

use std::fs;
use std::path::Path;
use std::process::Command;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;

use crate::cli::{FaceCommand, FaceMatchArgs};
use crate::error::{CrmError, Result};
use crate::face_matching::{self, BoundingBox, FaceMatchResult, QueryFaceprint};
use crate::output::{self, Format};
use crate::photos_faces;
use crate::photos_library;

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
    faceprint: Option<String>,
}

pub(crate) fn run(format: Format, command: FaceCommand) -> Result<()> {
    let FaceCommand::Match(args) = command;
    match_photo(format, args)
}

fn match_photo(format: Format, args: FaceMatchArgs) -> Result<()> {
    require_file(&args.photo, "query photo")?;
    let library = photos_library::discover_library(args.library)?;
    let database_path = library.join("database/Photos.sqlite");
    require_file(&database_path, "Photos database")?;

    let stored = photos_faces::load(&database_path)?;
    let queries = query_faceprints(&args.photo, None)?;
    let result = face_matching::rank(&stored, queries, args.limit as usize)?;
    emit(format, &result)
}

pub(crate) fn query_faceprints(
    photo: &Path,
    preview: Option<&Path>,
) -> Result<Vec<QueryFaceprint>> {
    let response = run_helper(photo, preview, false)?;
    response
        .into_iter()
        .map(|face| {
            let encoded = face.faceprint.ok_or_else(|| {
                CrmError::PhotoFaceMatching("Vision helper returned no faceprint".into())
            })?;
            let bytes = STANDARD.decode(encoded).map_err(|error| {
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

pub(crate) fn detect_faces(photo: &Path, preview: &Path) -> Result<Vec<QueryFaceprint>> {
    Ok(run_helper(photo, Some(preview), true)?
        .into_iter()
        .map(|face| QueryFaceprint {
            face_index: face.face_index,
            bounding_box: face.bounding_box,
            vector: Vec::new(),
        })
        .collect())
}

fn run_helper(photo: &Path, preview: Option<&Path>, detect_only: bool) -> Result<Vec<HelperFace>> {
    let cache = std::env::temp_dir().join("personal-crm-swift-module-cache");
    fs::create_dir_all(&cache).map_err(|source| CrmError::Io {
        path: cache.clone(),
        source,
    })?;
    let mut command = Command::new("xcrun");
    command.args(["swift", HELPER]).arg(photo);
    if let Some(preview) = preview {
        command.arg(preview);
    }
    if detect_only {
        command.arg("--detect-only");
    }
    let response = command
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
    Ok(response.faces)
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

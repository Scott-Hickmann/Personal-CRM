use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::cli::{FaceCommand, FaceMatchArgs};
use crate::error::{CrmError, Result};
use crate::output::{self, Format};
use crate::photos_faces;

const HELPER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/face-match.swift");
const FACEPRINT_MARKER: &[u8] = &[0xf0, 0xca, 0xde, 0x70, 0x63, 0x61, 0x66, 0x00];

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct FaceMatchResult {
    matches: Vec<FaceMatch>,
    named_people: usize,
    faceprints_considered: usize,
    faceprints_compared: usize,
    invalid_faceprints: usize,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct FaceMatch {
    photos_person_id: String,
    name: String,
    distance: f32,
    faceprints_compared: usize,
}

#[derive(Deserialize)]
struct HelperResponse {
    faceprint: String,
}

struct Aggregate {
    name: String,
    distance: f32,
    count: usize,
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
    let query = query_faceprint(&args.photo)?;
    let mut aggregate = HashMap::<String, Aggregate>::new();
    let mut compared = 0;
    let mut invalid = 0;
    for reference in &stored.references {
        let Some(candidate) = stored_faceprint(&reference.data) else {
            invalid += 1;
            continue;
        };
        let Some(distance) = cosine_distance(&query, &candidate) else {
            invalid += 1;
            continue;
        };
        compared += 1;
        aggregate
            .entry(reference.person_id.clone())
            .and_modify(|current| {
                current.distance = current.distance.min(distance);
                current.count += 1;
            })
            .or_insert_with(|| Aggregate {
                name: reference.name.clone(),
                distance,
                count: 1,
            });
    }
    if compared == 0 {
        return Err(CrmError::PhotoFaceMatching(
            "no compatible named Photos faceprints could be compared".into(),
        ));
    }

    let mut matches = aggregate
        .into_iter()
        .map(|(photos_person_id, value)| FaceMatch {
            photos_person_id,
            name: value.name,
            distance: value.distance,
            faceprints_compared: value.count,
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        left.distance
            .total_cmp(&right.distance)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    matches.truncate(args.limit as usize);
    let result = FaceMatchResult {
        matches,
        named_people: stored.named_people,
        faceprints_considered: stored.references.len(),
        faceprints_compared: compared,
        invalid_faceprints: invalid,
    };
    emit(format, &result)
}

fn query_faceprint(photo: &Path) -> Result<Vec<f32>> {
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
    let bytes = STANDARD.decode(response.faceprint).map_err(|error| {
        CrmError::PhotoFaceMatching(format!("invalid faceprint from Vision helper: {error}"))
    })?;
    float_vector(&bytes)
        .ok_or_else(|| CrmError::PhotoFaceMatching("invalid faceprint from Vision helper".into()))
}

fn stored_faceprint(data: &[u8]) -> Option<Vec<f32>> {
    let marker = data
        .windows(FACEPRINT_MARKER.len())
        .position(|window| window == FACEPRINT_MARKER)?;
    let metadata = marker + FACEPRINT_MARKER.len();
    if data.get(metadata..metadata + 3)? != [2, 0, 0] {
        return None;
    }
    let count = u32::from_le_bytes(data.get(metadata + 3..metadata + 7)?.try_into().ok()?) as usize;
    let start = metadata + 7;
    float_vector(data.get(start..start.checked_add(count.checked_mul(4)?)?)?)
}

fn float_vector(data: &[u8]) -> Option<Vec<f32>> {
    if data.is_empty() || !data.len().is_multiple_of(4) {
        return None;
    }
    Some(
        data.chunks_exact(4)
            .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
            .collect(),
    )
}

fn cosine_distance(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.len() != right.len() {
        return None;
    }
    let mut dot = 0.0_f64;
    let mut left_norm = 0.0_f64;
    let mut right_norm = 0.0_f64;
    for (&left, &right) in left.iter().zip(right) {
        if !left.is_finite() || !right.is_finite() {
            return None;
        }
        let left = f64::from(left);
        let right = f64::from(right);
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }
    (left_norm > 0.0 && right_norm > 0.0)
        .then(|| (1.0 - dot / (left_norm * right_norm).sqrt()).clamp(0.0, 2.0) as f32)
}

fn emit(format: Format, result: &FaceMatchResult) -> Result<()> {
    let mut rows = vec!["person\tdistance\tfaceprints".to_owned()];
    rows.extend(result.matches.iter().map(|item| {
        format!(
            "{}\t{:.4}\t{}",
            item.name, item.distance, item.faceprints_compared
        )
    }));
    rows.push(format!(
        "\nCompared {} of {} faceprints across {} named people; {} invalid. Lower distance is closer.",
        result.faceprints_compared,
        result.faceprints_considered,
        result.named_people,
        result.invalid_faceprints
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stored_faceprint_and_computes_distance() {
        let vector = [1.0_f32, 0.0];
        let mut stored = FACEPRINT_MARKER.to_vec();
        stored.extend([2, 0, 0]);
        stored.extend(2_u32.to_le_bytes());
        stored.extend(vector.into_iter().flat_map(f32::to_le_bytes));
        let parsed = stored_faceprint(&stored).unwrap();
        assert_eq!(parsed, vector);
        assert_eq!(cosine_distance(&parsed, &parsed), Some(0.0));
    }
}

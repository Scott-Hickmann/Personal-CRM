use std::collections::HashMap;

use serde::Serialize;

use crate::error::{CrmError, Result};
use crate::photos_faces::{PhotoFaceprint, PhotoFaceprints};

const FACEPRINT_MARKER: &[u8] = &[0xf0, 0xca, 0xde, 0x70, 0x63, 0x61, 0x66, 0x00];

#[derive(Debug, serde::Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FaceMatchResult {
    pub faces: Vec<QueryFaceMatches>,
    pub named_people: usize,
    pub reference_faceprints_considered: usize,
    pub reference_faceprints_usable: usize,
    pub invalid_reference_faceprints: usize,
}

#[derive(Debug, serde::Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryFaceMatches {
    pub face_index: usize,
    pub bounding_box: BoundingBox,
    pub matches: Vec<FaceMatch>,
}

#[derive(Clone, Copy, Debug, serde::Deserialize, Serialize)]
pub(crate) struct BoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, serde::Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FaceMatch {
    pub photos_person_id: String,
    pub name: String,
    pub distance: f32,
    pub faceprints_compared: usize,
}

pub(crate) struct QueryFaceprint {
    pub face_index: usize,
    pub bounding_box: BoundingBox,
    pub vector: Vec<f32>,
}

struct Aggregate {
    name: String,
    distance: f32,
    count: usize,
}

pub(crate) fn rank(
    stored: &PhotoFaceprints,
    queries: Vec<QueryFaceprint>,
    limit: usize,
) -> Result<FaceMatchResult> {
    let mut invalid = 0;
    let candidates = stored
        .references
        .iter()
        .filter_map(|reference| match stored_faceprint(&reference.data) {
            Some(vector) => Some((reference, vector)),
            None => {
                invalid += 1;
                None
            }
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Err(CrmError::PhotoFaceMatching(
            "no compatible named Photos faceprints could be compared".into(),
        ));
    }
    let faces = queries
        .into_iter()
        .map(|query| {
            let matches = match_face(&query.vector, &candidates, limit);
            if matches.is_empty() {
                return Err(CrmError::PhotoFaceMatching(format!(
                    "face {} has an incompatible faceprint",
                    query.face_index
                )));
            }
            Ok(QueryFaceMatches {
                face_index: query.face_index,
                bounding_box: query.bounding_box,
                matches,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(FaceMatchResult {
        faces,
        named_people: stored.named_people,
        reference_faceprints_considered: stored.references.len(),
        reference_faceprints_usable: candidates.len(),
        invalid_reference_faceprints: invalid,
    })
}

fn match_face(
    query: &[f32],
    candidates: &[(&PhotoFaceprint, Vec<f32>)],
    limit: usize,
) -> Vec<FaceMatch> {
    let mut aggregate = HashMap::<String, Aggregate>::new();
    for (reference, candidate) in candidates {
        let Some(distance) = cosine_distance(query, candidate) else {
            continue;
        };
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
    matches.truncate(limit);
    matches
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

pub(crate) fn float_vector(data: &[u8]) -> Option<Vec<f32>> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(vector: &[f32]) -> Vec<u8> {
        let mut stored = FACEPRINT_MARKER.to_vec();
        stored.extend([2, 0, 0]);
        stored.extend(u32::try_from(vector.len()).unwrap().to_le_bytes());
        stored.extend(vector.iter().flat_map(|value| value.to_le_bytes()));
        stored
    }

    #[test]
    fn parses_stored_faceprint_and_computes_distance() {
        let vector = [1.0_f32, 0.0];
        let stored = encoded(&vector);
        let parsed = stored_faceprint(&stored).unwrap();
        assert_eq!(parsed, vector);
        assert_eq!(cosine_distance(&parsed, &parsed), Some(0.0));
    }

    #[test]
    fn ranks_each_query_face_independently() {
        let stored = PhotoFaceprints {
            named_people: 2,
            references: vec![
                PhotoFaceprint {
                    person_id: "left".into(),
                    name: "Left".into(),
                    data: encoded(&[1.0, 0.0]),
                },
                PhotoFaceprint {
                    person_id: "right".into(),
                    name: "Right".into(),
                    data: encoded(&[0.0, 1.0]),
                },
            ],
        };
        let queries = vec![
            QueryFaceprint {
                face_index: 1,
                bounding_box: BoundingBox {
                    x: 0.1,
                    y: 0.2,
                    width: 0.3,
                    height: 0.4,
                },
                vector: vec![1.0, 0.0],
            },
            QueryFaceprint {
                face_index: 2,
                bounding_box: BoundingBox {
                    x: 0.6,
                    y: 0.2,
                    width: 0.3,
                    height: 0.4,
                },
                vector: vec![0.0, 1.0],
            },
        ];

        let result = rank(&stored, queries, 1).unwrap();
        assert_eq!(result.faces.len(), 2);
        assert_eq!(result.faces[0].matches[0].photos_person_id, "left");
        assert_eq!(result.faces[1].matches[0].photos_person_id, "right");
    }
}

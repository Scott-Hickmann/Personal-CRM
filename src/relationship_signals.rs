use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::Result;

const HALF_LIFE_DAYS: f64 = 365.0;

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct RelationalComponents {
    pub assessed_interactions: i64,
    pub meaningful_interactions: i64,
    pub intimacy: f64,
    pub emotional_support: f64,
    pub practical_support: f64,
    pub affection: f64,
    pub shared_activity: f64,
    pub conflict_repair: f64,
    pub evidence: Vec<RelationalEvidence>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RelationalEvidence {
    pub occurred_at: String,
    pub summary: String,
}

pub fn aggregate(connection: &Connection, person_id: &str) -> Result<(f64, RelationalComponents)> {
    let mut statement = connection.prepare(
        "SELECT rs.intimacy, rs.emotional_support, rs.practical_support, rs.affection,
                rs.shared_activity, rs.conflict_repair, rs.confidence, rs.evidence,
                i.occurred_at, MAX(0.0, julianday('now') - julianday(i.occurred_at))
         FROM relationship_signals rs JOIN interactions i ON i.id=rs.interaction_id
         WHERE rs.person_id=?1 AND i.deleted_at IS NULL
         ORDER BY i.occurred_at DESC",
    )?;
    let rows = statement
        .query_map([person_id], |row| {
            Ok(Signal {
                dimensions: [
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ],
                confidence: row.get(6)?,
                evidence: row.get(7)?,
                occurred_at: row.get(8)?,
                age_days: row.get(9)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(score(&rows))
}

struct Signal {
    dimensions: [f64; 6],
    confidence: f64,
    evidence: String,
    occurred_at: String,
    age_days: f64,
}

fn score(rows: &[Signal]) -> (f64, RelationalComponents) {
    let mut components = RelationalComponents {
        assessed_interactions: rows.len() as i64,
        ..RelationalComponents::default()
    };
    let mut dimension_totals = [0.0; 6];
    let mut meaningful_weight = 0.0;
    let mut quality_total = 0.0;
    for row in rows {
        let normalized = row.dimensions.map(|value| (value / 3.0).clamp(0.0, 1.0));
        let maximum = normalized.iter().copied().fold(0.0, f64::max);
        if maximum == 0.0 {
            continue;
        }
        components.meaningful_interactions += 1;
        let weight =
            row.confidence.clamp(0.0, 1.0) * 0.5_f64.powf(row.age_days.max(0.0) / HALF_LIFE_DAYS);
        let mean = normalized.iter().sum::<f64>() / normalized.len() as f64;
        quality_total += (maximum * 0.6 + mean * 0.4) * weight;
        meaningful_weight += weight;
        for (total, value) in dimension_totals.iter_mut().zip(normalized) {
            *total += value * weight;
        }
        if !row.evidence.trim().is_empty() && components.evidence.len() < 3 {
            components.evidence.push(RelationalEvidence {
                occurred_at: row.occurred_at.clone(),
                summary: row.evidence.clone(),
            });
        }
    }
    if meaningful_weight == 0.0 {
        return (0.0, components);
    }
    let dimension_scores = dimension_totals.map(|total| total / meaningful_weight * 100.0);
    components.intimacy = dimension_scores[0];
    components.emotional_support = dimension_scores[1];
    components.practical_support = dimension_scores[2];
    components.affection = dimension_scores[3];
    components.shared_activity = dimension_scores[4];
    components.conflict_repair = dimension_scores[5];

    let quality = quality_total / meaningful_weight;
    let evidence_depth = 1.0 - (-meaningful_weight / 4.0).exp();
    let breadth = dimension_totals
        .iter()
        .filter(|total| **total >= 0.5)
        .count() as f64
        / dimension_totals.len() as f64;
    let relational_score =
        (quality * 50.0 + evidence_depth * 30.0 + breadth * 20.0).clamp(0.0, 100.0);
    (relational_score, components)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(dimensions: [f64; 6]) -> Signal {
        Signal {
            dimensions,
            confidence: 1.0,
            evidence: "Supported by a message".into(),
            occurred_at: "2026-09-01".into(),
            age_days: 0.0,
        }
    }

    #[test]
    fn confidence_weights_evidence_without_becoming_relationship_strength() {
        let low_confidence = Signal {
            confidence: 0.1,
            ..signal([3.0, 0.0, 0.0, 0.0, 0.0, 0.0])
        };
        let (score, components) = score(&[low_confidence]);

        assert!(score < 50.0);
        assert_eq!(components.intimacy, 100.0);
    }

    #[test]
    fn repeated_broad_evidence_scores_above_one_narrow_signal() {
        let (narrow, _) = score(&[signal([3.0, 0.0, 0.0, 0.0, 0.0, 0.0])]);
        let broad_rows: Vec<_> = (0..6).map(|_| signal([3.0; 6])).collect();
        let (broad, components) = score(&broad_rows);

        assert!(broad > narrow);
        assert_eq!(components.meaningful_interactions, 6);
        assert_eq!(components.evidence.len(), 3);
    }
}

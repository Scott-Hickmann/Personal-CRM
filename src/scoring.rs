use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::affinity_calibration::{Calibration, CalibrationPoint, target_score};
use crate::error::Result;
use crate::progress::ProgressTracker;
use crate::relationship_signals::{self, RelationalComponents};

#[derive(Debug, Serialize)]
pub struct ScoreExplanation {
    pub person_id: String,
    pub display_name: String,
    pub affinity_score: f64,
    pub affinity_tier: String,
    pub activity_state: String,
    pub behavioral_score: f64,
    pub relational_score: f64,
    pub closeness_rating: Option<i64>,
    pub calibration: Calibration,
    pub components: Components,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Components {
    pub interactions_30d: i64,
    pub interactions_90d: i64,
    pub interactions_365d: i64,
    pub active_weeks_90d: i64,
    pub channels_90d: i64,
    pub incoming_90d: i64,
    pub outgoing_90d: i64,
    pub days_since_last: Option<f64>,
    pub relationship_span_days: f64,
    pub base_score: f64,
    pub relational: RelationalComponents,
}

struct Candidate {
    person_id: String,
    behavioral_score: f64,
    relational_score: f64,
    components: Components,
}

pub fn recalculate_all(connection: &Connection, progress: &mut ProgressTracker) -> Result<usize> {
    let mut statement = connection.prepare(
        "SELECT id, display_name FROM people p WHERE p.lifecycle_state='active' AND NOT EXISTS
         (SELECT 1 FROM identities i WHERE i.person_id=p.id AND i.is_self=1 AND i.active=1)",
    )?;
    let people: Vec<(String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    let total = people.len() as u64;
    progress.stage(
        "Recalculating relationship scores",
        1,
        1,
        total,
        false,
        "people",
    );
    let mut candidates = Vec::with_capacity(people.len());
    for (id, name) in &people {
        progress.focus([name.clone()]);
        candidates.push(calculate_candidate(connection, id)?);
    }
    let ratings = ratings(connection)?;
    let calibration = fit_calibration(&candidates, &ratings);
    for (index, candidate) in candidates.iter().enumerate() {
        progress.focus([people[index].1.clone()]);
        persist(
            connection,
            candidate,
            &calibration,
            ratings.get(&candidate.person_id).copied(),
        )?;
        progress.progress(
            "Recalculating relationship scores",
            (index + 1) as u64,
            total,
            false,
            "people",
        );
    }
    progress.finish_stage(
        "Recalculated relationship scores",
        total,
        total,
        false,
        "people",
    );
    Ok(people.len())
}

pub fn explain(connection: &Connection, person_id: &str) -> Result<ScoreExplanation> {
    let calibration = current_calibration(connection)?;
    let stored = connection
        .query_row(
            "SELECT p.id, p.display_name, p.affinity_score, p.affinity_tier, p.activity_state,
                    m.behavioral_score, m.relational_score, m.components_json, ar.rating
             FROM people p JOIN metrics m ON m.person_id=p.id
             LEFT JOIN affinity_ratings ar ON ar.person_id=p.id WHERE p.id=?1",
            [person_id],
            |row| {
                let components: String = row.get(7)?;
                Ok(ScoreExplanation {
                    person_id: row.get(0)?,
                    display_name: row.get(1)?,
                    affinity_score: row.get(2)?,
                    affinity_tier: row.get(3)?,
                    activity_state: row.get(4)?,
                    behavioral_score: row.get(5)?,
                    relational_score: row.get(6)?,
                    closeness_rating: row.get(8)?,
                    calibration: calibration.clone(),
                    components: serde_json::from_str(&components).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            7,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                })
            },
        )
        .optional()?;
    if let Some(explanation) = stored {
        return Ok(explanation);
    }

    let candidate = calculate_candidate(connection, person_id)?;
    let (display_name, rating): (String, Option<i64>) = connection.query_row(
        "SELECT p.display_name, ar.rating FROM people p
         LEFT JOIN affinity_ratings ar ON ar.person_id=p.id WHERE p.id=?1",
        [person_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let affinity_score = rating
        .map(target_score)
        .unwrap_or_else(|| calibration.apply(candidate.components.base_score));
    Ok(ScoreExplanation {
        person_id: candidate.person_id,
        display_name,
        affinity_score,
        affinity_tier: tier(affinity_score).into(),
        activity_state: activity(candidate.components.days_since_last).into(),
        behavioral_score: candidate.behavioral_score,
        relational_score: candidate.relational_score,
        closeness_rating: rating,
        calibration,
        components: candidate.components,
    })
}

fn calculate_candidate(connection: &Connection, person_id: &str) -> Result<Candidate> {
    let mut components = connection.query_row(
        "SELECT COUNT(DISTINCT CASE WHEN i.occurred_at >= datetime('now', '-30 days') THEN i.id END),
                COUNT(DISTINCT CASE WHEN i.occurred_at >= datetime('now', '-90 days') THEN i.id END),
                COUNT(DISTINCT CASE WHEN i.occurred_at >= datetime('now', '-365 days') THEN i.id END),
                COUNT(DISTINCT CASE WHEN i.occurred_at >= datetime('now', '-90 days')
                    THEN strftime('%Y-%W', i.occurred_at) END),
                COUNT(DISTINCT CASE WHEN i.occurred_at >= datetime('now', '-90 days') THEN i.channel END),
                COUNT(DISTINCT CASE WHEN i.occurred_at >= datetime('now', '-90 days')
                    AND i.direction='incoming' THEN i.id END),
                COUNT(DISTINCT CASE WHEN i.occurred_at >= datetime('now', '-90 days')
                    AND i.direction='outgoing' THEN i.id END),
                julianday('now') - julianday(MAX(i.occurred_at)),
                COALESCE(julianday(MAX(i.occurred_at)) - julianday(MIN(i.occurred_at)), 0.0)
         FROM interactions i JOIN interaction_participants ip ON ip.interaction_id=i.id
         WHERE ip.person_id=?1 AND i.deleted_at IS NULL",
        [person_id],
        |row| {
            Ok(Components {
                interactions_30d: row.get(0)?,
                interactions_90d: row.get(1)?,
                interactions_365d: row.get(2)?,
                active_weeks_90d: row.get(3)?,
                channels_90d: row.get(4)?,
                incoming_90d: row.get(5)?,
                outgoing_90d: row.get(6)?,
                days_since_last: row.get(7)?,
                relationship_span_days: row.get(8)?,
                base_score: 0.0,
                relational: RelationalComponents::default(),
            })
        },
    )?;
    let behavioral_score = behavioral_score(&components);
    let (relational_score, relational) = relationship_signals::aggregate(connection, person_id)?;
    components.relational = relational;
    components.base_score = base_affinity(
        behavioral_score,
        relational_score,
        components.relational.assessed_interactions,
    );
    Ok(Candidate {
        person_id: person_id.into(),
        behavioral_score,
        relational_score,
        components,
    })
}

fn behavioral_score(components: &Components) -> f64 {
    let recent_volume = logarithmic(components.interactions_90d as f64, 30.0) * 25.0;
    let annual_volume = logarithmic(components.interactions_365d as f64, 120.0) * 10.0;
    let consistency = saturate(components.active_weeks_90d as f64, 13.0) * 20.0;
    let channels = saturate(components.channels_90d as f64, 3.0) * 5.0;
    let balance = match (components.incoming_90d, components.outgoing_90d) {
        (0, 0) => 0.0,
        (incoming, outgoing) => {
            incoming.min(outgoing) as f64 / incoming.max(outgoing) as f64 * 15.0
        }
    };
    let recency = components
        .days_since_last
        .map(|days| 0.5_f64.powf(days.max(0.0) / 60.0) * 15.0)
        .unwrap_or(0.0);
    let duration = saturate(components.relationship_span_days, 730.0) * 10.0;
    (recent_volume + annual_volume + consistency + channels + balance + recency + duration)
        .clamp(0.0, 100.0)
}

fn base_affinity(behavioral: f64, relational: f64, assessed_interactions: i64) -> f64 {
    let coverage = 1.0 - (-(assessed_interactions as f64) / 10.0).exp();
    let relational_weight = 0.35 * coverage;
    behavioral * (1.0 - relational_weight) + relational * relational_weight
}

fn fit_calibration(candidates: &[Candidate], ratings: &HashMap<String, i64>) -> Calibration {
    let points: Vec<_> = candidates
        .iter()
        .filter_map(|candidate| {
            ratings
                .get(&candidate.person_id)
                .map(|rating| CalibrationPoint {
                    base_score: candidate.components.base_score,
                    rating: *rating,
                })
        })
        .collect();
    Calibration::fit(&points)
}

fn current_calibration(connection: &Connection) -> Result<Calibration> {
    let mut statement = connection.prepare(
        "SELECT m.components_json, ar.rating
         FROM metrics m JOIN affinity_ratings ar ON ar.person_id=m.person_id",
    )?;
    let points = statement
        .query_map([], |row| {
            let json: String = row.get(0)?;
            let components: Components = serde_json::from_str(&json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            Ok(CalibrationPoint {
                base_score: components.base_score,
                rating: row.get(1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(Calibration::fit(&points))
}

fn ratings(connection: &Connection) -> Result<HashMap<String, i64>> {
    let mut statement = connection.prepare("SELECT person_id, rating FROM affinity_ratings")?;
    Ok(statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?)
}

fn persist(
    connection: &Connection,
    candidate: &Candidate,
    calibration: &Calibration,
    rating: Option<i64>,
) -> Result<()> {
    let score = rating
        .map(target_score)
        .unwrap_or_else(|| calibration.apply(candidate.components.base_score));
    let tier = tier(score);
    let activity = activity(candidate.components.days_since_last);
    let components_json = serde_json::to_string(&candidate.components)
        .map_err(|error| crate::error::CrmError::Serialization(error.to_string()))?;
    connection.execute(
        "INSERT INTO metrics(person_id, behavioral_score, relational_score, components_json, model_version)
         VALUES (?1, ?2, ?3, ?4, 'hybrid-v1')
         ON CONFLICT(person_id) DO UPDATE SET behavioral_score=excluded.behavioral_score,
         relational_score=excluded.relational_score, components_json=excluded.components_json,
         model_version=excluded.model_version, calculated_at=CURRENT_TIMESTAMP",
        params![
            candidate.person_id,
            candidate.behavioral_score,
            candidate.relational_score,
            components_json
        ],
    )?;
    connection.execute(
        "UPDATE people SET affinity_score=?2, affinity_tier=?3, activity_state=?4,
         updated_at=CURRENT_TIMESTAMP WHERE id=?1",
        params![candidate.person_id, score, tier, activity],
    )?;
    Ok(())
}

fn logarithmic(value: f64, reference: f64) -> f64 {
    (value.max(0.0).ln_1p() / reference.ln_1p()).clamp(0.0, 1.0)
}

fn saturate(value: f64, maximum: f64) -> f64 {
    (value / maximum).clamp(0.0, 1.0)
}

fn tier(score: f64) -> &'static str {
    if score >= 80.0 {
        "core"
    } else if score >= 60.0 {
        "close"
    } else if score >= 40.0 {
        "familiar"
    } else if score >= 20.0 {
        "acquaintance"
    } else {
        "peripheral"
    }
}

fn activity(days: Option<f64>) -> &'static str {
    match days {
        Some(days) if days <= 30.0 => "active",
        Some(days) if days <= 90.0 => "cooling",
        Some(_) => "dormant",
        None => "never",
    }
}

#[cfg(test)]
mod tests;

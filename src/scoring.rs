use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::error::Result;

#[derive(Debug, Serialize)]
pub struct ScoreExplanation {
    pub person_id: String,
    pub display_name: String,
    pub affinity_score: f64,
    pub affinity_tier: String,
    pub activity_state: String,
    pub behavioral_score: f64,
    pub semantic_score: f64,
    pub components: Components,
}

#[derive(Debug, Serialize, serde::Deserialize)]
pub struct Components {
    pub interactions_90d: i64,
    pub active_days_90d: i64,
    pub channels_90d: i64,
    pub incoming_90d: i64,
    pub outgoing_90d: i64,
    pub days_since_last: Option<f64>,
}

pub fn recalculate_all(connection: &Connection) -> Result<usize> {
    let mut statement = connection.prepare(
        "SELECT id FROM people p WHERE NOT EXISTS
         (SELECT 1 FROM identities i WHERE i.person_id=p.id AND i.is_self=1)",
    )?;
    let ids: Vec<String> = statement
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    let transaction = connection.unchecked_transaction()?;
    for id in &ids {
        calculate_one(&transaction, id)?;
    }
    transaction.commit()?;
    Ok(ids.len())
}

pub fn explain(connection: &Connection, person_id: &str) -> Result<ScoreExplanation> {
    if connection
        .query_row(
            "SELECT 1 FROM metrics WHERE person_id=?1",
            [person_id],
            |_| Ok(()),
        )
        .optional()?
        .is_none()
    {
        calculate_one(connection, person_id)?;
    }
    connection
        .query_row(
            "SELECT p.id, p.display_name, p.affinity_score, p.affinity_tier, p.activity_state,
         m.behavioral_score, m.semantic_score, m.components_json
         FROM people p JOIN metrics m ON m.person_id=p.id WHERE p.id=?1",
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
                    semantic_score: row.get(6)?,
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
        .map_err(Into::into)
}

fn calculate_one(connection: &Connection, person_id: &str) -> Result<()> {
    let components = connection.query_row(
        "SELECT COUNT(DISTINCT CASE WHEN i.occurred_at >= datetime('now', '-90 days') THEN i.id END),
         COUNT(DISTINCT CASE WHEN i.occurred_at >= datetime('now', '-90 days') THEN date(i.occurred_at) END),
         COUNT(DISTINCT CASE WHEN i.occurred_at >= datetime('now', '-90 days') THEN i.channel END),
         COUNT(DISTINCT CASE WHEN i.occurred_at >= datetime('now', '-90 days') AND i.direction='incoming' THEN i.id END),
         COUNT(DISTINCT CASE WHEN i.occurred_at >= datetime('now', '-90 days') AND i.direction='outgoing' THEN i.id END),
         julianday('now') - julianday(MAX(i.occurred_at))
         FROM interactions i JOIN interaction_participants ip ON ip.interaction_id=i.id
         WHERE ip.person_id=?1 AND i.deleted_at IS NULL",
        [person_id],
        |row| {
            Ok(Components {
                interactions_90d: row.get(0)?,
                active_days_90d: row.get(1)?,
                channels_90d: row.get(2)?,
                incoming_90d: row.get(3)?,
                outgoing_90d: row.get(4)?,
                days_since_last: row.get(5)?,
            })
        },
    )?;
    let volume = saturate(components.interactions_90d as f64, 30.0) * 40.0;
    let consistency = saturate(components.active_days_90d as f64, 12.0) * 25.0;
    let channels = saturate(components.channels_90d as f64, 3.0) * 10.0;
    let balance = if components.incoming_90d + components.outgoing_90d == 0 {
        0.0
    } else {
        let smaller = components.incoming_90d.min(components.outgoing_90d) as f64;
        let larger = components.incoming_90d.max(components.outgoing_90d) as f64;
        smaller / larger * 15.0
    };
    let recency = components
        .days_since_last
        .map(|days| (1.0 - (days / 90.0)).clamp(0.0, 1.0) * 10.0)
        .unwrap_or(0.0);
    let behavioral = volume + consistency + channels + balance + recency;
    let semantic: f64 = connection.query_row(
        "SELECT COALESCE(AVG(confidence) * 100.0, 0.0) FROM relationships
         WHERE source_person_id=?1 OR target_person_id=?1",
        [person_id],
        |row| row.get(0),
    )?;
    let score = behavioral * 0.7 + semantic * 0.3;
    let tier = tier(score);
    let activity = activity(components.days_since_last);
    let components_json = serde_json::to_string(&components)
        .map_err(|error| crate::error::CrmError::Serialization(error.to_string()))?;
    connection.execute(
        "INSERT INTO metrics(person_id, behavioral_score, semantic_score, components_json, model_version)
         VALUES (?1, ?2, ?3, ?4, 'behavior-v1')
         ON CONFLICT(person_id) DO UPDATE SET behavioral_score=excluded.behavioral_score,
         semantic_score=excluded.semantic_score, components_json=excluded.components_json,
         model_version=excluded.model_version, calculated_at=CURRENT_TIMESTAMP",
        params![person_id, behavioral, semantic, components_json],
    )?;
    connection.execute(
        "UPDATE people SET affinity_score=?2, affinity_tier=?3, activity_state=?4,
         updated_at=CURRENT_TIMESTAMP WHERE id=?1",
        params![person_id, score, tier, activity],
    )?;
    Ok(())
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
mod tests {
    use super::*;

    #[test]
    fn tiers_and_activity_have_explicit_boundaries() {
        assert_eq!(tier(80.0), "core");
        assert_eq!(tier(59.9), "familiar");
        assert_eq!(tier(0.0), "peripheral");
        assert_eq!(activity(Some(91.0)), "dormant");
        assert_eq!(activity(None), "never");
    }
}

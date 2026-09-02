mod overview;
mod person;

use rusqlite::Connection;
use serde::Serialize;

use crate::error::Result;
use crate::graph::ConnectionGraph;
use crate::repository::Person;
use crate::scoring::ScoreExplanation;

pub use overview::load as overview;
pub use person::{load as person, load_interaction as interaction};

#[derive(Debug, Serialize)]
pub struct Overview {
    pub people: Vec<OverviewPerson>,
    pub graph: ConnectionGraph,
}

#[derive(Debug, Serialize)]
pub struct OverviewPerson {
    pub id: String,
    pub display_name: String,
    pub lifecycle_state: String,
    pub affinity_score: Option<f64>,
    pub affinity_tier: Option<String>,
    pub activity_state: Option<String>,
    pub interaction_count: i64,
    pub last_interaction_at: Option<String>,
    pub is_self: bool,
    pub tags: Vec<String>,
    pub identities: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PersonDetail {
    pub person: Person,
    pub score: ScoreExplanation,
    pub interactions: Vec<InteractionPreview>,
    pub relationships: Vec<Relationship>,
    pub important_dates: Vec<ImportantDate>,
    pub followups: Vec<Followup>,
    pub cadence: Option<Cadence>,
    pub summaries: Vec<SemanticSummary>,
    pub photo: Option<PhotoLink>,
}

#[derive(Debug, Serialize)]
pub struct InteractionPreview {
    pub id: String,
    pub channel: String,
    pub kind: String,
    pub occurred_at: String,
    pub direction: Option<String>,
    pub subject: Option<String>,
    pub preview: Option<String>,
    pub has_body: bool,
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Serialize)]
pub struct InteractionBody {
    pub id: String,
    pub subject: Option<String>,
    pub body: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Attachment {
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size_bytes: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct Relationship {
    pub id: String,
    pub person_id: String,
    pub display_name: String,
    pub relationship_type: String,
    pub confidence: f64,
    pub status: String,
    pub evidence: serde_json::Value,
    pub first_observed_at: Option<String>,
    pub last_observed_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImportantDate {
    pub id: String,
    pub label: String,
    pub date: String,
    pub recurring: bool,
}

#[derive(Debug, Serialize)]
pub struct Followup {
    pub id: String,
    pub body: String,
    pub due_at: Option<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct Cadence {
    pub interval_days: i64,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct SemanticSummary {
    pub id: String,
    pub summary: String,
    pub model_version: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct PhotoLink {
    pub photos_name: Option<String>,
    pub photos_asset_id: Option<String>,
    pub state: String,
    pub reviewed_at: Option<String>,
    pub updated_at: String,
}

fn separated(value: String) -> Vec<String> {
    value
        .split('\u{1f}')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn json(value: String) -> serde_json::Value {
    serde_json::from_str(&value).unwrap_or(serde_json::Value::Null)
}

fn attachments(connection: &Connection, interaction_id: &str) -> Result<Vec<Attachment>> {
    let mut statement = connection.prepare(
        "SELECT filename, mime_type, size_bytes FROM attachments WHERE interaction_id=?1",
    )?;
    Ok(statement
        .query_map([interaction_id], |row| {
            Ok(Attachment {
                filename: row.get(0)?,
                mime_type: row.get(1)?,
                size_bytes: row.get(2)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?)
}

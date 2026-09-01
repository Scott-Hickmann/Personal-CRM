use std::path::{Path, PathBuf};

use crate::cli::{GraphArgs, PersonReference};
use crate::config::Config;
use crate::db;
use crate::error::{CrmError, Result};
use crate::graph;
use crate::output::{self, Format};
use crate::{repository, scoring};

pub fn explain(format: Format, config_path: PathBuf, args: PersonReference) -> Result<()> {
    let (_, connection) = open(&config_path)?;
    let person_id = repository::resolve_person_id(&connection, &args.person)?;
    let explanation = scoring::explain(&connection, &person_id)?;
    let table = format!(
        "{}\naffinity   {:.1} ({})\nactivity   {}\nbehavior   {:.1}\nsemantic   {:.1}\n90d count  {}\nlast seen  {}",
        explanation.display_name,
        explanation.affinity_score,
        explanation.affinity_tier,
        explanation.activity_state,
        explanation.behavioral_score,
        explanation.semantic_score,
        explanation.components.interactions_90d,
        explanation
            .components
            .days_since_last
            .map(|days| format!("{days:.0} days"))
            .unwrap_or_else(|| "never".into())
    );
    output::emit(format, "explain", &explanation, table)
}

pub fn graph(format: Format, config_path: PathBuf, args: GraphArgs) -> Result<()> {
    let (_, connection) = open(&config_path)?;
    let person_id = args
        .person
        .as_deref()
        .map(|person| repository::resolve_person_id(&connection, person))
        .transpose()?;
    let graph = graph::build(&connection, person_id.as_deref(), args.min_confidence)?;
    let table = graph.mermaid.clone();
    output::emit(format, "graph", &graph, table)
}

fn open(config_path: &Path) -> Result<(Config, rusqlite::Connection)> {
    if !config_path.exists() {
        return Err(CrmError::ConfigMissing(config_path.to_path_buf()));
    }
    let config = Config::load(config_path)?;
    let database = config_path
        .parent()
        .ok_or_else(|| CrmError::InvalidConfig("configuration path has no parent".into()))?
        .join("crm.sqlite3");
    Ok((config, db::open(&database)?))
}

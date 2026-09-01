use std::collections::BTreeMap;

use rusqlite::{Connection, params};
use serde::Serialize;

use crate::error::{CrmError, Result};

#[derive(Debug, Serialize)]
pub struct ConnectionGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub mermaid: String,
}

#[derive(Debug, Serialize)]
pub struct GraphNode {
    pub id: String,
    pub person_id: String,
    pub label: String,
}

#[derive(Debug, Serialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub relationship_type: String,
    pub confidence: f64,
}

struct RelationshipRow {
    source_id: String,
    source_name: String,
    target_id: String,
    target_name: String,
    relationship_type: String,
    confidence: f64,
}

pub fn build(
    connection: &Connection,
    person_id: Option<&str>,
    min_confidence: f64,
) -> Result<ConnectionGraph> {
    if !(0.0..=1.0).contains(&min_confidence) {
        return Err(CrmError::InvalidConfig(
            "minimum confidence must be between 0 and 1".into(),
        ));
    }
    let base = "SELECT r.source_person_id, source.display_name, r.target_person_id,
                target.display_name, r.relationship_type, r.confidence
                FROM relationships r
                JOIN people source ON source.id=r.source_person_id AND source.lifecycle_state='active'
                JOIN people target ON target.id=r.target_person_id AND target.lifecycle_state='active'";
    let rows = if let Some(person_id) = person_id {
        collect(
            connection,
            &format!(
                "{base} WHERE r.confidence >= ?1 AND (r.source_person_id=?2 OR r.target_person_id=?2)
                 ORDER BY r.confidence DESC"
            ),
            params![min_confidence, person_id],
        )?
    } else {
        collect(
            connection,
            &format!("{base} WHERE r.confidence >= ?1 ORDER BY r.confidence DESC"),
            [min_confidence],
        )?
    };
    assemble(rows)
}

fn collect<P: rusqlite::Params>(
    connection: &Connection,
    sql: &str,
    parameters: P,
) -> Result<Vec<RelationshipRow>> {
    let mut statement = connection.prepare(sql)?;
    Ok(statement
        .query_map(parameters, |row| {
            Ok(RelationshipRow {
                source_id: row.get(0)?,
                source_name: row.get(1)?,
                target_id: row.get(2)?,
                target_name: row.get(3)?,
                relationship_type: row.get(4)?,
                confidence: row.get(5)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?)
}

fn assemble(rows: Vec<RelationshipRow>) -> Result<ConnectionGraph> {
    let mut people = BTreeMap::new();
    for row in &rows {
        people.insert(row.source_id.clone(), row.source_name.clone());
        people.insert(row.target_id.clone(), row.target_name.clone());
    }
    let identifiers: BTreeMap<_, _> = people
        .keys()
        .enumerate()
        .map(|(index, id)| (id.clone(), format!("n{index}")))
        .collect();
    let nodes = people
        .into_iter()
        .map(|(person_id, label)| GraphNode {
            id: identifiers[&person_id].clone(),
            person_id,
            label,
        })
        .collect::<Vec<_>>();
    let edges = rows
        .into_iter()
        .map(|row| GraphEdge {
            source: identifiers[&row.source_id].clone(),
            target: identifiers[&row.target_id].clone(),
            relationship_type: row.relationship_type,
            confidence: row.confidence,
        })
        .collect::<Vec<_>>();
    let mut lines = vec!["graph LR".to_owned()];
    for node in &nodes {
        lines.push(format!("    {}[\"{}\"]", node.id, escape(&node.label)));
    }
    for edge in &edges {
        lines.push(format!(
            "    {} -- \"{} {:.0}%\" --> {}",
            edge.source,
            escape(&edge.relationship_type),
            edge.confidence * 100.0,
            edge.target
        ));
    }
    Ok(ConnectionGraph {
        nodes,
        edges,
        mermaid: lines.join("\n"),
    })
}

fn escape(value: &str) -> String {
    value.replace('&', "&amp;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_mermaid_labels() {
        assert_eq!(escape("A & \"B\""), "A &amp; &quot;B&quot;");
    }
}

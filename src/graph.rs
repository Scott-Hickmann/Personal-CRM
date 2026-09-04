use std::collections::BTreeMap;

use rusqlite::{Connection, params};
use serde::Serialize;

use crate::error::Result;

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
    pub shared_context_count: i64,
}

struct RelationshipRow {
    source_id: String,
    source_name: String,
    target_id: String,
    target_name: String,
    shared_context_count: i64,
}

pub fn build(connection: &Connection, person_id: Option<&str>) -> Result<ConnectionGraph> {
    let base = "SELECT r.source_person_id, source.display_name, r.target_person_id,
                target.display_name, r.shared_context_count
                FROM relationships r
                JOIN people source ON source.id=r.source_person_id AND source.lifecycle_state='active'
                JOIN people target ON target.id=r.target_person_id AND target.lifecycle_state='active'";
    let rows = if let Some(person_id) = person_id {
        collect(
            connection,
            &format!(
                "{base} WHERE r.source_person_id=?1 OR r.target_person_id=?1
                 ORDER BY r.shared_context_count DESC"
            ),
            params![person_id],
        )?
    } else {
        collect(
            connection,
            &format!("{base} ORDER BY r.shared_context_count DESC"),
            [],
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
                shared_context_count: row.get(4)?,
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
            shared_context_count: row.shared_context_count,
        })
        .collect::<Vec<_>>();
    let mut lines = vec!["graph LR".to_owned()];
    for node in &nodes {
        lines.push(format!("    {}[\"{}\"]", node.id, escape(&node.label)));
    }
    for edge in &edges {
        lines.push(format!(
            "    {} -- \"{} shared\" --- {}",
            edge.source, edge.shared_context_count, edge.target
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

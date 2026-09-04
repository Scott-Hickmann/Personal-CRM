use crate::error::Result;
use rusqlite::Connection;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Serialize)]
pub struct Input {
    pub people: Vec<(String, String)>,
    pub edges: Vec<(usize, usize, f64, f64)>,
    pub evidence: Vec<Evidence>,
}

#[derive(Clone, Serialize)]
pub struct Evidence {
    pub kind: String,
    pub label: String,
    pub source: String,
    pub members: BTreeSet<String>,
}

pub fn load(connection: &Connection) -> Result<Input> {
    let mut statement = connection.prepare(
        "SELECT r.source_person_id, a.display_name, r.target_person_id, b.display_name,
                r.shared_context_count
         FROM relationships r JOIN people a ON a.id=r.source_person_id
         JOIN people b ON b.id=r.target_person_id
         WHERE a.lifecycle_state='active' AND b.lifecycle_state='active'
         AND NOT EXISTS(SELECT 1 FROM identities i WHERE i.is_self=1
                        AND i.person_id IN (a.id,b.id))
         ORDER BY r.source_person_id, r.target_person_id",
    )?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, f64>(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut people = BTreeMap::new();
    for (a, an, b, bn, _) in &rows {
        people.insert(a.clone(), an.clone());
        people.insert(b.clone(), bn.clone());
    }
    let people: Vec<_> = people.into_iter().collect();
    let indices: BTreeMap<_, _> = people
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (id.clone(), i))
        .collect();
    let mut statement = connection.prepare(
        "SELECT source_person_id, target_person_id, source_id, thread_native_id
         FROM relationship_contexts ORDER BY source_id, thread_native_id, source_person_id, target_person_id")?;
    let contexts = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut conversations: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for (a, b, source, thread) in &contexts {
        if indices.contains_key(a) && indices.contains_key(b) {
            let members = conversations
                .entry((source.clone(), thread.clone()))
                .or_default();
            members.insert(a.clone());
            members.insert(b.clone());
        }
    }
    let mut weights: BTreeMap<(String, String), f64> = BTreeMap::new();
    let participation = participation(connection)?;
    for (a, b, source, thread) in &contexts {
        if indices.contains_key(a) && indices.contains_key(b) {
            let count = conversations[&(source.clone(), thread.clone())].len();
            let messages = |person: &String| {
                participation
                    .get(&(source.clone(), thread.clone(), person.clone()))
                    .copied()
                    .unwrap_or(0)
            };
            // Mutual participation is evidence of a shared circle, not direct replies.
            let boost = (messages(a).min(messages(b)) as f64).ln_1p();
            *weights.entry((a.clone(), b.clone())).or_default() +=
                (1.0 + boost) / (count - 1).max(1) as f64;
        }
    }
    let edges = rows
        .into_iter()
        .map(|(a, _, b, _, raw)| {
            (
                indices[&a],
                indices[&b],
                weights.get(&(a, b)).copied().unwrap_or(raw).max(0.000001),
                raw,
            )
        })
        .collect();
    let mut evidence = Vec::new();
    let mut statement = connection.prepare(
        "SELECT source_id, thread_native_id, MAX(conversation_title)
         FROM conversation_memberships WHERE active=1 AND conversation_title IS NOT NULL
         AND TRIM(conversation_title)<>'' GROUP BY source_id, thread_native_id ORDER BY source_id, thread_native_id")?;
    for row in statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (source, thread, label) = row?;
        if let Some(members) = conversations.get(&(source.clone(), thread.clone())) {
            evidence.push(Evidence {
                kind: "conversation".into(),
                label,
                source: format!("{source}:{thread}"),
                members: members.clone(),
            });
        }
    }
    let mut statement = connection.prepare(
        "SELECT person_id, 'tag', tag FROM tags
         UNION ALL SELECT person_id, key, value FROM facts
         WHERE LOWER(key) IN ('company','organization','school','team','club') ORDER BY 2,3,1",
    )?;
    let mut shared: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for row in statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (id, kind, label) = row?;
        if indices.contains_key(&id) && !label.trim().is_empty() {
            shared.entry((kind, label)).or_default().insert(id);
        }
    }
    for ((kind, label), members) in shared {
        evidence.push(Evidence {
            source: format!("{kind}:{label}"),
            kind,
            label,
            members,
        });
    }
    Ok(Input {
        people,
        edges,
        evidence,
    })
}

fn participation(connection: &Connection) -> Result<BTreeMap<(String, String, String), i64>> {
    let mut statement = connection.prepare(
        "SELECT i.source_id, i.thread_native_id, p.person_id, COUNT(DISTINCT i.id)
         FROM interactions i JOIN interaction_participants p ON p.interaction_id=i.id
         WHERE i.deleted_at IS NULL AND i.thread_native_id IS NOT NULL
           AND p.role='sender' AND p.person_id IS NOT NULL
           AND NOT EXISTS(SELECT 1 FROM identities own WHERE own.person_id=p.person_id AND own.is_self=1)
           AND EXISTS(SELECT 1 FROM relationship_contexts r
                      WHERE r.source_id=i.source_id AND r.thread_native_id=i.thread_native_id)
         GROUP BY i.source_id, i.thread_native_id, p.person_id
         ORDER BY i.source_id, i.thread_native_id, p.person_id",
    )?;
    Ok(statement
        .query_map([], |r| Ok(((r.get(0)?, r.get(1)?, r.get(2)?), r.get(3)?)))?
        .collect::<std::result::Result<_, _>>()?)
}

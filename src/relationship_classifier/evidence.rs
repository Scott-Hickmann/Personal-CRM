use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, params};
use serde_json::{Value, json};

use super::PromptMessage;
use super::markdown::render_markdown;
use crate::error::{CrmError, Result};

const MAX_MESSAGES: usize = 12;
const MAX_PER_CONTEXT: usize = 4;
const MAX_PER_AUTHOR: usize = 4;
const MAX_BODY_CHARS: usize = 1_000;
const MAX_INPUT_CHARS: usize = 12_000;

pub(super) struct Candidate {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub source_name: String,
    pub target_name: String,
    pub structure_revision: i64,
}

#[derive(Clone)]
pub(super) struct Context {
    pub source_id: String,
    pub thread_native_id: String,
    pub channel: String,
    pub title: Option<String>,
    pub member_count: i64,
    pub first_observed_at: String,
    pub last_observed_at: String,
}

pub(super) struct EvidenceMessage {
    pub id: String,
    pub context_key: String,
    pub occurred_at: String,
    pub author_id: Option<String>,
    pub author_name: String,
    pub direction: Option<String>,
    pub pair_explicit: bool,
    pub subject: Option<String>,
    pub body: String,
    pub member_count: i64,
    pub bucket: u8,
}

pub(super) struct Prepared {
    pub messages: Vec<PromptMessage>,
    pub message_ids: HashSet<String>,
}

pub(super) fn prepare(
    connection: &Connection,
    candidate: &Candidate,
    template: &str,
    schema_template: &Value,
) -> Result<Option<Prepared>> {
    let contexts = contexts(connection, candidate)?;
    let mut selected = select_messages(connection, candidate, &contexts)?;
    if selected.is_empty() {
        return Ok(None);
    }
    let mut markdown = render_markdown(candidate, &contexts, &selected);
    while markdown.chars().count() > MAX_INPUT_CHARS && selected.len() > 1 {
        let worst = selected
            .iter()
            .enumerate()
            .max_by_key(|(_, message)| (message.bucket, message.member_count, &message.occurred_at))
            .map(|(index, _)| index)
            .unwrap();
        selected.remove(worst);
        markdown = render_markdown(candidate, &contexts, &selected);
    }
    let message_ids = selected.iter().map(|item| item.id.clone()).collect();
    let schema = classification_schema(schema_template, &candidate.id, &message_ids)?;
    let prompt = replace_once(
        template,
        "{{json_schema}}",
        &serde_json::to_string(&schema).map_err(serialization)?,
    )?;
    Ok(Some(Prepared {
        messages: vec![
            PromptMessage {
                role: "system",
                content: prompt,
            },
            PromptMessage {
                role: "user",
                content: markdown,
            },
        ],
        message_ids,
    }))
}

fn contexts(connection: &Connection, candidate: &Candidate) -> Result<Vec<Context>> {
    let mut statement = connection.prepare(
        "SELECT rc.source_id, rc.thread_native_id, rc.channel,
                COALESCE((SELECT i.subject FROM interactions i
                 WHERE i.source_id=rc.source_id AND i.thread_native_id=rc.thread_native_id
                   AND i.deleted_at IS NULL AND i.subject IS NOT NULL AND trim(i.subject) != ''
                 ORDER BY i.occurred_at DESC LIMIT 1),
                 (SELECT MAX(cm.conversation_title) FROM conversation_memberships cm
                  WHERE cm.source_id=rc.source_id AND cm.thread_native_id=rc.thread_native_id
                    AND cm.active=1)),
                (SELECT COUNT(DISTINCT cm.person_id) FROM conversation_memberships cm
                 JOIN people p ON p.id=cm.person_id AND p.lifecycle_state='active'
                 WHERE cm.source_id=rc.source_id AND cm.thread_native_id=rc.thread_native_id
                   AND cm.active=1 AND cm.person_id IS NOT NULL),
                rc.first_observed_at, rc.last_observed_at
         FROM relationship_contexts rc
         WHERE rc.source_person_id=?1 AND rc.target_person_id=?2
         ORDER BY rc.last_observed_at DESC, rc.source_id, rc.thread_native_id",
    )?;
    statement
        .query_map(params![candidate.source_id, candidate.target_id], |row| {
            Ok(Context {
                source_id: row.get(0)?,
                thread_native_id: row.get(1)?,
                channel: row.get(2)?,
                title: row.get(3)?,
                member_count: row.get(4)?,
                first_observed_at: row.get(5)?,
                last_observed_at: row.get(6)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()
        .map_err(Into::into)
}

fn select_messages(
    connection: &Connection,
    candidate: &Candidate,
    contexts: &[Context],
) -> Result<Vec<EvidenceMessage>> {
    let context_counts: HashMap<_, _> = contexts
        .iter()
        .map(|context| {
            (
                format!("{}\0{}", context.source_id, context.thread_native_id),
                context.member_count,
            )
        })
        .collect();
    let mut statement = connection.prepare(
        "SELECT i.id, i.source_id, i.thread_native_id, i.occurred_at, i.direction,
                i.subject, i.body,
                (SELECT ip.person_id FROM interaction_participants ip
                 WHERE ip.interaction_id=i.id AND ip.role IN ('sender', 'caller')
                   AND ip.person_id IS NOT NULL LIMIT 1),
                (SELECT p.display_name FROM interaction_participants ip
                 JOIN people p ON p.id=ip.person_id
                 WHERE ip.interaction_id=i.id AND ip.role IN ('sender', 'caller')
                   AND ip.person_id IS NOT NULL LIMIT 1),
                EXISTS(SELECT 1 FROM interaction_participants ip
                       WHERE ip.interaction_id=i.id AND ip.person_id=?1)
                AND EXISTS(SELECT 1 FROM interaction_participants ip
                           WHERE ip.interaction_id=i.id AND ip.person_id=?2)
         FROM interactions i JOIN relationship_contexts rc
           ON rc.source_id=i.source_id AND rc.thread_native_id=i.thread_native_id
         WHERE rc.source_person_id=?1 AND rc.target_person_id=?2
           AND i.deleted_at IS NULL AND i.body IS NOT NULL AND trim(i.body) != ''",
    )?;
    let rows = statement
        .query_map(params![candidate.source_id, candidate.target_id], |row| {
            let source_id: String = row.get(1)?;
            let thread_native_id: String = row.get(2)?;
            let author_id: Option<String> = row.get(7)?;
            let direction: Option<String> = row.get(4)?;
            Ok(EvidenceMessage {
                id: row.get(0)?,
                context_key: format!("{source_id}\0{thread_native_id}"),
                occurred_at: row.get(3)?,
                author_name: row.get::<_, Option<String>>(8)?.unwrap_or_else(|| {
                    if direction.as_deref() == Some("outgoing") {
                        "CRM owner".into()
                    } else {
                        "Other participant".into()
                    }
                }),
                author_id,
                direction,
                pair_explicit: row.get(9)?,
                subject: row.get(5)?,
                body: row.get(6)?,
                member_count: *context_counts
                    .get(&format!("{source_id}\0{thread_native_id}"))
                    .unwrap_or(&i64::MAX),
                bucket: 0,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut eligible = rows
        .into_iter()
        .filter_map(|mut message| {
            message.bucket = priority(&message, candidate)?;
            message.body = message.body.chars().take(MAX_BODY_CHARS).collect();
            Some(message)
        })
        .collect::<Vec<_>>();
    eligible.sort_by(|left, right| {
        left.bucket
            .cmp(&right.bucket)
            .then(left.member_count.cmp(&right.member_count))
            .then(right.occurred_at.cmp(&left.occurred_at))
            .then(left.id.cmp(&right.id))
    });
    let mut context_counts = HashMap::<String, usize>::new();
    let mut author_counts = HashMap::<String, usize>::new();
    let mut selected = Vec::new();
    for message in eligible {
        let author = message
            .author_id
            .clone()
            .unwrap_or_else(|| message.author_name.clone());
        if context_counts
            .get(&message.context_key)
            .copied()
            .unwrap_or(0)
            >= MAX_PER_CONTEXT
            || author_counts.get(&author).copied().unwrap_or(0) >= MAX_PER_AUTHOR
        {
            continue;
        }
        *context_counts
            .entry(message.context_key.clone())
            .or_default() += 1;
        *author_counts.entry(author).or_default() += 1;
        selected.push(message);
        if selected.len() == MAX_MESSAGES {
            break;
        }
    }
    selected.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then(left.id.cmp(&right.id))
    });
    Ok(selected)
}

fn priority(message: &EvidenceMessage, candidate: &Candidate) -> Option<u8> {
    let body = message.body.to_lowercase();
    let names_source = body.contains(&candidate.source_name.to_lowercase());
    let names_target = body.contains(&candidate.target_name.to_lowercase());
    match message.author_id.as_deref() {
        Some(id) if id == candidate.source_id && names_target => Some(0),
        Some(id) if id == candidate.target_id && names_source => Some(0),
        _ if names_source && names_target => Some(1),
        Some(id) if id == candidate.source_id || id == candidate.target_id => Some(2),
        _ if message.direction.as_deref() == Some("outgoing") && message.pair_explicit => Some(3),
        _ => None,
    }
}

fn classification_schema(
    template: &Value,
    relationship_id: &str,
    message_ids: &HashSet<String>,
) -> Result<Value> {
    let mut schema = template.clone();
    *schema
        .pointer_mut("/properties/relationship_id/const")
        .ok_or_else(|| CrmError::Serialization("classification schema has no ID const".into()))? =
        json!(relationship_id);
    let mut ids = message_ids.iter().cloned().collect::<Vec<_>>();
    ids.sort();
    *schema
        .pointer_mut("/properties/evidence_message_ids/items/enum")
        .ok_or_else(|| {
            CrmError::Serialization("classification schema has no message ID enum".into())
        })? = json!(ids);
    Ok(schema)
}

fn replace_once(template: &str, marker: &str, value: &str) -> Result<String> {
    if template.matches(marker).count() != 1 {
        return Err(CrmError::Serialization(format!(
            "prompt must contain exactly one {marker} marker"
        )));
    }
    Ok(template.replace(marker, value))
}

fn serialization(error: serde_json::Error) -> CrmError {
    CrmError::Serialization(error.to_string())
}

#[cfg(test)]
#[path = "evidence/tests.rs"]
mod tests;

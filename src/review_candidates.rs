use std::collections::{BTreeMap, BTreeSet, HashSet};

use rusqlite::{Connection, params};

use crate::error::Result;
use crate::review;

pub(crate) fn enqueue(connection: &Connection) -> Result<usize> {
    resolve_ineligible(connection)?;
    let mut statement = connection.prepare(
        "SELECT lower(trim(ip.identity_value)), COUNT(*), group_concat(DISTINCT i.channel),
                MAX(NULLIF(trim(ip.display_name), ''))
         FROM interaction_participants ip JOIN interactions i ON i.id=ip.interaction_id
         WHERE ip.person_id IS NULL AND ip.identity_value IS NOT NULL
           AND trim(ip.identity_value) != '' AND i.deleted_at IS NULL
           AND lower(i.channel) IN ('whatsapp', 'whatsapp_call', 'gmail')
         GROUP BY lower(trim(ip.identity_value)) ORDER BY COUNT(*) DESC",
    )?;
    let rows: Vec<(String, i64, String, Option<String>)> = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    let mut grouped = BTreeMap::<String, Candidate>::new();
    for (identity, count, channels, name) in rows {
        if is_non_person_whatsapp_identity(&identity) {
            continue;
        }
        let normalized = crate::repository::normalize_observed_identity(&identity);
        if normalized.is_empty() {
            continue;
        }
        let channel_set = channels
            .split(',')
            .map(str::trim)
            .filter(|channel| !channel.is_empty())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let entry = grouped
            .entry(normalized.clone())
            .or_insert_with(|| Candidate {
                identity: contact_identity(&identity, &normalized, &channel_set),
                count: 0,
                channels: BTreeSet::new(),
                name: None,
            });
        entry.count += count;
        entry.channels.extend(channel_set);
        if entry.name.is_none() {
            entry.name = name;
        }
    }
    let mut candidates = Vec::new();
    for (_, candidate) in grouped {
        if !identity_belongs_to_icloud_contact(connection, &candidate.identity)? {
            candidates.push(candidate);
        }
    }
    candidates.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then(left.identity.cmp(&right.identity))
    });
    let mut active_subjects = HashSet::new();
    for candidate in &candidates {
        let channels = candidate
            .channels
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(",");
        let (phone, email) = if candidate.identity.contains('@') {
            (None, Some(candidate.identity.as_str()))
        } else {
            (Some(candidate.identity.as_str()), None)
        };
        let label = crate::contact_label::format(candidate.name.as_deref(), phone, email);
        let sources = source_labels(channels.split(','));
        review::enqueue(
            connection,
            "contact_candidate",
            &candidate.identity,
            &format!("Create an iCloud contact for {label}?"),
            serde_json::json!({"name": candidate.name, "identity": candidate.identity, "interaction_count": candidate.count, "channels": channels, "sources": sources}),
        )?;
        active_subjects.insert(candidate.identity.clone());
    }
    resolve_stale_communication_candidates(connection, &active_subjects)?;
    Ok(candidates.len())
}

struct Candidate {
    identity: String,
    count: i64,
    channels: BTreeSet<String>,
    name: Option<String>,
}

fn contact_identity(identity: &str, normalized: &str, channels: &BTreeSet<String>) -> String {
    if normalized
        .chars()
        .all(|character| character.is_ascii_digit())
        && channels
            .iter()
            .any(|channel| channel.to_ascii_lowercase().starts_with("whatsapp"))
    {
        format!("+{normalized}")
    } else {
        identity.to_owned()
    }
}

fn resolve_stale_communication_candidates(
    connection: &Connection,
    active_subjects: &HashSet<String>,
) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT id, subject_key, details_json FROM review_items
         WHERE kind='contact_candidate' AND status='pending'",
    )?;
    let rows: Vec<(String, String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    for (id, subject, details) in rows {
        let details: serde_json::Value = serde_json::from_str(&details).unwrap_or_default();
        let is_google = details.get("source").and_then(|value| value.as_str()) == Some("google");
        if !is_google && has_supported_source(&details) && !active_subjects.contains(&subject) {
            review::resolve(connection, &id)?;
        }
    }
    Ok(())
}

fn resolve_ineligible(connection: &Connection) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT id, subject_key, details_json FROM review_items
         WHERE kind='contact_candidate' AND status='pending'",
    )?;
    let rows: Vec<(String, String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);
    for (id, subject, details) in rows {
        let details = serde_json::from_str(&details).unwrap_or_default();
        if is_non_person_whatsapp_identity(&subject)
            || !has_supported_source(&details)
            || candidate_matches_icloud_contact(connection, &subject, &details)?
        {
            review::resolve(connection, &id)?;
        }
    }
    Ok(())
}

fn has_supported_source(details: &serde_json::Value) -> bool {
    if details.get("source").and_then(|value| value.as_str()) == Some("google") {
        return true;
    }
    details
        .get("channels")
        .and_then(|value| value.as_str())
        .is_some_and(|channels| {
            channels.split(',').any(|channel| {
                matches!(
                    channel.trim().to_ascii_lowercase().as_str(),
                    "whatsapp" | "whatsapp_call" | "gmail"
                )
            })
        })
}

pub(crate) fn identity_belongs_to_icloud_contact(
    connection: &Connection,
    identity: &str,
) -> Result<bool> {
    let normalized = crate::repository::normalize_observed_identity(identity);
    connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM identities i JOIN people p ON p.id=i.person_id
                 WHERE i.normalized_value=?1 AND i.active=1
                   AND p.lifecycle_state='active' AND p.apple_contact_id IS NOT NULL
                 UNION ALL
                 SELECT 1 FROM excluded_icloud_identities
                 WHERE normalized_value=?1
             )",
            [normalized],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

pub(crate) fn resolve_pending_subject(
    connection: &Connection,
    kind: &str,
    subject: &str,
) -> Result<()> {
    connection.execute(
        "UPDATE review_items SET status='resolved', resolved_at=CURRENT_TIMESTAMP,
         updated_at=CURRENT_TIMESTAMP WHERE kind=?1 AND subject_key=?2 AND status='pending'",
        params![kind, subject],
    )?;
    Ok(())
}

fn candidate_matches_icloud_contact(
    connection: &Connection,
    subject: &str,
    details: &serde_json::Value,
) -> Result<bool> {
    let mut identities = Vec::new();
    if let Some(identity) = details.get("identity").and_then(|value| value.as_str()) {
        identities.push(identity);
    }
    for key in ["emails", "phones"] {
        identities.extend(
            details
                .get(key)
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
                .filter_map(|item| item.get("value").and_then(|value| value.as_str())),
        );
    }
    if identities.is_empty() && !subject.starts_with("google:") {
        identities.push(subject);
    }
    for identity in identities {
        if identity_belongs_to_icloud_contact(connection, identity)? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn source(details: &serde_json::Value) -> Option<String> {
    let mut sources = details
        .get("sources")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    if sources.is_empty()
        && let Some(channels) = details.get("channels").and_then(|value| value.as_str())
    {
        sources = source_labels(channels.split(','));
    }
    if sources.is_empty()
        && details.get("source").and_then(|value| value.as_str()) == Some("google")
    {
        let account = details.get("account").and_then(|value| value.as_str());
        sources.push(account.map_or_else(|| "Google".into(), |value| format!("Google ({value})")));
    }
    (!sources.is_empty()).then(|| sources.join(", "))
}

fn source_labels<'a>(sources: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut labels = sources
        .map(|source| match source.trim().to_ascii_lowercase().as_str() {
            "whatsapp" | "whatsapp_call" => "WhatsApp".to_owned(),
            "imessage" => "iMessage".to_owned(),
            "sms" => "SMS".to_owned(),
            "rcs" => "RCS".to_owned(),
            "apple_call" => "Apple Calls".to_owned(),
            "gmail" => "Gmail".to_owned(),
            _ => source.trim().to_owned(),
        })
        .filter(|source| !source.is_empty())
        .collect::<Vec<_>>();
    labels.sort();
    labels.dedup();
    labels
}

fn is_non_person_whatsapp_identity(identity: &str) -> bool {
    let identity = identity.trim().to_ascii_lowercase();
    ["@g.us", "@broadcast", "@newsletter", "@lid"]
        .iter()
        .any(|suffix| identity.ends_with(suffix))
}

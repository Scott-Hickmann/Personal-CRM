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
    let mut candidates = Vec::new();
    for row in rows {
        if !is_non_person_whatsapp_identity(&row.0)
            && !identity_belongs_to_icloud_contact(connection, &row.0)?
        {
            candidates.push(row);
        }
    }
    for (identity, count, channels, name) in &candidates {
        let label = name.as_deref().unwrap_or(identity);
        let sources = source_labels(channels.split(','));
        review::enqueue(
            connection,
            "contact_candidate",
            identity,
            &format!("Create an iCloud contact for {label}?"),
            serde_json::json!({"name": name, "identity": identity, "interaction_count": count, "channels": channels, "sources": sources}),
        )?;
    }
    Ok(candidates.len())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn interaction(connection: &Connection, id: &str, channel: &str, identity: &str, name: &str) {
        connection
            .execute(
                "INSERT INTO interactions(
                     id, source_id, native_id, channel, kind, occurred_at, last_seen_at
                 ) VALUES (?1, 'whatsapp', ?1, ?2, 'message',
                           '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                params![id, channel],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO interaction_participants(
                     interaction_id, identity_value, display_name, role
                 ) VALUES (?1, ?2, ?3, 'sender')",
                params![id, identity, name],
            )
            .unwrap();
    }

    #[test]
    fn candidate_includes_source_and_name() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute(
                "INSERT INTO sources(id, kind) VALUES ('whatsapp', 'whatsapp')",
                [],
            )
            .unwrap();
        interaction(
            &connection,
            "message",
            "whatsapp",
            "15550100@s.whatsapp.net",
            "Alex",
        );

        enqueue(&connection).unwrap();

        let item = review::pending(&connection).unwrap().pop().unwrap();
        assert_eq!(item.source.as_deref(), Some("WhatsApp"));
        assert_eq!(item.details["name"], "Alex");
    }

    #[test]
    fn group_and_existing_contact_are_ineligible() {
        let directory = tempfile::tempdir().unwrap();
        let connection = db::open(&directory.path().join("crm.sqlite3")).unwrap();
        connection
            .execute_batch(
                "INSERT INTO sources(id, kind) VALUES ('whatsapp', 'whatsapp');
                 INSERT INTO people(id, display_name, apple_contact_id, lifecycle_state)
                 VALUES ('person', 'Alex', 'apple-1', 'active');
                 INSERT INTO identities(id, person_id, kind, value, normalized_value, active)
                 VALUES ('phone', 'person', 'phone', '+1 555 0100', '15550100', 1);",
            )
            .unwrap();
        interaction(
            &connection,
            "group",
            "whatsapp",
            "120363000000@g.us",
            "Family",
        );
        interaction(
            &connection,
            "person",
            "whatsapp",
            "15550100@s.whatsapp.net",
            "Alex",
        );
        interaction(&connection, "sms", "SMS", "+16660100", "Unknown");
        review::enqueue(
            &connection,
            "contact_candidate",
            "15550100@s.whatsapp.net",
            "Create an iCloud contact for Alex?",
            serde_json::json!({
                "identity": "15550100@s.whatsapp.net",
                "channels": "whatsapp"
            }),
        )
        .unwrap();
        review::enqueue(
            &connection,
            "contact_candidate",
            "+16660100",
            "Create an iCloud contact for Unknown?",
            serde_json::json!({"identity": "+16660100", "channels": "SMS"}),
        )
        .unwrap();

        assert_eq!(enqueue(&connection).unwrap(), 0);
        assert!(review::pending(&connection).unwrap().is_empty());
    }
}

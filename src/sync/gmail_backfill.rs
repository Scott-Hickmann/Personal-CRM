use std::collections::HashSet;

use rusqlite::{Connection, params};

use crate::error::{CrmError, Result};
use crate::gmail::{ApiClient, ApiResponse, MessageList};
use crate::progress::{ProgressStage, ProgressTracker};

const POLICY_VERSION: u8 = 1;
const DISCOVERY_YEARS: u8 = 2;
const SCAN_BATCH_SIZE: i64 = 25;
pub(super) const MESSAGE_BATCH_SIZE: i64 = 50;

#[derive(Debug)]
pub(super) struct QueuedMessage {
    pub id: String,
    pub known_scope: bool,
}

#[derive(Debug, Default)]
pub(super) struct ScanReport {
    pub pages: usize,
    pub messages_found: usize,
}

pub(super) fn seed(
    crm: &Connection,
    source_id: &str,
    self_addresses: &HashSet<String>,
) -> Result<()> {
    let mut statement = crm.prepare(
        "SELECT DISTINCT lower(trim(i.normalized_value))
         FROM identities i JOIN people p ON p.id=i.person_id
         WHERE i.kind='email' AND i.active=1 AND i.is_self=0
           AND p.lifecycle_state='active' AND p.apple_contact_id IS NOT NULL
         ORDER BY lower(trim(i.normalized_value))",
    )?;
    let emails: Vec<String> = statement
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);

    let active_keys: HashSet<String> = emails
        .iter()
        .filter(|email| email.contains('@') && !self_addresses.contains(*email))
        .map(|email| contact_scope_key(email))
        .collect();
    let mut old_statement = crm.prepare(
        "SELECT scope_key FROM gmail_sync_scopes
         WHERE source_id=?1 AND kind='contact' AND completed_at IS NULL",
    )?;
    let obsolete: Vec<String> = old_statement
        .query_map([source_id], |row| row.get(0))?
        .collect::<std::result::Result<Vec<String>, _>>()?
        .into_iter()
        .filter(|key| !active_keys.contains(key))
        .collect();
    drop(old_statement);
    for key in obsolete {
        crm.execute(
            "UPDATE gmail_sync_scopes SET completed_at=CURRENT_TIMESTAMP,
             updated_at=CURRENT_TIMESTAMP WHERE source_id=?1 AND scope_key=?2",
            params![source_id, key],
        )?;
    }

    for email in emails {
        if !email.contains('@') || self_addresses.contains(&email) {
            continue;
        }
        let query = format!(
            "{{from:{email} to:{email} cc:{email} bcc:{email}}} -category:promotions \
             -category:social -category:forums -in:spam -in:trash"
        );
        insert_scope(
            crm,
            source_id,
            &contact_scope_key(&email),
            "contact",
            &query,
        )?;
    }
    insert_scope(
        crm,
        source_id,
        &format!("discovery:v{POLICY_VERSION}:sent:{DISCOVERY_YEARS}y"),
        "discovery",
        &format!("in:sent newer_than:{DISCOVERY_YEARS}y -in:spam -in:trash"),
    )?;
    Ok(())
}

fn insert_scope(
    crm: &Connection,
    source_id: &str,
    key: &str,
    kind: &str,
    query: &str,
) -> Result<()> {
    crm.execute(
        "INSERT INTO gmail_sync_scopes(source_id, scope_key, kind, query)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(source_id, scope_key) DO UPDATE SET query=excluded.query",
        params![source_id, key, kind, query],
    )?;
    Ok(())
}

fn contact_scope_key(email: &str) -> String {
    format!("contact:v{POLICY_VERSION}:{email}")
}

pub(super) fn scan(
    crm: &Connection,
    client: &ApiClient,
    source_id: &str,
    account: &str,
    stage: ProgressStage,
    progress: &mut ProgressTracker,
) -> Result<ScanReport> {
    let (completed, total) = scope_counts(crm, source_id)?;
    progress.stage(
        format!("Finding messages involving people in {account}"),
        stage.current,
        stage.total,
        total,
        false,
        "contact searches",
    );
    progress.progress_now(
        format!("Finding messages involving people in {account}"),
        completed,
        total,
        false,
        "contact searches",
    );
    let mut statement = crm.prepare(
        "SELECT scope_key, kind, query, page_token
         FROM gmail_sync_scopes WHERE source_id=?1 AND completed_at IS NULL
         ORDER BY updated_at, scope_key LIMIT ?2",
    )?;
    let scopes: Vec<(String, String, String, Option<String>)> = statement
        .query_map(params![source_id, SCAN_BATCH_SIZE], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    drop(statement);

    let mut report = ScanReport::default();
    for (scope_key, kind, query, page_token) in scopes {
        let contact = scope_key.rsplit(':').next().unwrap_or(&scope_key);
        progress.focus_now([format!("{contact} · Gmail search · {account}")]);
        let path = list_path(&query, page_token.as_deref());
        let page: MessageList = match client.get(&path)? {
            ApiResponse::Data(page) => page,
            ApiResponse::NotFound => {
                return Err(CrmError::Network("Gmail message search disappeared".into()));
            }
        };
        let found = page.messages.len();
        let transaction = crate::db::immediate_transaction(crm)?;
        for message in page.messages {
            enqueue_from_scope(&transaction, source_id, &message.id, &kind)?;
        }
        transaction.execute(
            "UPDATE gmail_sync_scopes SET page_token=?3,
             messages_found=messages_found+?4,
             completed_at=CASE WHEN ?3 IS NULL THEN CURRENT_TIMESTAMP ELSE NULL END,
             updated_at=CURRENT_TIMESTAMP WHERE source_id=?1 AND scope_key=?2",
            params![source_id, scope_key, page.next_page_token, found as i64],
        )?;
        transaction.commit()?;
        report.pages += 1;
        report.messages_found += found;
        let (completed, total) = scope_counts(crm, source_id)?;
        progress.progress_now(
            format!(
                "Finding messages involving people in {account} ({} queued from latest search)",
                found
            ),
            completed,
            total,
            false,
            "contact searches",
        );
    }
    let (completed, total) = scope_counts(crm, source_id)?;
    if completed == total {
        progress.finish_stage(
            format!("Found people-focused Gmail history in {account}"),
            total,
            total,
            false,
            "contact searches",
        );
    }
    Ok(report)
}

fn list_path(query: &str, page_token: Option<&str>) -> String {
    let mut pairs = url::form_urlencoded::Serializer::new(String::new());
    pairs
        .append_pair("includeSpamTrash", "false")
        .append_pair("maxResults", "500")
        .append_pair("q", query);
    if let Some(token) = page_token {
        pairs.append_pair("pageToken", token);
    }
    format!("messages?{}", pairs.finish())
}

fn enqueue_from_scope(
    crm: &Connection,
    source_id: &str,
    message_id: &str,
    kind: &str,
) -> Result<()> {
    let known = i64::from(kind == "contact");
    let discovery = i64::from(kind == "discovery");
    crm.execute(
        "INSERT INTO gmail_message_state(
             source_id, message_id, known_scope, discovery_scope
         ) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(source_id, message_id) DO UPDATE SET
           known_scope=MAX(gmail_message_state.known_scope, excluded.known_scope),
           discovery_scope=MAX(gmail_message_state.discovery_scope, excluded.discovery_scope),
           status=CASE
             WHEN gmail_message_state.status='skipped'
              AND excluded.known_scope>gmail_message_state.known_scope THEN 'queued'
             ELSE gmail_message_state.status END,
           reason=CASE
             WHEN gmail_message_state.status='skipped'
              AND excluded.known_scope>gmail_message_state.known_scope THEN NULL
             ELSE gmail_message_state.reason END,
           updated_at=CASE
             WHEN gmail_message_state.status='skipped'
              AND excluded.known_scope>gmail_message_state.known_scope
             THEN CURRENT_TIMESTAMP ELSE gmail_message_state.updated_at END",
        params![source_id, message_id, known, discovery],
    )?;
    Ok(())
}

pub(super) fn enqueue_changed(crm: &Connection, source_id: &str, message_id: &str) -> Result<()> {
    crm.execute(
        "INSERT INTO gmail_message_state(source_id, message_id, discovery_scope, status)
         VALUES (?1, ?2, 1, 'queued')
         ON CONFLICT(source_id, message_id) DO UPDATE SET
           status='queued', reason=NULL, updated_at=CURRENT_TIMESTAMP",
        params![source_id, message_id],
    )?;
    Ok(())
}

pub(super) fn mark(
    crm: &Connection,
    source_id: &str,
    message_id: &str,
    status: &str,
    reason: Option<&str>,
) -> Result<()> {
    crm.execute(
        "UPDATE gmail_message_state SET status=?3, reason=?4,
         updated_at=CURRENT_TIMESTAMP WHERE source_id=?1 AND message_id=?2",
        params![source_id, message_id, status, reason],
    )?;
    Ok(())
}

pub(super) fn queued(crm: &Connection, source_id: &str) -> Result<(Vec<QueuedMessage>, u64)> {
    let total: i64 = crm.query_row(
        "SELECT COUNT(*) FROM gmail_message_state WHERE source_id=?1 AND status='queued'",
        [source_id],
        |row| row.get(0),
    )?;
    let mut statement = crm.prepare(
        "SELECT message_id, known_scope FROM gmail_message_state
         WHERE source_id=?1 AND status='queued'
         ORDER BY updated_at, message_id LIMIT ?2",
    )?;
    let messages = statement
        .query_map(params![source_id, MESSAGE_BATCH_SIZE], |row| {
            Ok(QueuedMessage {
                id: row.get(0)?,
                known_scope: row.get::<_, i64>(1)? != 0,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok((messages, total as u64))
}

pub(super) fn reset(crm: &Connection, source_id: &str) -> Result<()> {
    let transaction = crate::db::immediate_transaction(crm)?;
    transaction.execute(
        "UPDATE gmail_sync_scopes SET page_token=NULL, messages_found=0,
         completed_at=NULL, updated_at=CURRENT_TIMESTAMP WHERE source_id=?1",
        [source_id],
    )?;
    transaction.execute(
        "UPDATE gmail_message_state SET status='queued', reason=NULL,
         updated_at=CURRENT_TIMESTAMP WHERE source_id=?1",
        [source_id],
    )?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn reset_all(crm: &Connection) -> Result<()> {
    let transaction = crate::db::immediate_transaction(crm)?;
    transaction.execute(
        "UPDATE gmail_sync_scopes SET page_token=NULL, messages_found=0,
         completed_at=NULL, updated_at=CURRENT_TIMESTAMP",
        [],
    )?;
    transaction.execute(
        "UPDATE gmail_message_state SET status='queued', reason=NULL,
         updated_at=CURRENT_TIMESTAMP",
        [],
    )?;
    transaction.commit()?;
    Ok(())
}

pub(crate) fn has_pending(crm: &Connection) -> Result<bool> {
    crm.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM gmail_sync_scopes scope JOIN sources source ON source.id=scope.source_id
             WHERE scope.completed_at IS NULL AND source.last_sync_at IS NOT NULL
               AND julianday(source.last_sync_at)>=julianday('now', '-5 minutes')
             UNION ALL
             SELECT 1 FROM gmail_message_state message JOIN sources source ON source.id=message.source_id
             WHERE message.status='queued' AND source.last_sync_at IS NOT NULL
               AND julianday(source.last_sync_at)>=julianday('now', '-5 minutes')
         )",
        [],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn scope_counts(crm: &Connection, source_id: &str) -> Result<(u64, u64)> {
    let counts: (i64, i64) = crm.query_row(
        "SELECT COALESCE(SUM(completed_at IS NOT NULL), 0), COUNT(*)
         FROM gmail_sync_scopes WHERE source_id=?1",
        [source_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok((counts.0 as u64, counts.1 as u64))
}

#[cfg(test)]
#[path = "gmail_backfill_tests.rs"]
mod tests;

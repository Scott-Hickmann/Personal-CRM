use std::collections::HashSet;

use chrono::{TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use super::gmail_message::{addresses, header, import_attachments, is_automated, message_text};
use super::{SyncReport, add_participant, finish_source, upsert_interaction};
use crate::error::{CrmError, Result};
use crate::gmail::{ApiClient, ApiResponse, Credentials, GmailMessage, HistoryPage, MessageList};
use crate::progress::{ProgressStage, ProgressTracker};

struct GmailSource<'a> {
    id: &'a str,
    account: &'a str,
    stage: ProgressStage,
}

pub fn sync(
    config: &crate::config::Config,
    crm: &Connection,
    progress: &mut ProgressTracker,
) -> Result<Vec<SyncReport>> {
    let credentials_path = config.gmail.credentials_path.as_ref().ok_or_else(|| {
        CrmError::InvalidConfig("Gmail credentials path is not configured".into())
    })?;
    let credentials = Credentials::load(credentials_path)?;
    let account_count = config.gmail.accounts.len();
    let mut reports = Vec::with_capacity(account_count);
    for (index, account) in config.gmail.accounts.iter().enumerate() {
        let stage_current = (index + 1) as u64;
        let stage_total = account_count as u64;
        let stage = ProgressStage {
            current: stage_current,
            total: stage_total,
        };
        progress.stage(
            format!("Connecting to Gmail inbox {account}"),
            stage.current,
            stage.total,
            1,
            false,
            "connection",
        );
        let report = sync_account(config, crm, &credentials, account, progress, stage)?;
        progress.event(format!(
            "Finished {account}: {} imported, {} deleted",
            report.imported, report.deleted
        ));
        reports.push(report);
    }
    Ok(reports)
}

fn sync_account(
    config: &crate::config::Config,
    crm: &Connection,
    credentials: &Credentials,
    account: &str,
    progress: &mut ProgressTracker,
    stage: ProgressStage,
) -> Result<SyncReport> {
    let source_id = format!("gmail:{account}");
    let client = ApiClient::for_account(credentials, account)?;
    let cursor: Option<String> = crm
        .query_row(
            "SELECT cursor FROM sources WHERE id = ?1",
            [&source_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    crm.execute(
        "INSERT INTO sources(id, kind, account, status) VALUES (?1, 'gmail', ?2, 'syncing')
         ON CONFLICT(id) DO UPDATE SET account=excluded.account, status='syncing', error=NULL",
        params![source_id, account],
    )?;
    let source = GmailSource {
        id: &source_id,
        account,
        stage,
    };
    if let Some(cursor) = cursor
        && let Some(report) = partial_sync(config, crm, &client, &source, &cursor, progress)?
    {
        return Ok(report);
    }
    full_sync(config, crm, &client, &source, progress)
}

fn full_sync(
    config: &crate::config::Config,
    crm: &Connection,
    client: &ApiClient,
    source: &GmailSource<'_>,
    progress: &mut ProgressTracker,
) -> Result<SyncReport> {
    let account = source.account;
    let run_at = Utc::now().to_rfc3339();
    let mut page_token: Option<String> = None;
    let mut imported = HashSet::new();
    let mut latest_history = 0_u64;
    let mut processed = 0_u64;
    let mut total = None;
    loop {
        let mut path = "messages?includeSpamTrash=false&maxResults=500".to_owned();
        if let Some(token) = &page_token {
            path.push_str("&pageToken=");
            path.push_str(token);
        }
        let ApiResponse::Data(page): ApiResponse<MessageList> = client.get(&path)? else {
            return Err(CrmError::Network(
                "Gmail message listing disappeared".into(),
            ));
        };
        let first_page = total.is_none();
        total = Some(total.unwrap_or(0).max(page.result_size_estimate));
        if first_page {
            progress.stage(
                format!("Reading emails from {account}"),
                source.stage.current,
                source.stage.total,
                total.unwrap_or(0),
                true,
                "emails",
            );
        }
        for message in page.messages {
            if let Some(history) =
                import_message(config, crm, client, source.id, &message.id, &run_at)?
            {
                imported.insert(message.id);
                latest_history = latest_history.max(history);
            }
            processed += 1;
            progress.progress(
                format!("Reading emails from {account}"),
                processed,
                total.unwrap_or(processed).max(processed),
                true,
                "emails",
            );
        }
        page_token = page.next_page_token;
        if page_token.is_none() {
            break;
        }
    }
    progress.finish_stage(
        format!("Read emails from {account}"),
        processed,
        processed,
        false,
        "emails",
    );
    progress.progress_now(
        format!("Finalizing Gmail sync for {account}"),
        0,
        1,
        false,
        "step",
    );
    let deleted = finish_source(crm, source.id, &run_at)?;
    crm.execute(
        "UPDATE sources SET cursor = ?2 WHERE id = ?1",
        params![source.id, latest_history.to_string()],
    )?;
    progress.finish_stage(
        format!("Finalized Gmail sync for {account}"),
        1,
        1,
        false,
        "step",
    );
    Ok(report(account, imported.len(), deleted))
}

fn partial_sync(
    config: &crate::config::Config,
    crm: &Connection,
    client: &ApiClient,
    source: &GmailSource<'_>,
    cursor: &str,
    progress: &mut ProgressTracker,
) -> Result<Option<SyncReport>> {
    let account = source.account;
    let run_at = Utc::now().to_rfc3339();
    let mut page_token: Option<String> = None;
    let mut changed = HashSet::new();
    let mut deleted = HashSet::new();
    progress.stage(
        format!("Checking Gmail changes for {account}"),
        source.stage.current,
        source.stage.total,
        1,
        false,
        "history scan",
    );
    let current_cursor = loop {
        let mut path = format!("history?startHistoryId={cursor}&maxResults=500");
        if let Some(token) = &page_token {
            path.push_str("&pageToken=");
            path.push_str(token);
        }
        let page: HistoryPage = match client.get(&path)? {
            ApiResponse::NotFound => return Ok(None),
            ApiResponse::Data(page) => page,
        };
        let current_cursor = page.history_id;
        for history in page.history {
            for item in history.messages_deleted {
                deleted.insert(item.message.id);
            }
            for item in history
                .messages_added
                .into_iter()
                .chain(history.labels_added)
                .chain(history.labels_removed)
            {
                changed.insert(item.message.id);
            }
        }
        page_token = page.next_page_token;
        if page_token.is_none() {
            break current_cursor;
        }
    };
    for id in &deleted {
        delete_message(crm, source.id, id)?;
    }
    let changed_count = changed.difference(&deleted).count() as u64;
    let mut processed = 0_u64;
    let mut imported = 0;
    progress.stage(
        format!("Reading new and changed emails from {account}"),
        source.stage.current,
        source.stage.total,
        changed_count,
        false,
        "emails",
    );
    for id in changed.difference(&deleted) {
        if import_message(config, crm, client, source.id, id, &run_at)?.is_some() {
            imported += 1;
        }
        processed += 1;
        progress.progress(
            format!("Reading new and changed emails from {account}"),
            processed,
            changed_count,
            false,
            "emails",
        );
    }
    crm.execute(
        "UPDATE sources SET cursor=?2, status='ok', last_sync_at=CURRENT_TIMESTAMP WHERE id=?1",
        params![source.id, current_cursor],
    )?;
    progress.finish_stage(
        format!("Read new and changed emails from {account}"),
        processed,
        changed_count,
        false,
        "emails",
    );
    Ok(Some(report(account, imported, deleted.len())))
}

fn import_message(
    config: &crate::config::Config,
    crm: &Connection,
    client: &ApiClient,
    source_id: &str,
    id: &str,
    run_at: &str,
) -> Result<Option<u64>> {
    let message: GmailMessage = match client.get(&format!("messages/{id}?format=FULL"))? {
        ApiResponse::NotFound => {
            delete_message(crm, source_id, id)?;
            return Ok(None);
        }
        ApiResponse::Data(message) => message,
    };
    if message
        .label_ids
        .iter()
        .any(|label| label == "SPAM" || label == "TRASH")
    {
        delete_message(crm, source_id, id)?;
        return Ok(None);
    }
    let from = header(&message, "From").unwrap_or_default();
    let to = header(&message, "To").unwrap_or_default();
    let cc = header(&message, "Cc").unwrap_or_default();
    let subject = header(&message, "Subject");
    let self_addresses: HashSet<_> = config
        .self_identity
        .emails
        .iter()
        .chain(config.gmail.accounts.iter())
        .map(|value| value.to_lowercase())
        .collect();
    let outgoing = addresses(&from)
        .iter()
        .any(|email| self_addresses.contains(email));
    let automated = is_automated(&message, &from);
    let body = (!automated).then(|| message_text(&message)).flatten();
    let millis = message
        .internal_date
        .parse::<i64>()
        .map_err(|_| CrmError::Network(format!("Gmail message {id} has invalid internalDate")))?;
    let occurred_at = Utc
        .timestamp_millis_opt(millis)
        .single()
        .ok_or_else(|| {
            CrmError::Network(format!("Gmail message {id} has out-of-range internalDate"))
        })?
        .to_rfc3339();
    let interaction_id = upsert_interaction(
        crm,
        source_id,
        &message.id,
        Some(&message.thread_id),
        "gmail",
        "email",
        &occurred_at,
        Some(if outgoing { "outgoing" } else { "incoming" }),
        subject.as_deref(),
        body.as_deref(),
        &serde_json::json!({"classification": if automated { "automated" } else { "pending" }, "labels": message.label_ids}),
        run_at,
    )?;
    for email in addresses(&format!("{from},{to},{cc}")) {
        if !self_addresses.contains(&email) {
            add_participant(
                crm,
                &interaction_id,
                &email,
                None,
                if outgoing {
                    "recipient"
                } else {
                    "sender_or_recipient"
                },
            )?;
        }
    }
    import_attachments(crm, &interaction_id, &message.id, &message.payload)?;
    Ok(message.history_id.parse().ok())
}

fn delete_message(crm: &Connection, source_id: &str, native_id: &str) -> Result<()> {
    crm.execute("UPDATE interactions SET body=NULL, subject=NULL, deleted_at=CURRENT_TIMESTAMP WHERE source_id=?1 AND native_id=?2", params![source_id, native_id])?;
    crm.execute(
        "INSERT OR IGNORE INTO tombstones(source_id, native_id) VALUES (?1, ?2)",
        params![source_id, native_id],
    )?;
    Ok(())
}

fn report(account: &str, imported: usize, deleted: usize) -> SyncReport {
    SyncReport {
        source: format!("gmail:{account}"),
        imported,
        deleted,
        schema_fingerprint: "gmail-api-v1".into(),
    }
}

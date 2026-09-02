use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};

use super::SyncReport;
use super::gmail_backfill;
use super::gmail_import;
use super::gmail_store;
use crate::error::{CrmError, Result};
use crate::gmail::{ApiClient, ApiResponse, Credentials, HistoryPage, Profile};
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
        let stage = ProgressStage {
            current: (index + 1) as u64,
            total: account_count as u64,
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
            "Finished {account}: {} people-focused emails kept, {} excluded, {} deleted",
            report.imported, report.excluded, report.deleted
        ));
        reports.push(SyncReport {
            source: format!("gmail:{account}"),
            imported: report.imported,
            deleted: report.deleted,
            schema_fingerprint: "gmail-api-v1-people-focused".into(),
        });
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
) -> Result<AccountReport> {
    let source_id = format!("gmail:{account}");
    let client = ApiClient::for_account(credentials, account)?;
    crm.execute(
        "INSERT INTO sources(id, kind, account, schema_fingerprint, status)
         VALUES (?1, 'gmail', ?2, 'gmail-api-v1-people-focused', 'syncing')
         ON CONFLICT(id) DO UPDATE SET account=excluded.account,
         schema_fingerprint=excluded.schema_fingerprint, status='syncing', error=NULL",
        params![source_id, account],
    )?;
    let source = GmailSource {
        id: &source_id,
        account,
        stage,
    };
    let self_addresses = self_addresses(config);
    let known_emails = gmail_store::known_emails(crm)?;
    let pruned = gmail_store::prune_legacy_noise(crm, source.id)?;
    if pruned > 0 {
        progress.event(format!(
            "Excluded {pruned} legacy non-person Gmail interactions"
        ));
    }
    gmail_backfill::seed(crm, source.id, &self_addresses)?;
    sync_history(crm, &client, &source, progress)?;
    let scan = gmail_backfill::scan(crm, &client, source.id, account, stage, progress)?;
    if scan.pages > 0 {
        progress.event(format!(
            "Searched {} people scopes and found {} message references",
            scan.pages, scan.messages_found
        ));
    }
    let import_context = gmail_import::ImportContext {
        source_id: source.id,
        account,
        self_addresses: &self_addresses,
        known_emails: &known_emails,
        stage,
    };
    let processed = gmail_import::process_queue(crm, &client, &import_context, progress)?;
    crm.execute(
        "UPDATE sources SET status='ok', error=NULL, last_sync_at=CURRENT_TIMESTAMP,
         last_reconcile_at=CURRENT_TIMESTAMP WHERE id=?1",
        [source.id],
    )?;
    Ok(AccountReport {
        imported: processed.imported,
        excluded: processed.skipped + pruned,
        deleted: processed.deleted,
    })
}

fn self_addresses(config: &crate::config::Config) -> HashSet<String> {
    config
        .self_identity
        .emails
        .iter()
        .chain(config.gmail.accounts.iter())
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

fn sync_history(
    crm: &Connection,
    client: &ApiClient,
    source: &GmailSource<'_>,
    progress: &mut ProgressTracker,
) -> Result<()> {
    let cursor: Option<String> = crm
        .query_row(
            "SELECT cursor FROM sources WHERE id=?1",
            [source.id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let Some(cursor) = cursor.filter(|value| !value.is_empty() && value != "0") else {
        set_current_cursor(crm, client, source.id)?;
        progress.event(format!(
            "Established the incremental Gmail checkpoint for {}",
            source.account
        ));
        return Ok(());
    };
    progress.stage(
        format!("Checking Gmail changes for {}", source.account),
        source.stage.current,
        source.stage.total,
        1,
        false,
        "history scan",
    );
    let mut page_token: Option<String> = None;
    let mut changed = HashSet::new();
    let mut deleted = HashSet::new();
    let current_cursor = loop {
        let mut pairs = url::form_urlencoded::Serializer::new(String::new());
        pairs
            .append_pair("startHistoryId", &cursor)
            .append_pair("maxResults", "500");
        if let Some(token) = &page_token {
            pairs.append_pair("pageToken", token);
        }
        let path = format!("history?{}", pairs.finish());
        let page: HistoryPage = match client.get(&path)? {
            ApiResponse::NotFound => {
                gmail_backfill::reset(crm, source.id)?;
                set_current_cursor(crm, client, source.id)?;
                progress.event(format!(
                    "Gmail history expired for {}; restarting the resumable people backfill",
                    source.account
                ));
                return Ok(());
            }
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
    let transaction = crm.unchecked_transaction()?;
    for id in &deleted {
        gmail_store::discard_message(&transaction, source.id, id)?;
        gmail_backfill::mark(
            &transaction,
            source.id,
            id,
            "deleted",
            Some("gmail_deleted"),
        )?;
    }
    for id in changed.difference(&deleted) {
        gmail_backfill::enqueue_changed(&transaction, source.id, id)?;
    }
    transaction.execute(
        "UPDATE sources SET cursor=?2 WHERE id=?1",
        params![source.id, current_cursor],
    )?;
    transaction.commit()?;
    progress.finish_stage(
        format!(
            "Queued {} changed Gmail messages from {}",
            changed.difference(&deleted).count(),
            source.account
        ),
        1,
        1,
        false,
        "history scan",
    );
    Ok(())
}

fn set_current_cursor(crm: &Connection, client: &ApiClient, source_id: &str) -> Result<()> {
    let profile: Profile = match client.get("profile")? {
        ApiResponse::Data(profile) => profile,
        ApiResponse::NotFound => {
            return Err(CrmError::Network("Gmail profile disappeared".into()));
        }
    };
    if profile.history_id.is_empty() {
        return Err(CrmError::Network(
            "Gmail profile did not provide a history checkpoint".into(),
        ));
    }
    crm.execute(
        "UPDATE sources SET cursor=?2 WHERE id=?1",
        params![source_id, profile.history_id],
    )?;
    Ok(())
}

#[derive(Debug, Default)]
struct AccountReport {
    imported: usize,
    excluded: usize,
    deleted: usize,
}

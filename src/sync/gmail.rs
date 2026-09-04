use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, params};

use super::SyncReport;
use super::gmail_backfill;
use super::gmail_import;
use super::gmail_store;
use crate::error::{CrmError, Result};
use crate::gmail::{ApiClient, ApiResponse, Credentials, HistoryPage, Profile};
use crate::progress::{ProgressStage, ProgressTracker};

const POLICY_FINGERPRINT: &str = "gmail-api-v3-domain-ignore";

struct GmailSource<'a> {
    id: &'a str,
    account: &'a str,
    stage: ProgressStage,
}

struct GmailPolicy<'a> {
    ignored_domains: &'a [String],
    fingerprint: String,
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
    let self_addresses = self_addresses(config, crm)?;
    let policy = GmailPolicy {
        ignored_domains: &config.gmail.ignored_domains,
        fingerprint: policy_fingerprint(config),
    };
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
        let report = match sync_account(
            crm,
            &credentials,
            account,
            &self_addresses,
            &policy,
            progress,
            stage,
        ) {
            Ok(report) => report,
            Err(error) => {
                mark_failed(crm, account, &policy.fingerprint, &error.to_string());
                return Err(error);
            }
        };
        progress.event(format!(
            "Finished {account}: {} people-focused emails kept, {} excluded, {} deleted",
            report.imported, report.excluded, report.deleted
        ));
        reports.push(SyncReport {
            source: format!("gmail:{account}"),
            imported: report.imported,
            deleted: report.deleted,
            schema_fingerprint: policy.fingerprint.clone(),
            changed: report.imported > 0 || report.deleted > 0,
        });
    }
    Ok(reports)
}

fn mark_failed(crm: &Connection, account: &str, policy_fingerprint: &str, error: &str) {
    let source_id = format!("gmail:{account}");
    let Ok(transaction) = crate::db::immediate_transaction(crm) else {
        return;
    };
    if transaction
        .execute(
            "INSERT INTO sources(id, kind, account, schema_fingerprint, status, error)
             VALUES (?1, 'gmail', ?2, ?4, 'failed', ?3)
             ON CONFLICT(id) DO UPDATE SET status='failed', error=excluded.error",
            params![source_id, account, error, policy_fingerprint],
        )
        .is_ok()
    {
        let _ = transaction.commit();
    }
}

fn sync_account(
    crm: &Connection,
    credentials: &Credentials,
    account: &str,
    self_addresses: &HashSet<String>,
    policy: &GmailPolicy<'_>,
    progress: &mut ProgressTracker,
    stage: ProgressStage,
) -> Result<AccountReport> {
    let source_id = format!("gmail:{account}");
    let client = ApiClient::for_account(credentials, account)?;
    let previous_fingerprint: Option<String> = crm
        .query_row(
            "SELECT schema_fingerprint FROM sources WHERE id=?1",
            [&source_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    if previous_fingerprint
        .as_deref()
        .is_some_and(|fingerprint| fingerprint != policy.fingerprint)
    {
        gmail_backfill::reset(crm, &source_id)?;
        progress.event(format!(
            "Requeued Gmail history for {account} after the Gmail policy changed"
        ));
    }
    crm.execute(
        "INSERT INTO sources(id, kind, account, schema_fingerprint, status)
         VALUES (?1, 'gmail', ?2, ?3, 'syncing')
         ON CONFLICT(id) DO UPDATE SET account=excluded.account,
         schema_fingerprint=excluded.schema_fingerprint, status='syncing', error=NULL",
        params![source_id, account, policy.fingerprint],
    )?;
    let source = GmailSource {
        id: &source_id,
        account,
        stage,
    };
    let known_emails = gmail_store::known_emails(crm)?;
    let pruned = gmail_store::prune_legacy_noise(crm, source.id)?;
    if pruned > 0 {
        progress.event(format!(
            "Excluded {pruned} legacy non-person Gmail interactions"
        ));
    }
    let ignored = gmail_store::prune_ignored_domains(crm, source.id, policy.ignored_domains)?;
    if ignored > 0 {
        progress.event(format!(
            "Excluded {ignored} Gmail interactions from ignored domains"
        ));
    }
    gmail_backfill::seed(crm, source.id, self_addresses, policy.ignored_domains)?;
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
        self_addresses,
        known_emails: &known_emails,
        ignored_domains: policy.ignored_domains,
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
        excluded: processed.skipped + pruned + ignored,
        deleted: processed.deleted + pruned + ignored,
    })
}

fn policy_fingerprint(config: &crate::config::Config) -> String {
    let mut domains = config
        .gmail
        .ignored_domains
        .iter()
        .map(|domain| domain.trim().to_ascii_lowercase())
        .filter(|domain| !domain.is_empty())
        .collect::<Vec<_>>();
    domains.sort();
    domains.dedup();
    format!("{POLICY_FINGERPRINT}:{}", domains.join(","))
}

fn self_addresses(config: &crate::config::Config, crm: &Connection) -> Result<HashSet<String>> {
    let mut addresses = crate::repository::active_self_emails(crm)?;
    if addresses.is_empty() {
        return Err(CrmError::InvalidConfig(
            "the linked iCloud self contact has no active email addresses; update the contact and run `crm run contacts`"
                .into(),
        ));
    }
    addresses.extend(
        config
            .gmail
            .accounts
            .iter()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty()),
    );
    Ok(addresses)
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
    let transaction = crate::db::immediate_transaction(crm)?;
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

#[cfg(test)]
#[path = "gmail_tests.rs"]
mod tests;

#[derive(Debug, Default)]
struct AccountReport {
    imported: usize,
    excluded: usize,
    deleted: usize,
}

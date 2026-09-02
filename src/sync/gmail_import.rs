use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use super::gmail_backfill;
use super::gmail_message::{
    Mailbox, addresses, has_bulk_signals, header, is_automated, is_probable_person_email, mailboxes,
};
use super::gmail_store;
use crate::error::Result;
use crate::gmail::{ApiClient, ApiResponse, GmailMessage};
use crate::progress::{ProgressStage, ProgressTracker};

const MAX_DIRECT_RECIPIENTS: usize = 5;

#[derive(Debug, Default)]
pub(super) struct ProcessReport {
    pub imported: usize,
    pub skipped: usize,
    pub deleted: usize,
}

pub(super) struct ImportContext<'a> {
    pub source_id: &'a str,
    pub account: &'a str,
    pub self_addresses: &'a HashSet<String>,
    pub known_emails: &'a HashSet<String>,
    pub stage: ProgressStage,
}

enum Decision {
    Accept {
        outgoing: bool,
        candidate_eligible: bool,
        participants: Vec<Mailbox>,
    },
    Skip(&'static str),
}

enum PreparedMessage {
    Accept {
        message: Box<GmailMessage>,
        outgoing: bool,
        candidate_eligible: bool,
        participants: Vec<Mailbox>,
    },
    Skip(&'static str),
    Deleted,
}

pub(super) fn process_queue(
    crm: &Connection,
    client: &ApiClient,
    context: &ImportContext<'_>,
    progress: &mut ProgressTracker,
) -> Result<ProcessReport> {
    let (queued, queue_total) = gmail_backfill::queued(crm, context.source_id)?;
    let batch_total = queued.len() as u64;
    progress.stage(
        format!(
            "Processing people-focused emails from {} ({queue_total} queued)",
            context.account
        ),
        context.stage.current,
        context.stage.total,
        batch_total,
        false,
        "emails",
    );
    let mut report = ProcessReport::default();
    for (index, queued) in queued.into_iter().enumerate() {
        let prepared = prepare_message(
            client,
            &queued.id,
            queued.known_scope,
            context.self_addresses,
            context.known_emails,
        )?;
        let transaction = crate::db::immediate_transaction(crm)?;
        match prepared {
            PreparedMessage::Deleted => {
                if gmail_store::discard_message(&transaction, context.source_id, &queued.id)? {
                    report.deleted += 1;
                }
                gmail_backfill::mark(
                    &transaction,
                    context.source_id,
                    &queued.id,
                    "deleted",
                    Some("not_found"),
                )?;
            }
            PreparedMessage::Skip(reason) => {
                if gmail_store::discard_message(&transaction, context.source_id, &queued.id)? {
                    report.deleted += 1;
                }
                gmail_backfill::mark(
                    &transaction,
                    context.source_id,
                    &queued.id,
                    "skipped",
                    Some(reason),
                )?;
                report.skipped += 1;
            }
            PreparedMessage::Accept {
                message,
                outgoing,
                candidate_eligible,
                participants,
            } => {
                gmail_store::persist_message(
                    &transaction,
                    context.source_id,
                    &message,
                    outgoing,
                    candidate_eligible,
                    &participants,
                )?;
                gmail_backfill::mark(
                    &transaction,
                    context.source_id,
                    &queued.id,
                    "accepted",
                    None,
                )?;
                report.imported += 1;
            }
        }
        transaction.commit()?;
        let processed = (index + 1) as u64;
        progress.progress(
            format!(
                "Processing people-focused emails from {}: {} kept, {} excluded",
                context.account, report.imported, report.skipped
            ),
            processed,
            batch_total,
            false,
            "emails",
        );
    }
    progress.finish_stage(
        format!(
            "Processed Gmail batch for {}: {} kept, {} excluded",
            context.account, report.imported, report.skipped
        ),
        batch_total,
        batch_total,
        false,
        "emails",
    );
    Ok(report)
}

fn prepare_message(
    client: &ApiClient,
    id: &str,
    known_scope: bool,
    self_addresses: &HashSet<String>,
    known_emails: &HashSet<String>,
) -> Result<PreparedMessage> {
    let format = if known_scope { "FULL" } else { "METADATA" };
    let message: GmailMessage = match client.get(&format!("messages/{id}?format={format}"))? {
        ApiResponse::NotFound => return Ok(PreparedMessage::Deleted),
        ApiResponse::Data(message) => message,
    };
    match classify(&message, self_addresses, known_emails) {
        Decision::Skip(reason) => Ok(PreparedMessage::Skip(reason)),
        Decision::Accept {
            outgoing,
            candidate_eligible,
            participants,
        } => {
            let message = if known_scope {
                message
            } else {
                match client.get(&format!("messages/{id}?format=FULL"))? {
                    ApiResponse::NotFound => return Ok(PreparedMessage::Deleted),
                    ApiResponse::Data(message) => message,
                }
            };
            Ok(PreparedMessage::Accept {
                message: Box::new(message),
                outgoing,
                candidate_eligible,
                participants,
            })
        }
    }
}

fn classify(
    message: &GmailMessage,
    self_addresses: &HashSet<String>,
    known_emails: &HashSet<String>,
) -> Decision {
    let from = header(message, "From").unwrap_or_default();
    let from_addresses: HashSet<_> = addresses(&from).into_iter().collect();
    let outgoing = from_addresses
        .iter()
        .any(|email| self_addresses.contains(email));
    if has_bulk_signals(message) {
        return Decision::Skip("automated_or_bulk");
    }
    let mut participants = HashMap::<String, Mailbox>::new();
    for mailbox in [
        mailboxes(&from),
        mailboxes(&header(message, "To").unwrap_or_default()),
        mailboxes(&header(message, "Cc").unwrap_or_default()),
        mailboxes(&header(message, "Bcc").unwrap_or_default()),
    ]
    .into_iter()
    .flatten()
    {
        if !self_addresses.contains(&mailbox.email) {
            participants
                .entry(mailbox.email.clone())
                .and_modify(|current| {
                    if current.name.is_none() {
                        current.name.clone_from(&mailbox.name);
                    }
                })
                .or_insert(mailbox);
        }
    }
    if participants.is_empty() {
        return Decision::Skip("no_external_person");
    }
    let has_known_person = participants
        .keys()
        .any(|email| known_emails.contains(email));
    if !outgoing && !has_known_person && is_automated(message, &from) {
        return Decision::Skip("automated_or_bulk");
    }
    let candidate_eligible = outgoing
        && participants.len() <= MAX_DIRECT_RECIPIENTS
        && participants
            .keys()
            .any(|email| !known_emails.contains(email) && is_probable_person_email(email));
    if !has_known_person && !candidate_eligible {
        return Decision::Skip(if outgoing {
            "not_a_direct_person"
        } else {
            "incoming_unknown"
        });
    }
    let mut participants: Vec<_> = participants.into_values().collect();
    participants.sort_by(|left, right| left.email.cmp(&right.email));
    Decision::Accept {
        outgoing,
        candidate_eligible,
        participants,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gmail::MessagePart;
    use crate::gmail::api::{Header, MessageBody};

    fn message(from: &str, to: &str, labels: Vec<&str>) -> GmailMessage {
        GmailMessage {
            id: "message".into(),
            thread_id: "thread".into(),
            label_ids: labels.into_iter().map(str::to_owned).collect(),
            internal_date: "0".into(),
            payload: MessagePart {
                mime_type: "text/plain".into(),
                filename: String::new(),
                headers: vec![
                    Header {
                        name: "From".into(),
                        value: from.into(),
                    },
                    Header {
                        name: "To".into(),
                        value: to.into(),
                    },
                ],
                body: MessageBody::default(),
                parts: vec![],
            },
        }
    }

    fn set(values: &[&str]) -> HashSet<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn accepts_known_people_and_direct_outgoing_discovery() {
        let known = classify(
            &message("Alex <alex@example.com>", "me@example.com", vec![]),
            &set(&["me@example.com"]),
            &set(&["alex@example.com"]),
        );
        assert!(matches!(
            known,
            Decision::Accept {
                candidate_eligible: false,
                ..
            }
        ));

        let discovery = classify(
            &message("me@example.com", "Jane <jane@example.com>", vec![]),
            &set(&["me@example.com"]),
            &HashSet::new(),
        );
        assert!(matches!(
            discovery,
            Decision::Accept {
                candidate_eligible: true,
                ..
            }
        ));
    }

    #[test]
    fn rejects_unknown_incoming_and_marketing_to_known_people() {
        let unknown = classify(
            &message("stranger@example.com", "me@example.com", vec![]),
            &set(&["me@example.com"]),
            &HashSet::new(),
        );
        assert!(matches!(unknown, Decision::Skip("incoming_unknown")));

        let marketing = classify(
            &message(
                "Alex <alex@example.com>",
                "me@example.com",
                vec!["CATEGORY_PROMOTIONS"],
            ),
            &set(&["me@example.com"]),
            &set(&["alex@example.com"]),
        );
        assert!(matches!(marketing, Decision::Skip("automated_or_bulk")));
    }

    #[test]
    fn explicit_icloud_identity_wins_over_a_shared_mailbox_name() {
        let known = classify(
            &message("Support <support@example.com>", "me@example.com", vec![]),
            &set(&["me@example.com"]),
            &set(&["support@example.com"]),
        );
        assert!(matches!(known, Decision::Accept { .. }));
    }

    #[test]
    fn rejects_mass_outgoing_mail_without_a_known_person() {
        let recipients = (0..6)
            .map(|index| format!("person{index}@example.com"))
            .collect::<Vec<_>>()
            .join(",");
        let decision = classify(
            &message("me@example.com", &recipients, vec![]),
            &set(&["me@example.com"]),
            &HashSet::new(),
        );
        assert!(matches!(decision, Decision::Skip("not_a_direct_person")));
    }
}

use base64::Engine;
use rusqlite::{Connection, params};

use crate::error::Result;
use crate::gmail::{GmailMessage, MessagePart};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Mailbox {
    pub email: String,
    pub name: Option<String>,
}

pub(super) fn header(message: &GmailMessage, name: &str) -> Option<String> {
    message
        .payload
        .headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| header.value.clone())
}

pub(super) fn addresses(value: &str) -> Vec<String> {
    mailboxes(value)
        .into_iter()
        .map(|mailbox| mailbox.email)
        .collect()
}

pub(super) fn mailboxes(value: &str) -> Vec<Mailbox> {
    split_address_list(value)
        .into_iter()
        .filter_map(|part| {
            let part = part.trim();
            let (name, email) = match (part.find('<'), part.rfind('>')) {
                (Some(start), Some(end)) if start < end => {
                    let name = part[..start].trim().trim_matches('"').trim().to_owned();
                    (
                        (!name.is_empty()).then_some(name),
                        part[start + 1..end].trim(),
                    )
                }
                _ => (None, part),
            };
            let email = email.trim_matches('"').trim().to_lowercase();
            (email.contains('@') && !email.contains(char::is_whitespace))
                .then_some(Mailbox { email, name })
        })
        .collect()
}

fn split_address_list(value: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut angle_depth = 0_u8;
    for (index, character) in value.char_indices() {
        match character {
            '"' => quoted = !quoted,
            '<' if !quoted => angle_depth = angle_depth.saturating_add(1),
            '>' if !quoted => angle_depth = angle_depth.saturating_sub(1),
            ',' if !quoted && angle_depth == 0 => {
                result.push(&value[start..index]);
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    result.push(&value[start..]);
    result
}

pub(super) fn is_automated(message: &GmailMessage, from: &str) -> bool {
    has_bulk_signals(message)
        || addresses(from)
            .iter()
            .any(|email| !is_probable_person_email(email))
}

pub(super) fn has_bulk_signals(message: &GmailMessage) -> bool {
    message.label_ids.iter().any(|label| {
        matches!(
            label.as_str(),
            "SPAM"
                | "TRASH"
                | "DRAFT"
                | "CATEGORY_PROMOTIONS"
                | "CATEGORY_SOCIAL"
                | "CATEGORY_FORUMS"
        )
    }) || [
        "List-Id",
        "List-Unsubscribe",
        "List-Unsubscribe-Post",
        "X-Auto-Response-Suppress",
    ]
    .iter()
    .any(|name| header(message, name).is_some())
        || header(message, "Auto-Submitted").is_some_and(|value| !value.eq_ignore_ascii_case("no"))
        || header(message, "Precedence")
            .is_some_and(|value| matches!(value.to_lowercase().as_str(), "bulk" | "list" | "junk"))
}

pub(crate) fn is_probable_person_email(email: &str) -> bool {
    let local = email
        .trim()
        .to_ascii_lowercase()
        .split_once('@')
        .map(|(local, _)| local.to_owned())
        .unwrap_or_default();
    if local.is_empty() {
        return false;
    }
    let compact: String = local
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect();
    ![
        "noreply",
        "donotreply",
        "mailerdaemon",
        "postmaster",
        "newsletter",
        "notifications",
        "notification",
        "marketing",
        "receipts",
        "receipt",
        "billing",
        "invoices",
        "invoice",
        "orders",
        "order",
        "updates",
        "update",
        "alerts",
        "alert",
        "info",
        "support",
        "help",
        "sales",
        "admin",
        "contact",
        "team",
        "careers",
        "jobs",
    ]
    .iter()
    .any(|pattern| compact == *pattern || compact.starts_with(pattern))
}

pub(super) fn message_text(message: &GmailMessage) -> Option<String> {
    part_text(&message.payload, "text/plain")
        .or_else(|| part_text(&message.payload, "text/html").map(|html| strip_html(&html)))
}

fn part_text(part: &MessagePart, mime_type: &str) -> Option<String> {
    if part.mime_type == mime_type {
        let data = part.body.data.as_ref()?;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(data)
            .ok()?;
        return String::from_utf8(bytes).ok();
    }
    part.parts
        .iter()
        .find_map(|child| part_text(child, mime_type))
}

fn strip_html(html: &str) -> String {
    let mut text = String::new();
    let mut inside = false;
    for character in html.chars() {
        match character {
            '<' => inside = true,
            '>' => {
                inside = false;
                text.push(' ');
            }
            _ if !inside => text.push(character),
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn import_attachments(
    crm: &Connection,
    interaction_id: &str,
    message_id: &str,
    part: &MessagePart,
) -> Result<()> {
    if !part.filename.is_empty() || part.body.attachment_id.is_some() {
        let native = part.body.attachment_id.as_deref().unwrap_or(&part.filename);
        crm.execute(
            "INSERT OR REPLACE INTO attachments(id, interaction_id, filename, mime_type, size_bytes, source_reference) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![format!("gmail:{message_id}:{native}"), interaction_id, part.filename, part.mime_type, part.body.size, native],
        )?;
    }
    for child in &part.parts {
        import_attachments(crm, interaction_id, message_id, child)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gmail::api::{Header, MessageBody};

    fn message(headers: Vec<Header>) -> GmailMessage {
        GmailMessage {
            id: "1".into(),
            thread_id: "t".into(),
            label_ids: vec![],
            internal_date: "0".into(),
            payload: MessagePart {
                mime_type: "text/plain".into(),
                filename: String::new(),
                headers,
                body: MessageBody::default(),
                parts: vec![],
            },
        }
    }

    #[test]
    fn rejects_bulk_email() {
        assert!(is_automated(
            &message(vec![Header {
                name: "Precedence".into(),
                value: "bulk".into()
            }]),
            "person@example.com"
        ));
    }

    #[test]
    fn extracts_addresses() {
        assert_eq!(
            addresses("Alice <ALICE@example.com>, bob@example.com"),
            ["alice@example.com", "bob@example.com"]
        );
    }

    #[test]
    fn extracts_names_with_quoted_commas() {
        assert_eq!(
            mailboxes("\"Doe, Jane\" <jane@example.com>, Alex <alex@example.com>"),
            [
                Mailbox {
                    email: "jane@example.com".into(),
                    name: Some("Doe, Jane".into())
                },
                Mailbox {
                    email: "alex@example.com".into(),
                    name: Some("Alex".into())
                }
            ]
        );
    }

    #[test]
    fn rejects_marketing_categories_and_shared_mailboxes() {
        let mut promotional = message(vec![]);
        promotional.label_ids.push("CATEGORY_PROMOTIONS".into());
        assert!(is_automated(&promotional, "Alice <alice@example.com>"));
        assert!(is_automated(
            &message(vec![]),
            "Support <support@example.com>"
        ));
        assert!(is_probable_person_email("alice.smith@example.com"));
    }
}

use base64::Engine;
use rusqlite::{Connection, params};

use crate::error::Result;
use crate::gmail::{GmailMessage, MessagePart};

pub(super) fn header(message: &GmailMessage, name: &str) -> Option<String> {
    message
        .payload
        .headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| header.value.clone())
}

pub(super) fn addresses(value: &str) -> Vec<String> {
    value
        .split(',')
        .filter_map(|part| {
            let part = part.trim();
            let email = part
                .split_once('<')
                .and_then(|(_, rest)| rest.split_once('>').map(|(email, _)| email))
                .unwrap_or(part)
                .trim()
                .to_lowercase();
            (email.contains('@') && !email.contains(' ')).then_some(email)
        })
        .collect()
}

pub(super) fn is_automated(message: &GmailMessage, from: &str) -> bool {
    let from = from.to_lowercase();
    from.contains("no-reply")
        || from.contains("noreply")
        || ["List-Id", "X-Auto-Response-Suppress"]
            .iter()
            .any(|name| header(message, name).is_some())
        || header(message, "Auto-Submitted").is_some_and(|value| !value.eq_ignore_ascii_case("no"))
        || header(message, "Precedence")
            .is_some_and(|value| matches!(value.to_lowercase().as_str(), "bulk" | "list" | "junk"))
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
            history_id: "1".into(),
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
}

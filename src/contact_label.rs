use rusqlite::{Connection, OptionalExtension};

use crate::contact_publish::apple::AppleContact;
use crate::error::Result;

pub(crate) fn format(name: Option<&str>, phone: Option<&str>, email: Option<&str>) -> String {
    let name = nonempty(name);
    let phone = nonempty(phone);
    let email = nonempty(email);
    let base = name.or(phone).or(email).unwrap_or("Unnamed contact");
    let details = [phone, email]
        .into_iter()
        .flatten()
        .filter(|value| !value.eq_ignore_ascii_case(base))
        .collect::<Vec<_>>();
    if details.is_empty() {
        base.to_owned()
    } else {
        format!("{base} ({})", details.join(", "))
    }
}

pub(crate) fn apple(contact: &AppleContact) -> String {
    format(
        Some(&apple_name(contact)),
        contact.phones.first().map(|item| item.value.as_str()),
        contact.emails.first().map(|item| item.value.as_str()),
    )
}

pub(crate) fn person(connection: &Connection, person_id: &str, name: &str) -> Result<String> {
    Ok(format(
        Some(name),
        first_identity(connection, person_id, "phone")?.as_deref(),
        first_identity(connection, person_id, "email")?.as_deref(),
    ))
}

pub(crate) fn apple_name(contact: &AppleContact) -> String {
    let name = format!(
        "{} {}",
        contact.given_name.trim(),
        contact.family_name.trim()
    )
    .trim()
    .to_owned();
    if !name.is_empty() {
        name
    } else if !contact.organization.trim().is_empty() {
        contact.organization.trim().into()
    } else {
        "Unnamed contact".into()
    }
}

fn first_identity(connection: &Connection, person_id: &str, kind: &str) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT value FROM identities
             WHERE person_id=?1 AND kind=?2 AND active=1 ORDER BY value LIMIT 1",
            [person_id, kind],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_phone_then_email_after_name() {
        assert_eq!(
            format(
                Some("Scott Hickmann"),
                Some("+1234567890"),
                Some("scott@example.com")
            ),
            "Scott Hickmann (+1234567890, scott@example.com)"
        );
    }

    #[test]
    fn does_not_repeat_an_identity_used_as_the_name() {
        assert_eq!(format(None, Some("+1234567890"), None), "+1234567890");
    }
}

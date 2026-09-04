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
        &[],
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
        &[],
    );
    assert!(matches!(
        discovery,
        Decision::Accept {
            candidate_eligible: true,
            ..
        }
    ));

    let alias = classify(
        &message("alias@example.com", "Jane <jane@example.com>", vec![]),
        &set(&["primary@example.com", "alias@example.com"]),
        &HashSet::new(),
        &[],
    );
    assert!(matches!(
        alias,
        Decision::Accept {
            outgoing: true,
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
        &[],
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
        &[],
    );
    assert!(matches!(marketing, Decision::Skip("automated_or_bulk")));
}

#[test]
fn explicit_icloud_identity_wins_over_a_shared_mailbox_name() {
    let known = classify(
        &message("Support <support@example.com>", "me@example.com", vec![]),
        &set(&["me@example.com"]),
        &set(&["support@example.com"]),
        &[],
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
        &[],
    );
    assert!(matches!(decision, Decision::Skip("not_a_direct_person")));
}

#[test]
fn rejects_messages_with_any_ignored_external_domain() {
    let ignored = vec!["lists.stanford.edu".into()];
    let decision = classify(
        &message(
            "Alex <alex@stanford.edu>",
            "group@lists.stanford.edu, me@example.com",
            vec![],
        ),
        &set(&["me@example.com"]),
        &set(&["alex@stanford.edu"]),
        &ignored,
    );

    assert!(matches!(decision, Decision::Skip("ignored_domain")));
}

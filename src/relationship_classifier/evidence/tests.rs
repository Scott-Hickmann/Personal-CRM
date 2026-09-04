use super::*;

fn candidate() -> Candidate {
    Candidate {
        id: "a:b".into(),
        source_id: "a".into(),
        target_id: "b".into(),
        source_name: "Alex".into(),
        target_name: "Blair".into(),
        structure_revision: 1,
    }
}

fn message() -> EvidenceMessage {
    EvidenceMessage {
        id: "message".into(),
        context_key: "source\0thread".into(),
        occurred_at: "2026-01-01".into(),
        author_id: None,
        author_name: "CRM owner".into(),
        direction: Some("outgoing".into()),
        pair_explicit: false,
        subject: None,
        body: "Let's meet soon".into(),
        member_count: 3,
        bucket: 0,
    }
}

#[test]
fn owner_message_requires_both_people_as_explicit_participants() {
    let mut item = message();
    assert_eq!(priority(&item, &candidate()), None);
    item.pair_explicit = true;
    assert_eq!(priority(&item, &candidate()), Some(3));
}

#[test]
fn direct_address_ranks_ahead_of_generic_pair_authorship() {
    let mut item = message();
    item.author_id = Some("a".into());
    assert_eq!(priority(&item, &candidate()), Some(2));
    item.body = "Blair, can you review this?".into();
    assert_eq!(priority(&item, &candidate()), Some(0));
}

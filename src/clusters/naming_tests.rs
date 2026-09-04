use super::{input::Evidence, naming, tests::fixture};

#[test]
fn selected_name_always_keeps_its_evidence_in_the_displayed_shortlist() {
    let mut input = fixture();
    for i in 0..6 {
        input.evidence.push(Evidence {
            kind: "tag".into(),
            label: format!("A generic {i}"),
            source: format!("tag-{i}"),
            members: input.people.iter().map(|p| p.0.clone()).collect(),
        });
    }
    input.evidence.push(Evidence {
        kind: "conversation".into(),
        label: "Z hiking".into(),
        source: "specific-thread".into(),
        members: ["p0".into(), "p1".into()].into_iter().collect(),
    });
    let members = ["p0", "p1", "p2", "p3"].map(str::to_owned);
    let (name, evidence) = naming::suggest(&input, &members);
    assert_eq!(name, "Z hiking");
    assert_eq!(evidence.len(), 5);
    assert_eq!(evidence[0].source, "specific-thread");
}

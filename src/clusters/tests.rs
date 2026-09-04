use super::*;
use std::collections::BTreeSet;

pub(super) fn fixture() -> input::Input {
    let people = (0..8)
        .map(|i| (format!("p{i}"), format!("Person {i}")))
        .collect();
    let mut edges = Vec::new();
    for offset in [0, 4] {
        for a in offset..offset + 4 {
            for b in a + 1..offset + 4 {
                edges.push((a, b, 3.0, 3.0));
            }
        }
    }
    edges.push((3, 4, 0.02, 1.0));
    input::Input {
        people,
        edges,
        evidence: vec![],
    }
}

#[test]
fn seeded_leiden_finds_planted_groups_and_is_repeatable() {
    let input = fixture();
    let groups = detect::partition(&input, 1.0, false, 42).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups, detect::partition(&input, 1.0, false, 42).unwrap());
    assert!(groups.iter().any(|g| g == &vec!["p0", "p1", "p2", "p3"]));
    assert_eq!(detect::agreement(&groups, &groups), 1.0);
}

#[test]
fn names_require_shared_evidence_and_specificity() {
    let mut input = fixture();
    let members = vec!["p0".into(), "p1".into(), "p2".into(), "p3".into()];
    assert!(
        naming::suggest(&input, &members)
            .0
            .starts_with("Group around")
    );
    input.evidence.push(input::Evidence {
        kind: "conversation".into(),
        label: "Hiking crew".into(),
        source: "thread".into(),
        members: members.iter().cloned().collect(),
    });
    assert_eq!(naming::suggest(&input, &members).0, "Hiking crew");
    input.evidence[0].members = (0..20).map(|i| format!("p{i}")).collect();
    assert!(
        naming::suggest(&input, &members)
            .0
            .starts_with("Group around")
    );
}

#[test]
fn stable_ids_survive_small_changes_but_splits_and_merges_have_lineage() {
    let input = fixture();
    let old = compute(&input, "balanced", 1.0, &[]).unwrap().clusters;
    let mut changed: Vec<_> = old.iter().map(|c| c.members.clone()).collect();
    changed[0].push("new-person".into());
    assert_eq!(identity::assign("balanced", &changed, &old)[0].0, old[0].id);
    let split = vec![
        old[0].members[..2].to_vec(),
        old[0].members[2..].to_vec(),
        old[1].members.clone(),
    ];
    let ids = identity::assign("balanced", &split, &old);
    assert_ne!(ids[0].0, old[0].id);
    assert_ne!(ids[1].0, old[0].id);
    assert_eq!(ids[0].1[0].id, old[0].id);
    let merged = vec![old.iter().flat_map(|c| c.members.clone()).collect()];
    assert_eq!(identity::assign("balanced", &merged, &old)[0].1.len(), 2);
}

fn database() -> (tempfile::TempDir, Connection) {
    let directory = tempfile::tempdir().unwrap();
    let connection = crate::db::open(&directory.path().join("crm.sqlite3")).unwrap();
    connection
        .execute(
            "INSERT INTO sources(id,kind) VALUES ('test','imessage')",
            [],
        )
        .unwrap();
    for id in ["a", "b", "c", "d"] {
        connection.execute("INSERT INTO people(id,display_name,apple_contact_id,lifecycle_state) VALUES (?1,?1,?1,'active')",[id]).unwrap();
    }
    for (thread, members) in [("big", vec!["a", "b", "c", "d"]), ("small", vec!["a", "b"])] {
        for (i, a) in members.iter().enumerate() {
            connection.execute("INSERT INTO conversation_memberships(source_id,thread_native_id,person_id,identity_value,conversation_title) VALUES ('test',?1,?2,?2,'Friends')",params![thread,a]).unwrap();
            for b in &members[i + 1..] {
                connection.execute("INSERT INTO relationship_contexts(source_person_id,target_person_id,source_id,thread_native_id,channel,first_observed_at,last_observed_at,message_count) VALUES (?1,?2,'test',?3,'imessage','2026','2026',1)",params![a,b,thread]).unwrap();
            }
        }
    }
    connection.execute("INSERT INTO relationships(id,source_person_id,target_person_id,first_observed_at,last_observed_at,shared_context_count) SELECT source_person_id||target_person_id,source_person_id,target_person_id,'2026','2026',COUNT(*) FROM relationship_contexts GROUP BY 2,3",[]).unwrap();
    (directory, connection)
}

#[test]
fn big_chats_are_discounted_without_changing_display_counts() {
    let (_directory, connection) = database();
    let input = input::load(&connection).unwrap();
    let ab = input
        .edges
        .iter()
        .find(|&&(a, b, _, _)| a == 0 && b == 1)
        .unwrap();
    assert!((ab.2 - 4.0 / 3.0).abs() < 1e-9);
    assert_eq!(ab.3, 2.0);
    let ac = input
        .edges
        .iter()
        .find(|&&(a, b, _, _)| a == 0 && b == 2)
        .unwrap();
    assert!((ac.2 - 1.0 / 3.0).abs() < 1e-9);
}

#[test]
fn resolution_changes_invalidate_cached_partitions() {
    let (_directory, connection) = database();
    let initial = load(&connection).unwrap();
    assert_eq!(
        initial.iter().map(|v| v.resolution).collect::<Vec<_>>(),
        vec![0.5, 1.5, 2.5]
    );
    let mut stale = initial[1].clone();
    stale.resolution = 1.0;
    stale.clusters.clear();
    connection
        .execute(
            "UPDATE network_cluster_cache SET payload=?1 WHERE level='balanced'",
            [json(&stale).unwrap()],
        )
        .unwrap();
    let refreshed = load(&connection).unwrap();
    assert_eq!(refreshed[1].resolution, 1.5);
    assert!(!refreshed[1].clusters.is_empty());
    assert_eq!(json(&refreshed[0]).unwrap(), json(&initial[0]).unwrap());
}

#[test]
fn cache_names_and_invalidation_round_trip() {
    let (_directory, connection) = database();
    let initial = load(&connection).unwrap();
    let again = load(&connection).unwrap();
    assert_eq!(json(&initial).unwrap(), json(&again).unwrap());
    let id = &initial[1].clusters[0].id;
    rename(&connection, id, Some("My group")).unwrap();
    connection
        .execute("INSERT INTO tags(person_id,tag) VALUES ('a','test')", [])
        .unwrap();
    let refreshed = load(&connection).unwrap();
    let group = refreshed[1].clusters.iter().find(|c| &c.id == id).unwrap();
    assert_eq!(group.name, "My group");
    assert!(group.custom_name);
    rename(&connection, id, None).unwrap();
    assert!(
        !load(&connection).unwrap()[1]
            .clusters
            .iter()
            .find(|c| &c.id == id)
            .unwrap()
            .custom_name
    );
    assert!(rename(&connection, id, Some("  ")).is_err());
    assert!(rename(&connection, "missing", Some("x")).is_err());
}

#[test]
fn empty_and_disconnected_graphs_are_supported() {
    let input = input::Input {
        people: vec![],
        edges: vec![],
        evidence: vec![],
    };
    assert!(
        compute(&input, "balanced", 1.0, &[])
            .unwrap()
            .clusters
            .is_empty()
    );
    let mut input = fixture();
    input.edges.pop();
    let groups = detect::partition(&input, 0.05, false, 42).unwrap();
    assert_eq!(groups.len(), 2);
    let covered: BTreeSet<_> = groups.into_iter().flatten().collect();
    assert_eq!(covered.len(), 8);
}

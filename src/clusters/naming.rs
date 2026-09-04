use super::input::Input;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NameEvidence {
    pub kind: String,
    pub label: String,
    pub source: String,
    pub member_count: usize,
    pub coverage: f64,
    pub specificity: f64,
}

pub fn suggest(input: &Input, members: &[String]) -> (String, Vec<NameEvidence>) {
    let members: BTreeSet<_> = members.iter().collect();
    let mut evidence: Vec<_> = input
        .evidence
        .iter()
        .filter_map(|e| {
            let count = e.members.iter().filter(|id| members.contains(id)).count();
            if count < 2 {
                return None;
            }
            Some(NameEvidence {
                kind: e.kind.clone(),
                label: e.label.clone(),
                source: e.source.clone(),
                member_count: count,
                coverage: count as f64 / members.len() as f64,
                specificity: count as f64 / e.members.len() as f64,
            })
        })
        .collect();
    evidence.sort_by(|a, b| {
        (b.coverage * b.specificity)
            .total_cmp(&(a.coverage * a.specificity))
            .then(a.label.cmp(&b.label))
            .then(a.source.cmp(&b.source))
    });
    let candidate = evidence
        .iter()
        .find(|e| e.coverage >= 0.5 && e.specificity >= 0.6 && e.label.chars().count() <= 70);
    let name = candidate
        .map(|e| e.label.trim().to_owned())
        .unwrap_or_else(|| {
            let mut degrees = std::collections::BTreeMap::<&String, f64>::new();
            for &(a, b, weight, _) in &input.edges {
                for id in [&input.people[a].0, &input.people[b].0] {
                    if members.contains(id) {
                        *degrees.entry(id).or_default() += weight;
                    }
                }
            }
            let mut ranked: Vec<_> = input
                .people
                .iter()
                .filter(|(id, _)| members.contains(id))
                .collect();
            ranked.sort_by(|a, b| {
                degrees
                    .get(&b.0)
                    .unwrap_or(&0.0)
                    .total_cmp(degrees.get(&a.0).unwrap_or(&0.0))
                    .then(a.0.cmp(&b.0))
            });
            let names = ranked
                .iter()
                .take(2)
                .map(|(_, name)| name.as_str())
                .collect::<Vec<_>>()
                .join(" & ");
            format!("Group around {names}")
        });
    evidence.truncate(5);
    (name, evidence)
}

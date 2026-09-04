use super::{Cluster, Predecessor};
use sha2::{Digest, Sha256};

pub fn assign(
    level: &str,
    groups: &[Vec<String>],
    previous: &[Cluster],
) -> Vec<(String, Vec<Predecessor>)> {
    let overlap = |a: &[String], b: &[String]| a.iter().filter(|id| b.contains(id)).count();
    let related: Vec<Vec<usize>> = groups
        .iter()
        .map(|members| {
            previous
                .iter()
                .enumerate()
                .filter_map(|(i, old)| {
                    let count = overlap(members, &old.members);
                    (count > 0 && count as f64 / members.len().min(old.members.len()) as f64 >= 0.5)
                        .then_some(i)
                })
                .collect()
        })
        .collect();
    groups
        .iter()
        .enumerate()
        .map(|(index, members)| {
            let parents = &related[index];
            let inherited = parents.first().filter(|&&i| {
                let old = &previous[i];
                let count = overlap(members, &old.members);
                parents.len() == 1
                    && related.iter().filter(|p| p.contains(&i)).count() == 1
                    && count as f64 / (members.len() + old.members.len() - count) as f64 >= 0.5
            });
            if let Some(&i) = inherited {
                return (previous[i].id.clone(), previous[i].predecessors.clone());
            }
            let hash = format!(
                "{:x}",
                Sha256::digest(format!("{level}:{}", members.join("\0")).as_bytes())
            );
            (
                format!("cluster-{}", &hash[..20]),
                parents
                    .iter()
                    .map(|&i| Predecessor {
                        id: previous[i].id.clone(),
                        name: previous[i].name.clone(),
                    })
                    .collect(),
            )
        })
        .collect()
}

mod detect;
mod identity;
mod input;
mod naming;
#[cfg(test)]
mod naming_tests;
#[cfg(test)]
mod participation_tests;
#[cfg(test)]
mod tests;

use crate::error::{CrmError, Result};
use naming::NameEvidence;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cluster {
    pub id: String,
    pub name: String,
    pub suggested_name: String,
    pub custom_name: bool,
    pub color: String,
    pub members: Vec<String>,
    pub evidence: Vec<NameEvidence>,
    pub predecessors: Vec<Predecessor>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Predecessor {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bridge {
    pub person_id: String,
    pub primary_cluster: String,
    pub secondary_cluster: String,
    pub external_share: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClusterLevel {
    pub level: String,
    pub resolution: f64,
    pub clusters: Vec<Cluster>,
    pub bridges: Vec<Bridge>,
    pub seed_agreement: f64,
    pub raw_weight_agreement: f64,
    pub raw_cluster_count: usize,
    pub internal_weight_share: f64,
    pub computed_at: String,
}

pub fn load(connection: &Connection) -> Result<Vec<ClusterLevel>> {
    let transaction = crate::db::immediate_transaction(connection)?;
    let input = input::load(&transaction)?;
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(format!("leiden-v3:{}", json(&input)?))
    );
    let mut levels = Vec::new();
    for (level, resolution) in [("broad", 0.5), ("balanced", 1.5), ("detailed", 2.5)] {
        let cached: Option<(String, String)> = transaction
            .query_row(
                "SELECT fingerprint,payload FROM network_cluster_cache WHERE level=?1",
                [level],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let previous: Option<ClusterLevel> = cached
            .as_ref()
            .map(|(_, payload)| decode(payload))
            .transpose()?;
        let mut result = if cached
            .as_ref()
            .is_some_and(|(hash, _)| hash == &fingerprint)
            && previous
                .as_ref()
                .is_some_and(|v| v.resolution == resolution)
        {
            previous.unwrap()
        } else {
            let mut old = previous.map(|v| v.clusters).unwrap_or_default();
            apply_names(&transaction, &mut old)?;
            let result = compute(&input, level, resolution, &old)?;
            transaction.execute("INSERT INTO network_cluster_cache(level,fingerprint,payload) VALUES (?1,?2,?3)
                ON CONFLICT(level) DO UPDATE SET fingerprint=excluded.fingerprint,payload=excluded.payload,updated_at=CURRENT_TIMESTAMP",
                params![level,fingerprint,json(&result)?])?;
            result
        };
        apply_names(&transaction, &mut result.clusters)?;
        levels.push(result);
    }
    transaction.commit()?;
    Ok(levels)
}

fn compute(
    input: &input::Input,
    level: &str,
    resolution: f64,
    previous: &[Cluster],
) -> Result<ClusterLevel> {
    let groups = detect::partition(input, resolution, false, 42)?;
    let alternate = detect::partition(input, resolution, false, 43)?;
    let raw = detect::partition(input, resolution, true, 42)?;
    let identities = identity::assign(level, &groups, previous);
    let clusters: Vec<_> = groups
        .iter()
        .zip(identities)
        .map(|(members, (id, predecessors))| {
            let (name, evidence) = naming::suggest(input, members);
            let digest = Sha256::digest(id.as_bytes());
            let hue = u16::from_be_bytes([digest[0], digest[1]]) % 360;
            Cluster {
                id,
                suggested_name: name.clone(),
                name,
                custom_name: false,
                color: format!("hsl({hue},65%,55%)"),
                members: members.clone(),
                evidence,
                predecessors,
            }
        })
        .collect();
    let membership: BTreeMap<_, _> = clusters
        .iter()
        .flat_map(|c| c.members.iter().map(move |id| (id, &c.id)))
        .collect();
    let mut links_by_group: BTreeMap<&String, BTreeMap<&String, (f64, usize)>> = BTreeMap::new();
    let (mut internal, mut total) = (0.0, 0.0);
    for &(a, b, w, _) in &input.edges {
        let a = &input.people[a].0;
        let b = &input.people[b].0;
        total += w;
        if membership[a] == membership[b] {
            internal += w;
        }
        for (node, other) in [(a, b), (b, a)] {
            let entry = links_by_group
                .entry(node)
                .or_default()
                .entry(membership[other])
                .or_default();
            entry.0 += w;
            entry.1 += 1;
        }
    }
    let mut bridges = Vec::new();
    for (id, weights) in links_by_group {
        let sum: f64 = weights.values().map(|v| v.0).sum();
        for (cluster, (weight, count)) in weights {
            if cluster != membership[id] && count >= 2 && weight / sum >= 0.2 {
                bridges.push(Bridge {
                    person_id: id.clone(),
                    primary_cluster: membership[id].clone(),
                    secondary_cluster: cluster.clone(),
                    external_share: weight / sum,
                });
            }
        }
    }
    Ok(ClusterLevel {
        level: level.into(),
        resolution,
        clusters,
        bridges,
        seed_agreement: detect::agreement(&groups, &alternate),
        raw_weight_agreement: detect::agreement(&groups, &raw),
        raw_cluster_count: raw.len(),
        internal_weight_share: if total > 0.0 { internal / total } else { 1.0 },
        computed_at: chrono::Utc::now().to_rfc3339(),
    })
}

fn apply_names(connection: &Connection, clusters: &mut [Cluster]) -> Result<()> {
    let mut statement =
        connection.prepare("SELECT name FROM network_cluster_names WHERE cluster_id=?1")?;
    for cluster in clusters {
        let name: Option<String> = statement
            .query_row([&cluster.id], |r| r.get(0))
            .optional()?;
        cluster.custom_name = name.is_some();
        cluster.name = name.unwrap_or_else(|| cluster.suggested_name.clone());
    }
    Ok(())
}

pub fn rename(connection: &Connection, id: &str, name: Option<&str>) -> Result<()> {
    let levels = load(connection)?;
    if !levels.iter().any(|l| l.clusters.iter().any(|c| c.id == id)) {
        return Err(CrmError::InvalidQuery(
            "Cluster no longer exists; refresh the network.".into(),
        ));
    }
    if let Some(name) = name {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err(CrmError::InvalidQuery(
                "Cluster names must be 1–80 characters without control characters.".into(),
            ));
        }
        connection.execute(
            "INSERT INTO network_cluster_names(cluster_id,name) VALUES (?1,?2)
            ON CONFLICT(cluster_id) DO UPDATE SET name=excluded.name,updated_at=CURRENT_TIMESTAMP",
            params![id, name],
        )?;
    } else {
        connection.execute(
            "DELETE FROM network_cluster_names WHERE cluster_id=?1",
            [id],
        )?;
    }
    Ok(())
}

fn json(value: &impl Serialize) -> Result<String> {
    serde_json::to_string(value).map_err(|e| CrmError::Serialization(e.to_string()))
}
fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T> {
    serde_json::from_str(value).map_err(|e| CrmError::Serialization(e.to_string()))
}

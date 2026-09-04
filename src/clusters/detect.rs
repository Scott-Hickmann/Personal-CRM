use super::input::Input;
use crate::error::{CrmError, Result};
use leiden_rs::{GraphDataBuilder, Leiden, LeidenConfig};
use std::collections::{BTreeMap, BTreeSet};

pub fn partition(input: &Input, resolution: f64, raw: bool, seed: u64) -> Result<Vec<Vec<String>>> {
    if input.people.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder = GraphDataBuilder::new(input.people.len());
    for &(a, b, weight, count) in &input.edges {
        builder
            .add_edge(a, b, if raw { count } else { weight })
            .map_err(error)?;
    }
    let graph = builder.build().map_err(error)?;
    let output = Leiden::new(LeidenConfig {
        resolution,
        seed: Some(seed),
        max_iterations: 30,
        ..Default::default()
    })
    .run(&graph)
    .map_err(error)?;
    let mut groups = BTreeMap::<usize, Vec<usize>>::new();
    for (node, group) in output.partition.iter() {
        groups.entry(group).or_default().push(node);
    }
    // Enforce connected components even if an implementation regresses on disconnected inputs.
    let mut adjacency = vec![Vec::new(); input.people.len()];
    for &(a, b, _, _) in &input.edges {
        adjacency[a].push(b);
        adjacency[b].push(a);
    }
    let mut result = Vec::new();
    for members in groups.into_values() {
        let mut remaining: BTreeSet<_> = members.into_iter().collect();
        while let Some(start) = remaining.pop_first() {
            let mut stack = vec![start];
            let mut component = Vec::new();
            while let Some(node) = stack.pop() {
                component.push(input.people[node].0.clone());
                for &neighbor in &adjacency[node] {
                    if remaining.remove(&neighbor) {
                        stack.push(neighbor);
                    }
                }
            }
            component.sort();
            result.push(component);
        }
    }
    result.sort();
    Ok(result)
}

fn error(error: impl std::fmt::Display) -> CrmError {
    CrmError::Ui(format!("clustering failed: {error}"))
}

// Fraction of people in matching best-overlap groups, symmetrized across both partitions.
pub fn agreement(a: &[Vec<String>], b: &[Vec<String>]) -> f64 {
    fn score(a: &[Vec<String>], b: &[Vec<String>]) -> f64 {
        let n: usize = a.iter().map(Vec::len).sum();
        if n == 0 {
            return 1.0;
        }
        a.iter()
            .map(|group| {
                b.iter()
                    .map(|other| group.iter().filter(|id| other.contains(id)).count())
                    .max()
                    .unwrap_or(0)
            })
            .sum::<usize>() as f64
            / n as f64
    }
    (score(a, b) + score(b, a)) / 2.0
}

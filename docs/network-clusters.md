# Network communities

On the Network page, choose **Cluster colors** to show communities and their names.
Use **Broad**, **Balanced**, or **Detailed** to choose granularity. Clicking a group
highlights its visible members without changing their positions. Search still
selects a person; tier and activity filters only change visibility, not membership.
The sidebar shows naming evidence, members, and people connecting groups. Cluster
headings appear at the center of their visible members. Affinity colors remain the
default, and all existing layouts keep the same positions and spacing.

Select a group to rename it. **Use suggestion** removes the custom override.
Names persist in the local CRM database, independently of the browser.

## Detection and evaluation

The backend uses `leiden-rs` 0.8.1 with modularity, refinement enabled, at most 30
iterations, and seed 42. Resolution presets are 0.5, 1.0, and 2.0. Input people
and edges are ordered by ID. Connected components are checked after detection so
disconnected components never share a community. Unconnected contacts and the
owner are excluded, matching the visible relationship graph.

Each observed conversation contributes `1 / (participants - 1)` to each pair,
using distinct eligible people. This discounts large conversations. If an edge
has no detailed conversation records, its stored context count is the fallback.
The original relationship count displayed in the graph is never changed.

Each preset also runs seed 43 and a raw-context-count baseline. The sidebar reports:

- Symmetric best-overlap membership agreement across seeds.
- Raw-weight community count and agreement with discounted weights.
- Fraction of discounted edge weight inside communities.

These are diagnostics, not confidence in a social interpretation. Bridge people
have at least two connections to another community accounting for at least 20%
of their total discounted connection weight. A person can bridge multiple groups
while retaining one primary membership.

Initial evaluation on the local network produced 27 / 35 / 49 groups, with about
96% / 90% / 92% cross-seed agreement. The raw-count baseline produced 26 / 31 / 38
groups. Results may change with subsequent source syncs. The synthetic tests cover
known communities, disconnected inputs, repeatability, and large-chat discounting.

## Naming

Suggested names use recorded conversation titles, tags, or explicit company,
organization, school, team, and club facts. Evidence is ranked by coverage times
specificity. A name needs at least two matching members, 50% group coverage,
60% specificity to this group, and a label at most 70 characters long. The sidebar
retains the naming evidence plus the highest-ranked supporting items (up to five)
and their member counts.

Without sufficient evidence, the fallback is “Group around [representative
members]”, ranked by connection weight. No message bodies are read for naming.
This implementation is deterministic and local: it does not require an AI provider,
send contact data elsewhere, or infer an organization or relationship from names.
There is no model prompt because no model is called. Custom names take precedence.

## Cache and identity

`crm cluster list` computes or reads all presets from SQLite. A versioned SHA-256
fingerprint covers input people, weights, and naming evidence. Unchanged inputs
reuse the cached result. The browser never runs clustering. The read and cache
update use a single transaction for a consistent input snapshot.

IDs and colors survive a one-to-one match with at least 50% Jaccard member overlap.
Splits and merges create new identities and record predecessor IDs and names.
Custom names remain stored against their original identities and are not silently
copied to unrelated children or merged groups. The sidebar displays predecessor
names when membership changes. Renaming does not refetch the graph or reset its
camera; an outdated cluster ID is rejected with a refresh message.

```sh
crm cluster list
crm --format json cluster list
crm cluster rename cluster-<id> "Weekend friends"
crm cluster reset-name cluster-<id>
```

Clustering data uses additive migration 020 (`network_cluster_cache` and
`network_cluster_names`). Contact records and conversation counts are unchanged.

References: [Leiden paper](https://www.nature.com/articles/s41598-019-41695-z),
[Rust implementation](https://docs.rs/leiden-rs/0.8.1/leiden_rs/).

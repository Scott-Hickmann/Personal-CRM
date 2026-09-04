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
iterations, and seed 42. Resolution presets are 0.5, 1.5, and 2.5. Input people
and edges are ordered by ID. Connected components are checked after detection so
disconnected components never share a community. Unconnected contacts and the
owner are excluded, matching the visible relationship graph.

Each observed conversation contributes
`(1 + ln(1 + min(messages_A, messages_B))) / (participants - 1)` to each pair,
using distinct eligible people. Message counts are distinct, non-deleted messages
sent by each person in that source/thread, not recipient appearances or total
thread traffic. This boosts mutual participation with diminishing returns while
discounting large conversations. Silent members or missing sender data retain the
original shared-thread contribution. Direct messages with the owner do not boost
connections between other people. Co-participation is not evidence of direct replies.
If an edge has no detailed conversation records, its stored context count is the fallback.
The original relationship count displayed in the graph is never changed.

Each preset also runs seed 43 and a raw-context-count baseline. The sidebar reports:

- Symmetric best-overlap membership agreement across seeds.
- Raw shared-thread-count community count and agreement with participation weights
  (the baseline has neither the participation boost nor the large-chat discount).
- Fraction of discounted edge weight inside communities.

These are diagnostics, not confidence in a social interpretation. Bridge people
have at least two connections to another community accounting for at least 20%
of their total discounted connection weight. A person can bridge multiple groups
while retaining one primary membership.

Participation-weighted evaluation on the local network produced 29 / 44 / 56 groups,
with about 97% / 93% / 91% cross-seed agreement. The raw-count baseline produced
26 / 40 / 42 groups. At unchanged resolutions, Balanced grew from 38 to 44 groups
and its largest group shrank from 153 to 126 people; Detailed grew from 49 to 56
and its largest shrank from 96 to 72. Cross-seed agreement fell from 97% to 93%
for Balanced and 94% to 91% for Detailed. These results do not establish that the
new groups are socially more meaningful; that requires reviewing membership.
Results may change with subsequent source syncs. The synthetic tests cover
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
reuse the cached result only when the preset resolution also matches. The browser never runs clustering. The read and cache
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

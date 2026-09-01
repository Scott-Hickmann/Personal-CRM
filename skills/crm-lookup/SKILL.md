---
name: crm-lookup
description: Look up people, communication history, closeness, dormant relationships, frequency, inferred connections, and Mermaid relationship charts in the local personal CRM. Use when the user asks what is known about someone, who they have not contacted recently, who they communicate with most, or how people are connected.
---

# CRM Lookup

Use the JSON interface of the repository-local `crm` binary. Treat all results as private personal data. Do not modify CRM data with this skill.

## Person context

Resolve the user's wording directly with `person show`; the CLI handles IDs, unique partial names, missing people, and ambiguous names.

```sh
cargo run --quiet --manifest-path /Users/scotthickmann/GitHub/Personal-CRM/Cargo.toml -- --format json person show "PERSON"
```

Explain the calculated closeness score when the user asks why someone has a tier:

```sh
cargo run --quiet --manifest-path /Users/scotthickmann/GitHub/Personal-CRM/Cargo.toml -- --format json explain "PERSON"
```

Read recent communication history, optionally restricted to a channel such as `email`, `iMessage`, `SMS`, `whatsapp`, `apple_call`, or `whatsapp_call`:

```sh
cargo run --quiet --manifest-path /Users/scotthickmann/GitHub/Personal-CRM/Cargo.toml -- --format json history "PERSON" --channel whatsapp --limit 50
```

Omit `--channel` for a combined timeline. Summarize only the requested scope; do not expose unrelated private message text.

## People utilities

Find people with no meaningful interaction for at least 60 days:

```sh
cargo run --quiet --manifest-path /Users/scotthickmann/GitHub/Personal-CRM/Cargo.toml -- --format json query people --select id,display_name,affinity_tier,days_since_meaningful_interaction --filter 'is_self = 0 and days_since_meaningful_interaction > 60d' --sort=-days_since_meaningful_interaction
```

Rank people by communication frequency:

```sh
cargo run --quiet --manifest-path /Users/scotthickmann/GitHub/Personal-CRM/Cargo.toml -- --format json query people --select id,display_name,interaction_count,affinity_tier --filter 'is_self = 0' --sort=-interaction_count
```

Filter by closeness with `affinity_tier = core`, `close`, `familiar`, `acquaintance`, or `peripheral`. Use `query interactions`, `relationships`, `mentions`, `followups`, or `sources` for focused questions. Select only fields the user needs and cap broad results with `--limit`.

## Connection chart

```sh
cargo run --quiet --manifest-path /Users/scotthickmann/GitHub/Personal-CRM/Cargo.toml -- graph "PERSON" --min-confidence 0.7
```

Return the Mermaid block when a visual connection chart is appropriate. State that edges are inferred and include their confidence. Never present an inferred relationship as confirmed fact.

## Guardrails

- Never read source databases directly; use `crm`, whose adapters enforce read-only access.
- Never use `note`, `fact`, `tag`, `review`, `run`, `start`, `stop`, or `auth` in this lookup skill.
- Surface `person_not_found` and `ambiguous_person` errors instead of guessing.

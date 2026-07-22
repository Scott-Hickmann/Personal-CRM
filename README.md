# Personal CRM

`crm` is a private, macOS-only command-line CRM. It imports communication metadata and available text into its own SQLite database, keeps source systems read-only, and uses local Ollama models to summarize interactions and infer relationship context.

## Requirements

- macOS
- Rust and Cargo
- Full Disk Access for the terminal or Codex process when reading protected Apple and WhatsApp databases
- Ollama with `qwen3.5:9b` and `embeddinggemma` for analysis
- A Google OAuth desktop client JSON file for each Gmail setup session

No binary installation is required. Add this to `~/.zshrc`:

```zsh
alias crm='cargo run --quiet --manifest-path /Users/scotthickmann/GitHub/Personal-CRM/Cargo.toml --'
```

## Initialize

```sh
crm config init \
  --self-name "Scott Hickmann" \
  --self-email "first@example.com" \
  --self-email "second@example.com" \
  --self-phone "+15551234567"
```

The default configuration and CRM database live in `~/Library/Application Support/crm/`. The directory is mode `0700`; configuration and database files are mode `0600`.

Review discovered source paths and health checks:

```sh
crm config show
crm doctor
crm status
```

## Gmail

Create a Google OAuth client with application type “Desktop app,” download its JSON credentials, then authorize each account interactively:

```sh
crm auth gmail add --credentials /path/to/client_secret.json
crm auth gmail list
crm auth gmail remove account@example.com
```

Refresh tokens are stored in macOS Keychain, not the configuration file. Gmail access uses the read-only scope and GET requests only. Sync excludes Spam and Trash. Deterministic bulk/automated email is reduced to metadata; local analysis can remove retained bodies it classifies as non-personal. All configured self email addresses are treated equally, including work and personal addresses.

## Synchronize and analyze

```sh
crm sync all
crm sync contacts
crm sync imessage
crm sync whatsapp
crm sync calls
crm sync gmail
crm analyze --limit 20
```

`sync all` imports Contacts, iMessage/SMS/MMS, WhatsApp messages, Apple/FaceTime calls, WhatsApp calls, and configured Gmail accounts. Repeated syncs are incremental where the source provides a cursor and reconcile deleted records into local tombstones.

Analysis sends bounded batches only to the configured local Ollama server. The editable prompt and response schema are in [`prompts/`](prompts/). There is intentionally no remote or alternate-model fallback: analysis fails when Ollama or a configured model is unavailable.

## People and manual information

```sh
crm person add --name "Alex Example"
crm person show "Alex"
crm history "Alex" --channel whatsapp --limit 50
crm note add --person "Alex" --text "Met through Sam"
crm fact set --person "Alex" --key "birthday" --value "May 4"
crm tag add --person "Alex" --tag "climbing"
```

Every mutation supports `--dry-run`. Partial names are accepted only when they resolve to exactly one person.

## Queries and relationship utilities

Commands support table output by default and stable `--format json` or `--format jsonl` output for agents.

```sh
crm query people \
  --select id,display_name,interaction_count,affinity_tier \
  --filter 'is_self = 0' \
  --sort=-interaction_count

crm query people \
  --select id,display_name,days_since_meaningful_interaction \
  --filter 'is_self = 0 and days_since_meaningful_interaction > 60d' \
  --sort=-days_since_meaningful_interaction

crm explain "Alex"
crm graph "Alex" --min-confidence 0.7
```

The query language accepts allowlisted fields only, uses bound SQL parameters, and supports `and`, `or`, `not`, parentheses, comparisons, `contains`, `in`, `is null`, day/week durations, sorting, grouping, selection, and limits.

`crm graph` emits a Mermaid diagram from inferred, confidence-filtered relationships. Unknown mentioned people remain unresolved; analysis never creates contacts automatically.

## Closeness model

Closeness is explainable rather than purely model-generated:

- Behavioral score: 90-day volume (40%), active-day consistency (25%), channel diversity (10%), incoming/outgoing balance (15%), and recency (10%).
- Final affinity: 70% behavioral score and 30% mean confidence of inferred relationship evidence.
- Tiers: `core` (80+), `close` (60+), `familiar` (40+), `acquaintance` (20+), and `peripheral` (below 20).
- Activity: `active` through 30 days, `cooling` through 90 days, `dormant` after 90 days, or `never`.

Use `crm explain PERSON` to see the components for one person. Inferred relationship types use the single-word fallback `unclear` when evidence is weak or unknown.

## Source safety

Local source SQLite databases are opened with SQLite's read-only flag, `PRAGMA query_only`, and an authorizer that rejects inserts, updates, deletes, schema changes, writable pragmas, and attached databases. Schema fingerprints and required columns are checked before import. Gmail is read with a read-only OAuth scope. The CRM never sends messages or writes to Contacts, Gmail, iMessage, WhatsApp, or call history.

Only the CRM-owned SQLite database, configuration file, and macOS Keychain token entries are mutated.

## Codex skills

The repository contains three skills:

- `crm-lookup`: read people, history, dormant/frequency views, scores, and connection charts.
- `crm-update`: dry-run and apply explicit people, note, fact, and tag mutations.
- `crm-sync-analyze`: health-check, synchronize, and run bounded local analysis; suitable for a user-configured daily Codex automation.

They are linked into `~/.agents/skills` during project setup. The skills invoke Cargo with the absolute manifest path, so they do not depend on the interactive zsh alias.

## Current limitations

- iMessage text stored only in Apple's archived `attributedBody` format is not decoded; standard text is imported.
- Source schemas are private implementation details of macOS and WhatsApp. `crm doctor` stops on an incompatible schema rather than guessing.
- Identity merging is exact by normalized email or phone. Ambiguous people are surfaced rather than automatically merged.

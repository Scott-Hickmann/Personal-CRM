# Personal CRM

`crm` is a private, macOS-only personal CRM with a launchd-managed background daemon. One selected iCloud Contacts container is the identity source of truth: every active CRM person maps to exactly one iCloud contact, while communication sources contribute interactions and suggestions without creating people. Local Ollama models summarize interactions and infer relationship context.

## Requirements

- macOS
- Rust and Cargo
- Full Disk Access for the terminal or Codex process when reading protected Apple and WhatsApp databases
- Ollama with `qwen3.5:9b` and `embeddinggemma` for analysis
- A Google OAuth desktop client JSON file for each Gmail setup session
- The Google People API enabled on the OAuth project when publishing contacts

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

## Configure authoritative contacts and Google mirrors

The selected iCloud container defines every active CRM person. Personal Google Contacts receives every usable iCloud contact, including phone-only contacts. Google Workspace receives only contacts with a work-labeled email or an email in a configured work domain.

List the available Apple contact containers. This reads the same protected local databases as `crm run contacts`, so the terminal or Codex process needs Full Disk Access:

```sh
crm contacts containers
```

Authorize Google Contacts access once for each Google account. This uses a separate Keychain token and does not broaden or replace the Gmail read token. If Gmail is already configured, its OAuth credentials file is reused; otherwise provide it explicitly on the first authorization:

```sh
crm auth contacts add
crm auth contacts add --credentials /path/to/client_secret.json
crm auth contacts list
```

Configure the selected iCloud container, both Google roles, and every domain that identifies a work email:

```sh
crm contacts configure \
  --source-container "CONTAINER-ID" \
  --personal-account "me@gmail.com" \
  --workspace-account "me@example.com" \
  --work-domain "example.com"
```

Run the first reconciliation and review existing CRM-only people before starting the daemon:

```sh
crm run contacts
crm review
crm review REVIEW-ID --link-icloud ICLOUD-CONTACT-ID
crm review REVIEW-ID --delete-person
crm review REVIEW-ID --approve
```

`--link-icloud` links a migration record to an existing iCloud contact. `--delete-person` permanently removes the migration-pending CRM person while preserving shared interactions as unassigned. Approving a migration or contact-candidate review creates a contact in the selected iCloud container and then reconciles it into CRM. Linking and approving are explicit, manual source writes and require macOS Contacts permission; deleting affects only the local CRM.

Published Google contacts carry a private ownership marker. The daemon automatically creates, updates, and recreates managed replicas. It never modifies an unmarked Google contact merely because an identity matches. Collisions and proposed deletions enter `crm review` and require explicit approval.

## Run the daemon

```sh
crm start
crm status
crm review
crm stop
```

The daemon watches the selected iCloud Contacts database, iMessage, WhatsApp, and call-history databases with a short debounce. It polls Gmail history every minute and reconciles Photos periodically. New interactions trigger bounded, debounced Ollama analysis followed by deterministic scoring. Jobs are coalesced in SQLite, survive restarts, and retry failures.

Manual recovery uses one command surface:

```sh
crm run contacts
crm run communications
crm run gmail
crm run analysis
crm run scoring
crm run photos
crm run google-publish
crm run suggestions
```

Analysis sends bounded batches only to the configured local Ollama server. The editable prompt and response schema are in [`prompts/`](prompts/). There is intentionally no remote or alternate-model fallback: analysis fails when Ollama or a configured model is unavailable.

## People and manual information

```sh
crm person show "Alex"
crm history "Alex" --channel whatsapp --limit 50
crm note add --person "Alex" --text "Met through Sam"
crm fact set --person "Alex" --key "birthday" --value "May 4"
crm tag add --person "Alex" --tag "climbing"
```

People cannot be created directly in CRM. Unmanaged Google Contacts and unresolved WhatsApp or Gmail identities become contact candidates in `crm review`; each suggestion shows its source, and approval creates the iCloud contact first. SMS, RCS, iMessage, Apple Calls, WhatsApp groups, and identities already present on an active iCloud contact are excluded. Notes, facts, and tags support `--dry-run`. Partial names are accepted only when they resolve to exactly one active or retired person.

## Match a face from Photos

Match a local photo containing one or more faces against the named people stored by macOS Photos:

```sh
crm face match /path/to/photo.jpg
```

The command reads the System Photo Library database read-only, compares every detected face against up to three stored faceprints per named person, and ranks the closest results for each face. Faces are numbered top-to-bottom and then left-to-right; normalized bounding boxes use a bottom-left origin. `--limit` controls the number of results per face. Lower distance means greater facial similarity; this is not security-grade biometric identification. The query photo is not copied or retained, and nothing is uploaded or written to Photos.

Full Disk Access may be required. If the library is outside the standard Pictures directory or more than one library is present, select it explicitly:

```sh
crm face match /path/to/photo.jpg --library "/path/to/Photos Library.photoslibrary"
```

Photos uses a private database schema and faceprint runtime, so the command validates both and stops if a macOS update makes either incompatible. Reference originals do not need to be downloaded from iCloud because matching uses the faceprints Photos already stores locally.

## Link CRM people to Photos

Review CRM people one at a time, confirm existing named Photos people, or provide a local image when no Photos person exists:

```sh
crm photos review
crm photos review --person "Alex"
crm photos review --person "Alex" --photo /path/to/alex.jpg
```

Name matches are suggestions only and always require confirmation. When a local image contains multiple faces, the command opens an annotated preview and asks which numbered face belongs to the CRM person. Approved images are imported through the Photos scripting interface into the `CRM Imports` album and tagged with `personal-crm:<person-id>`. The source image is not changed or retained by the CRM. If the same file is selected for another person in a group photo, the existing Photos asset is reused.

The imported asset is linked to the CRM immediately. Apple Photos may need time to analyze it; name the face in Photos, then complete the named-person association:

```sh
crm photos reconcile
crm photos status
```

`reconcile` confirms named faces on imported assets, refreshes Photos name changes, and marks removed or merged Photos identities as stale. The workflow never writes directly to `Photos.sqlite`; only the explicit, approved import changes Photos. Full Disk Access and permission to automate Photos may be required. Imports target the active System Photo Library, so `--library` is available for linking and reconciliation but intentionally disabled for importing.

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

Local source SQLite databases are opened with SQLite's read-only flag, `PRAGMA query_only`, and an authorizer that rejects inserts, updates, deletes, schema changes, writable pragmas, and attached databases. Schema fingerprints and required columns are checked before import. Gmail is read with a read-only OAuth scope. The CRM never sends messages or writes to Gmail messages, iMessage, WhatsApp, or call history.

External writes are limited to automatic non-destructive updates of tool-managed Google contacts, explicitly approved Google deletions, explicitly approved iCloud contact creation, and user-approved Photos imports. Source-deleted message bodies and locally classified non-personal email bodies may still be cleared for privacy; their interaction metadata and tombstones remain.

## Codex skills

The repository contains three skills:

- `crm-lookup`: read people, history, dormant/frequency views, scores, and connection charts.
- `crm-update`: dry-run and apply notes, facts, tags, and explicitly selected contact reviews.
- `crm-sync-analyze`: inspect daemon health or manually run recovery jobs.

They are linked into `~/.agents/skills` during project setup. The skills invoke Cargo with the absolute manifest path, so they do not depend on the interactive zsh alias.

## Current limitations

- iMessage text stored only in Apple's archived `attributedBody` format is not decoded; standard text is imported.
- Source schemas are private implementation details of macOS and WhatsApp. `crm doctor` stops on an incompatible schema rather than guessing.
- Identity merging is exact by normalized email or phone. Ambiguous people are surfaced rather than automatically merged.
- Deleted iCloud contacts become retained, inactive CRM people. Their Google deletions require review.

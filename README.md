# Personal CRM

`crm` is a private, Apple-silicon macOS personal CRM with a launchd-managed background daemon. One selected iCloud Contacts container is the identity source of truth: every active CRM person maps to exactly one iCloud contact, while communication sources contribute interactions and suggestions without creating people. Local MLX models summarize interactions, create semantic embeddings, and extract evidence about intimacy, support, affection, shared activity, and conflict repair.

## Requirements

- Apple-silicon macOS
- Rust and Cargo
- `uv` for the installed MLX runtime
- Node.js and pnpm 9 or newer for the local web companion
- Full Disk Access for the terminal or Codex process when reading protected Apple and WhatsApp databases
- About 6 GB of free storage for the default Gemma model downloads and MLX runtimes
- A Google OAuth desktop client JSON file for each Gmail setup session
- The Google People API enabled on the OAuth project when publishing contacts
- A persistent code-signing identity for Keychain access

## Install

The daemon must use a consistently signed executable. Otherwise macOS Keychain treats every rebuilt binary as a new application and asks for the login password once for each stored Google refresh token.

Create a self-signed code-signing certificate named `Personal CRM` once:

1. Open Keychain Access and choose **Certificate Assistant → Create a Certificate**.
2. Name it `Personal CRM`, set **Identity Type** to **Self Signed Root**, set **Certificate Type** to **Code Signing**, and create it in the login keychain. Set the certificate's trust policy to **Always Trust**.
3. Confirm that macOS recognizes the identity with `security find-identity -v -p codesigning`.

An Apple Development or Developer ID certificate also works. Set `PERSONAL_CRM_CODESIGN_IDENTITY` to its name or SHA-1 hash when running the installer.

Build, sign, and install the executable and its locked MLX runtimes:

```sh
./scripts/install-crm
```

The default installed path is `~/.local/bin/crm`; its private Python workers are installed beside it at `~/.local/bin/crm.support`. Override the executable with `PERSONAL_CRM_INSTALL_PATH` if necessary. The installer refuses to replace an existing signed installation when its designated code requirement differs. It restarts a daemon already using that path. During the first migration from a development executable, it stops the old daemon so Keychain access can be migrated before the signed daemon starts. The first analysis run downloads `mlx-community/gemma-4-E4B-it-qat-4bit` and `mlx-community/embeddinggemma-300m-bf16` into the standard Hugging Face cache.

The first signing operation may ask for permission to use the certificate's private key. Choose **Always Allow** so later installations are also non-interactive.

Add the installation directory to `PATH` in `~/.zshrc`:

```zsh
export PATH="$HOME/.local/bin:$PATH"
```

After the first signed installation, open Keychain Access and delete the existing `personal-crm.gmail` and `personal-crm.google-contacts` password items. Run the Gmail and Contacts authorization commands below again for each account, then run `crm start`. This one-time migration recreates the four refresh-token items for the signed executable; later signed upgrades retain access without password prompts. OAuth authorization state in Google and CRM synchronization checkpoints do not need to be reset.

Use `cargo test` for development. Do not use `cargo run` for commands that access the real Keychain or launch the daemon, because its rebuilt development executable does not have the installed binary's identity.

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

Refresh tokens are stored in macOS Keychain, not the configuration file. Gmail access uses the read-only scope and GET requests only. Gmail is treated as relationship evidence rather than a mailbox archive: historical searches cover messages involving active iCloud-contact email addresses, while contact discovery considers only direct outgoing mail from the last two years. Discovery messages are checked as header metadata first, and bodies are retrieved only after the message qualifies. Spam, Trash, Drafts, Promotions, Social, Forums, mailing lists, automated senders, and bulk mail are excluded before storage or analysis. Unknown incoming-only senders are ignored. All configured self email addresses are treated equally, including work and personal addresses.

The initial people-focused backfill is checkpointed by contact search and processes at most 50 message payloads per account in each daemon job. Completed searches and messages survive sleep, network loss, and daemon restarts. The daemon interleaves additional Gmail batches with review and local analysis; subsequent mailbox updates use Gmail history rather than repeating the backfill.

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
crm status --live
crm review
crm stop
```

`crm status --live` shows every active job, its current stage, and exact `x/n` progress for contacts, contact searches, messages, interactions, people, actions, and candidates. During Gmail backfill it reports completed contact searches, the queued message count, and kept-versus-excluded totals for the current batch.

## Local web companion

Start the private Next.js companion through the Rust CLI:

```sh
crm ui
crm ui --port 3100
```

The CLI starts `pnpm dev` in [`web/`](web/), binds it to the loopback interface only, and passes its own executable and active config path to the server. The app never opens the CRM database directly: contact search, detail views, interaction reveals, relationship graphs, notes, facts, tags, follow-ups, and closeness ratings all execute through Rust CLI JSON commands. Message bodies appear as previews until explicitly revealed. The relationship page renders the complete inferred network in Cytoscape and supports person, type, confidence, affinity-tier, activity, and layout filters.

The daemon watches the selected iCloud Contacts database, iMessage, WhatsApp, call-history, and Photos databases with a short debounce. Independent workers let those sources, Gmail, and local MLX analysis make progress concurrently; each workstream remains single-flight. Gmail history is polled every minute and linked Photos people are reconciled every five minutes as a fallback. New interactions trigger MLX analysis immediately, and each analysis run drains the complete pending snapshot before deterministic scoring runs. Jobs are coalesced in SQLite, changes arriving during a running job cause a follow-up pass, interrupted work survives restarts, and failures retry automatically.

iMessage, WhatsApp, Apple calls, and WhatsApp calls keep durable high-water checkpoints. Normal runs read only new rows plus a 1,000-row overlap for late edits. A daily full audit catches hard deletions that these local databases do not expose through a portable change feed. Contacts uses Core Data row versions as a lightweight change token when available, falling back to a full authoritative snapshot when the installed schema cannot provide one. Automatic Photos reconciliation queries only people already linked by UUID rather than loading every named Photos person.

Manual recovery uses one command surface:

```sh
crm run contacts
crm run imessage
crm run whatsapp
crm run apple-calls
crm run whatsapp-calls
crm run gmail
crm run analysis
crm run scoring
crm run photos
crm run google-publish
crm run suggestions
```

Analysis snapshots every eligible pending interaction and processes batches of eight by default. One batched MLX-LM pass extracts content metadata, participant scoring requests are flattened into batched passes, and only invalid responses enter one batched repair pass. Successful summaries are embedded together with MLX EmbeddingGemma. Each fully validated interaction is persisted in its own transaction, so successful peers complete even when another item fails and interrupted work resumes from the remaining records. Both model workers stay loaded for later analysis jobs in the same daemon process. Interactions imported during a running analysis cause an immediate follow-up pass; scoring waits until that follow-up work is drained. The editable prompts and response schemas are in [`prompts/`](prompts/). The `mlx` configuration section controls `generation_model`, `embedding_model`, `batch_size`, and `max_tokens`. There is intentionally no remote or alternate-model fallback.

## People and manual information

```sh
crm person show "Alex"
crm person delete RETIRED-PERSON-ID
crm history "Alex" --channel whatsapp --limit 50
crm note add --person "Alex" --text "Met through Sam"
crm fact set --person "Alex" --key "birthday" --value "May 4"
crm tag add --person "Alex" --tag "climbing"
```

People cannot be created directly in CRM. Unmanaged Google Contacts, unresolved WhatsApp people, and person-like recipients of direct outgoing Gmail become contact candidates in `crm review`; each suggestion shows its source, and approval creates the iCloud contact first. Incoming-only Gmail identities, shared mailboxes, SMS, RCS, iMessage, Apple Calls, WhatsApp groups, and identities already present on an active iCloud contact are excluded. Notes, facts, and tags support `--dry-run`. Partial names are accepted only when they resolve to exactly one active or retired person. `person delete` is restricted to retired, non-self people and preserves shared source interactions as unassigned.

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

crm query people --include-retired \
  --select id,display_name,lifecycle_state \
  --filter 'lifecycle_state = retired'

crm explain "Alex"
crm affinity rate --person "Alex" --rating 6
crm affinity clear --person "Alex"
crm graph "Alex" --min-confidence 0.7
```

People queries include active people by default; `--include-retired` makes retired records available for explicit filtering. The query language accepts allowlisted fields only, uses bound SQL parameters, and supports `and`, `or`, `not`, parentheses, comparisons, `contains`, `in`, `is null`, day/week durations, sorting, grouping, selection, and limits.

`crm graph` emits a Mermaid diagram from inferred, confidence-filtered relationships. Unknown mentioned people remain unresolved; analysis never creates contacts automatically.

## Closeness model

Closeness uses a local hybrid model:

- Behavioral evidence combines log-scaled 90-day volume (25%), 365-day volume (10%), active-week consistency (20%), channel diversity (5%), incoming/outgoing balance (15%), recency with a 60-day half-life (15%), and relationship duration (10%).
- Gemma 4 on MLX emits separate 0–3 observations for intimacy, emotional support, practical support, affection, shared activity, and conflict repair. Each observation retains a factual summary and its source interaction. Extraction confidence weights reliability; it never contributes relationship strength directly.
- Relational evidence uses a 365-day half-life and combines signal quality, repeated evidence, and breadth across the six dimensions. Its affinity weight grows with analyzed coverage up to 35%; before MLX evidence is available, the behavioral score remains the prior.
- Optional 1–7 user ratings directly anchor rated people to the matching tier. Once five ratings exist, they also fit a regularized monotonic calibration for unrated people; below that threshold the uncalibrated prior remains in place. Ratings can be managed in the person page or with `crm affinity rate` and `crm affinity clear`.
- Tiers: `core` (80+), `close` (60+), `familiar` (40+), `acquaintance` (20+), and `peripheral` (below 20).
- Activity: `active` through 30 days, `cooling` through 90 days, `dormant` after 90 days, or `never`.

Use `crm explain PERSON` or the person page to inspect behavior, model dimensions, evidence coverage, calibration, and the resulting score. Inferred relationship types use the single-word fallback `unclear` when evidence is weak or unknown. After upgrading from the earlier heuristic, retained interaction bodies are queued once for re-analysis so the new relational signals can be populated locally.

## Source safety

Local source SQLite databases are opened with SQLite's read-only flag, `PRAGMA query_only`, and an authorizer that rejects inserts, updates, deletes, schema changes, writable pragmas, and attached databases. Schema fingerprints and required columns are checked before import. Gmail is read with a read-only OAuth scope. The CRM never sends messages or writes to Gmail messages, iMessage, WhatsApp, or call history.

External writes are limited to automatic non-destructive updates of tool-managed Google contacts, explicitly approved Google deletions, explicitly approved iCloud contact creation, and user-approved Photos imports. Source-deleted message bodies and locally classified non-personal email bodies may still be cleared for privacy; their interaction metadata and tombstones remain.

## Codex skills

The repository contains three skills:

- `crm-lookup`: read people, history, dormant/frequency views, scores, and connection charts.
- `crm-update`: dry-run and apply notes, facts, tags, and explicitly selected contact reviews.
- `crm-sync-analyze`: inspect daemon health or manually run recovery jobs.

They are linked into `~/.agents/skills` during project setup. The skills invoke the signed binary at `/Users/scotthickmann/.local/bin/crm`, so they do not depend on interactive shell configuration and do not cause Keychain to reevaluate a development build.

## Current limitations

- iMessage text stored only in Apple's archived `attributedBody` format is not decoded; standard text is imported.
- Source schemas are private implementation details of macOS and WhatsApp. `crm doctor` stops on an incompatible schema rather than guessing.
- Identity merging is exact by normalized email or phone. Ambiguous people are surfaced rather than automatically merged.
- Deleted iCloud contacts become retained, inactive CRM people. Their Google deletions require review.

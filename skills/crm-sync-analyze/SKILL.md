---
name: crm-sync-analyze
description: Diagnose, synchronize, and locally analyze the personal CRM. Use for manual refreshes and scheduled Codex automations that should import macOS Contacts, iMessage, WhatsApp, call history, and configured Gmail accounts, then update summaries, mentions, inferred relationships, embeddings, closeness tiers, and activity states.
---

# CRM Sync and Analyze

Run a deterministic health-check, synchronization, and local Ollama analysis workflow. Source adapters perform real reads but must never write to Contacts, Gmail, iMessage, WhatsApp, or call-history stores.

## Workflow

1. Check configuration, database migrations, macOS source paths, permissions, and source schemas:

```sh
cargo run --quiet --manifest-path /Users/scotthickmann/GitHub/Personal-CRM/Cargo.toml -- --format json doctor
```

2. Stop and report the exact failing check if `doctor` fails. Full Disk Access may be required for protected macOS databases.
3. Synchronize every configured source:

```sh
cargo run --quiet --manifest-path /Users/scotthickmann/GitHub/Personal-CRM/Cargo.toml -- --format json sync all
```

4. Run one bounded local analysis batch:

```sh
cargo run --quiet --manifest-path /Users/scotthickmann/GitHub/Personal-CRM/Cargo.toml -- --format json analyze --limit 20
```

5. Report per-source imported/deleted counts and analysis counts. A zero count is a valid no-change result.

For scheduled automation, repeat analysis batches only when explicitly requested by the automation. Do not loop indefinitely: stop when `selected` is zero or at the automation's declared batch limit.

## Targeted synchronization

Use `sync contacts`, `sync imessage`, `sync whatsapp`, `sync calls`, or `sync gmail` only when the user names that source. Gmail sync is skipped when no Gmail accounts are configured.

## Failure handling

- OAuth setup is interactive and out of scope for scheduled runs. Report that `crm auth gmail add --credentials PATH` must be run manually.
- Analysis intentionally fails if Ollama is unavailable or either configured model is missing. Do not switch models, download models, or fall back to a remote service.
- Do not weaken source read-only protections or bypass schema checks.
- Never send messages or email, modify source data, or create account credentials.

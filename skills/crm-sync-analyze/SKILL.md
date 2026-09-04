---
name: crm-sync-analyze
description: Diagnose or manually refresh the daemon-managed personal CRM. Use when checking daemon health, retrying failed source work, or explicitly refreshing iCloud Contacts, local communication sources, Gmail, deterministic scores, Photos, or Google mirrors.
---

# CRM Sync and Maintenance

The launchd-managed CRM daemon normally handles synchronization and deterministic maintenance. Use this skill for health checks and explicit recovery. iCloud Contacts is authoritative; communication sources only contribute interactions and contact suggestions.

## Workflow

1. Inspect daemon, persisted work, review, and source status:

```sh
/Users/scotthickmann/.local/bin/crm --format json status
```

2. Use `doctor` only when paths, permissions, or source schemas may be broken.
3. Run only the recovery work needed for the request. A complete refresh is:

```sh
/Users/scotthickmann/.local/bin/crm --format json run contacts
/Users/scotthickmann/.local/bin/crm --format json run imessage
/Users/scotthickmann/.local/bin/crm --format json run whatsapp
/Users/scotthickmann/.local/bin/crm --format json run apple-calls
/Users/scotthickmann/.local/bin/crm --format json run whatsapp-calls
/Users/scotthickmann/.local/bin/crm --format json run gmail
/Users/scotthickmann/.local/bin/crm --format json run scoring
/Users/scotthickmann/.local/bin/crm --format json run google-publish
```

4. Run `crm review` after contact reconciliation. Never approve migration links, contact creation, Google deletions, or collisions unless the user explicitly requested that exact review action.
5. Report completed and failed work plus pending reviews. A no-change result is valid.

## Targeted synchronization

Use `run contacts`, `run imessage`, `run whatsapp`, `run apple-calls`, `run whatsapp-calls`, `run gmail`, `run scoring`, `run photos`, `run google-publish`, or `run suggestions`. Prefer `crm start` over creating a scheduled Codex automation.

## Failure handling

- OAuth setup and `crm start` are interactive setup operations; do not perform them unless requested.
- Do not weaken source read-only protections or bypass schema checks.
- Never send messages or email, modify communication sources, create account credentials, or approve a review implicitly.

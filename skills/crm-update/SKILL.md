---
name: crm-update
description: Add a note, fact, or tag to an existing iCloud-backed CRM person, or apply an explicitly requested contact review. Use when the user asks to remember personal information, organize a contact, or create an iCloud contact from a pending suggestion.
---

# CRM Update

Use the signed, installed `crm` binary. The user has authorized unambiguous local overlay updates without an extra confirmation, but each note, fact, or tag mutation must pass a dry-run preview first. Contact review actions require an explicit request naming the candidate or target.

## Workflow

1. Translate the request into exactly one supported mutation. People cannot be created directly in CRM because iCloud Contacts is authoritative.
2. For a note, fact, or tag, run the matching command with `--dry-run --format json`.
3. Inspect `person_id`, `operation`, `value`, and `dry_run`. Stop on missing or ambiguous people, an unexpected target, or a value that changes the user's meaning.
4. Repeat the same overlay command without `--dry-run`. For a contact review, inspect `crm review` and apply only the explicitly selected review ID.
5. Report the applied change concisely.

Do not combine multiple inferred changes. Ask a concise question when the intended person, fact key, or stored wording is materially unclear.

## Commands

List pending contact reviews:

```sh
/Users/scotthickmann/.local/bin/crm --format json review
```

Add a note:

```sh
/Users/scotthickmann/.local/bin/crm --format json note add --person "PERSON" --text "NOTE" --dry-run
```

Set a structured fact:

```sh
/Users/scotthickmann/.local/bin/crm --format json fact set --person "PERSON" --key "KEY" --value "VALUE" --dry-run
```

Add a tag:

```sh
/Users/scotthickmann/.local/bin/crm --format json tag add --person "PERSON" --tag "TAG" --dry-run
```

For the apply step, remove only `--dry-run`; preserve every other argument exactly.

When the user explicitly selects a pending contact candidate or migration review, apply that exact review ID:

```sh
/Users/scotthickmann/.local/bin/crm --format json review "REVIEW-ID" --approve
```

Use `--link-icloud CONTACT-ID` instead when the user explicitly chooses an existing iCloud contact. Approval can create an iCloud contact or delete one managed Google replica, so never infer review approval from a general request to update CRM data.

## Guardrails

- Store only what the user asked to remember. Do not manufacture facts from communication history.
- Prefer a note for free-form context, a fact for a stable key/value, and a tag for user-defined grouping.
- Contact creation is allowed only through an explicitly approved review. Do not edit Gmail, iMessage, WhatsApp, or call-history sources.
- This skill does not authenticate accounts, synchronize sources, or run model analysis.

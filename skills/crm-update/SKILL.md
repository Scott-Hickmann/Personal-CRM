---
name: crm-update
description: Add a person, note, fact, or tag to the local personal CRM. Use when the user asks to remember personal information or organize a contact. Preview every mutation with the CLI dry-run mode, inspect the exact target and value, then apply an unambiguous request without a second confirmation.
---

# CRM Update

Use the repository-local `crm` binary. The user has authorized unambiguous local CRM updates without an extra confirmation, but every supported mutation must pass a dry-run preview first.

## Workflow

1. Translate the request into exactly one supported mutation.
2. Run the matching command with `--dry-run --format json`.
3. Inspect `person_id`, `operation`, `value`, and `dry_run`. Stop on missing or ambiguous people, an unexpected target, or a value that changes the user's meaning.
4. Repeat the same command without `--dry-run`.
5. Report the applied change concisely.

Do not combine multiple inferred changes. Ask a concise question when the intended person, fact key, or stored wording is materially unclear.

## Commands

Add a person:

```sh
cargo run --quiet --manifest-path /Users/scotthickmann/GitHub/Personal-CRM/Cargo.toml -- --format json person add --name "NAME" --dry-run
```

Add a note:

```sh
cargo run --quiet --manifest-path /Users/scotthickmann/GitHub/Personal-CRM/Cargo.toml -- --format json note add --person "PERSON" --text "NOTE" --dry-run
```

Set a structured fact:

```sh
cargo run --quiet --manifest-path /Users/scotthickmann/GitHub/Personal-CRM/Cargo.toml -- --format json fact set --person "PERSON" --key "KEY" --value "VALUE" --dry-run
```

Add a tag:

```sh
cargo run --quiet --manifest-path /Users/scotthickmann/GitHub/Personal-CRM/Cargo.toml -- --format json tag add --person "PERSON" --tag "TAG" --dry-run
```

For the apply step, remove only `--dry-run`; preserve every other argument exactly.

## Guardrails

- Store only what the user asked to remember. Do not manufacture facts from communication history.
- Prefer a note for free-form context, a fact for a stable key/value, and a tag for user-defined grouping.
- Do not edit Gmail, Contacts, iMessage, WhatsApp, or call-history sources. The CLI has no source-write operation.
- This skill does not authenticate accounts, synchronize sources, or run model analysis.

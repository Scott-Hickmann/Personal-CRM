# Qwen analysis tuning sandbox

This read-only sandbox tests a two-pass CRM analysis design against local
Ollama. It never writes model output or changes the CRM database.

## Calls per interaction

1. One content call returns summary, personal classification, and mentions.
2. One relationship call per participant returns only that participant's six
   scores, confidence, and evidence.
3. Invalid stage output gets one repair call with the validator error and exact
   required ID. A second failure stops the interaction.
4. Code merges validated stage output. Nothing is persisted by this sandbox.

Each short prompt receives its exact compact JSON schema. The relationship
schema replaces `__PARTICIPANT_ID__` with a JSON Schema `const`, so a call cannot
legally return another participant.

## Commands

```sh
python3 -m unittest -v test_two_pass.py
python3 two_pass.py --case "three-person" --workers 3 --trials 3
python3 two_pass.py --case "eight participants" --workers 8
python3 two_pass.py --only-live \
  --live-db "/path/to/crm.sqlite3" \
  --live-id "interaction-id"
```

Use `--content-only` to tune the content prompt without running relationship
calls. Live message content is read in SQLite read-only mode and is never saved
or printed.

## Measured on qwen3.5:9b

The active Ollama model runner used one inference slot (`-np 1`). Concurrent
HTTP submission was correct, but completion times formed a queue rather than a
batch.

| Case | Participants | Serial | Concurrent | Ratio | Correct |
| --- | ---: | ---: | ---: | ---: | --- |
| Synthetic introduction, 3 trials | 3 | 71.80s | 65.72s | 1.09x | yes |
| Live blocking interaction | 3 | 29.12s | 14.57s | 2.00x | yes |
| Sparse group email | 8 | 59.61s | 62.09s | 0.96x | relationships yes |

The revised content prompt classified the sparse human-written email correctly
in three of three focused trials. The speed numbers vary with generated output
and daemon contention; they do not show true parallel inference while Ollama
runs with `-np 1`.

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

## Ollama versus MLX-LM speed

`main.py` compares warmed, single-request inference with thinking disabled. Run
the backends separately so their model weights do not compete for unified
memory:

```sh
uv run main.py ollama
ollama stop qwen3.5:9b
uv run main.py mlx-lm
uv run main.py mlx-lm --mlx-model mlx-community/gemma-4-e4b-4bit
```

The defaults compare Ollama's `qwen3.5:9b` Q4_K_M model with
`mlx-community/Qwen3.5-9B-MLX-4bit`. These are similar-sized 4-bit
quantizations, not identical weight formats. Each command performs a 32-token
warmup and three measured 256-token generations, then prints per-trial and
median prompt throughput, generation throughput, inference time, and wall
time. The final line is machine-readable JSON.

### Result

Measured September 2, 2026 on an M3 Pro MacBook Pro (12 cores, 18 GB unified
memory), using Ollama 0.32.15, MLX-LM 0.31.3, and MLX 0.32.2:

| Backend | Prompt | Generation | Inference | Wall |
| --- | ---: | ---: | ---: | ---: |
| Ollama Q4_K_M | 406.5 tok/s | 21.8 tok/s | 11.87s | 11.94s |
| MLX-LM Qwen 3.5 9B 4-bit | 187.7 tok/s | 27.0 tok/s | 9.71s | 9.95s |
| MLX-LM Gemma 4 E4B 4-bit | 411.0 tok/s | 39.2 tok/s | 6.63s | 6.86s |

MLX-LM was **1.24x faster at generation** and **1.22x faster in model
inference time**. Its median wall time was 1.20x faster. Ollama processed this
short 45-token prompt 2.17x faster, though its three prompt measurements were
more variable. All six measured runs reached the same 256-token limit.

Gemma 4 E4B was 1.45x faster than Qwen 3.5 9B at generation and used
4.36 GB peak memory instead of 5.27 GB. Its conversion has no chat template,
so the harness tokenized the benchmark prompt directly: Gemma processed 33
prompt tokens while Qwen processed 45 templated tokens. Generation used the
same prompt text and 256-token limit. MLX-LM 0.31.3 also needed the upstream
shared-KV loader fix mirrored in `main.py`; it only discards projections that
Gemma's shared-KV layers do not execute.

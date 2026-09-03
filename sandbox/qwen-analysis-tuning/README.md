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
uv run main.py mlx-lm --mlx-model mlx-community/gemma-4-E4B-it-qat-4bit
```

The defaults compare Ollama's `qwen3.5:9b` Q4_K_M model with
`mlx-community/Qwen3.5-9B-MLX-4bit`. These are similar-sized 4-bit
quantizations, not identical weight formats. Each command performs a 32-token
warmup and three measured 256-token generations, then prints per-trial and
median prompt throughput, generation throughput, inference time, and wall
time. The final line is machine-readable JSON.

### Result

Measured September 2–3, 2026 on an M3 Pro MacBook Pro (12 cores, 18 GB unified
memory), using Ollama 0.32.15, MLX-LM 0.31.3, and MLX 0.32.2:

| Backend | Prompt | Generation | Inference | Wall |
| --- | ---: | ---: | ---: | ---: |
| Ollama Q4_K_M | 406.5 tok/s | 21.8 tok/s | 11.87s | 11.94s |
| MLX-LM Qwen 3.5 9B 4-bit | 187.7 tok/s | 27.0 tok/s | 9.71s | 9.95s |
| MLX-LM Gemma 4 E4B IT QAT 4-bit | 331.0 tok/s | 27.8 tok/s | 9.33s | 9.53s |

MLX-LM was **1.24x faster at generation** and **1.22x faster in model
inference time**. Its median wall time was 1.20x faster. Ollama processed this
short 45-token prompt 2.17x faster, though its three prompt measurements were
more variable. All six measured runs reached the same 256-token limit.

The instruction-tuned Gemma checkpoint was 1.03x faster than Qwen 3.5 9B at
generation and used 5.96 GB peak memory instead of 5.27 GB. Generation used
the same prompt text and 256-token limit. MLX-LM 0.31.3 also needed the
upstream shared-KV loader fix mirrored in `main.py`; it only discards
projections that Gemma's shared-KV layers do not execute.

## Batched throughput

`batch.py` measures aggregate throughput for 1, 2, 4, and 8 simultaneous
requests. MLX-LM uses its native batch generator; Ollama receives concurrent
HTTP requests. Run each model separately:

```sh
uv run batch.py ollama
ollama stop qwen3.5:9b
uv run batch.py mlx-lm
uv run batch.py mlx-lm --mlx-model mlx-community/gemma-4-E4B-it-qat-4bit
```

The same machine and software versions produced these medians over three
trials, with 128 generated tokens per request. Effective throughput includes
all batch wall-clock overhead:

| Backend/model | Batch 1 | Batch 2 | Batch 4 | Batch 8 |
| --- | ---: | ---: | ---: | ---: |
| Ollama Qwen 3.5 9B | 16.1 tok/s | 16.1 tok/s | 16.0 tok/s | 15.8 tok/s |
| MLX-LM Qwen 3.5 9B | 19.2 tok/s | 35.9 tok/s | 63.5 tok/s | 70.1 tok/s |
| MLX-LM Gemma 4 E4B IT | 27.0 tok/s | 53.3 tok/s | 100.8 tok/s | 157.5 tok/s |

| Backend/model | Batch 1 wall | Batch 2 wall | Batch 4 wall | Batch 8 wall |
| --- | ---: | ---: | ---: | ---: |
| Ollama Qwen 3.5 9B | 7.93s | 15.88s | 32.09s | 64.69s |
| MLX-LM Qwen 3.5 9B | 6.67s | 7.13s | 8.06s | 14.60s |
| MLX-LM Gemma 4 E4B IT | 4.74s | 4.81s | 5.08s | 6.50s |

MLX-LM Qwen scaled to 3.65x its batch-1 throughput at batch 8. Its advantage
over Ollama grew from 1.19x at batch 1 to 4.44x at batch 8. Ollama's throughput
stayed flat and its wall time grew linearly because this Qwen 3.5 runner uses
one inference slot, so the concurrent requests queue. MLX-LM was close to
linear through batch 4, then began to saturate.

Gemma scaled to 5.83x its own batch-1 throughput and reached 157.5 tok/s at
batch 8, 2.25x MLX-LM Qwen's throughput. The Gemma-to-Qwen numbers compare
model choices and quantization schemes as well as inference speed, so they are
not a backend-only comparison. Also, MLX-LM's batch scheduler adds overhead at
batch 1: use the single-request generator for an interactive stream and
batching when multiple requests are ready together.

## Gemma E4B prompt accuracy

Accuracy uses the instruction-tuned
`mlx-community/gemma-4-E4B-it-qat-4bit` checkpoint. The similarly named
`mlx-community/gemma-4-e4b-4bit` checkpoint is a base model and is not suitable
for instruction-following evaluation.

```sh
ollama stop qwen3.5:9b
uv run two_pass.py --backend mlx-lm --trials 1
```

The fixture suite contains nine single-interaction cases and 25 participant
records. Each content prompt ran once and each relationship prompt ran twice,
for 59 validated responses total:

| Check | Gemma E4B IT | Qwen 3.5 9B |
| --- | ---: | ---: |
| Content cases passing | 9/9 | 9/9 |
| Full semantic cases passing | 9/9 | 8/9 |
| Responses validated | 59/59 | 59/59 |
| Repair calls | 0 | 0 |

Gemma passed automation detection, prompt-injection resistance, Unicode names,
mention extraction, exact zero-score cases, and the positive thresholds for
affection, shared activity, practical support, and conflict repair. Qwen's one
failure assigned nonzero relationship evidence to one recipient of an
automated build notification in both repeated scorings.

The first Gemma probe understood the content case but wrapped its JSON in a
Markdown fence. Removing Markdown fences around the schema examples and
explicitly requesting raw JSON fixed the protocol mismatch; all subsequent
responses parsed and validated on their first attempt. This is a small,
purpose-built regression suite rather than a broad accuracy benchmark.

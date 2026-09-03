#!/usr/bin/env python3
import argparse
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path
from statistics import median
import time

from main import load_mlx, render_mlx_prompt, run_ollama

ROOT = Path(__file__).resolve().parent


@dataclass
class BatchResult:
    batch_size: int
    generated_tokens: int
    wall_seconds: float
    effective_tps: float
    generation_tps: float | None = None
    peak_memory_gb: float | None = None


def run_mlx(runtime, prompt_tokens, batch_size, max_tokens):
    from mlx_lm import batch_generate

    mx, _, model, tokenizer = runtime
    if hasattr(mx, "reset_peak_memory"):
        mx.reset_peak_memory()
    started = time.perf_counter()
    response = batch_generate(
        model,
        tokenizer,
        [prompt_tokens] * batch_size,
        max_tokens=max_tokens,
        completion_batch_size=batch_size,
        prefill_batch_size=batch_size,
    )
    wall_seconds = time.perf_counter() - started
    stats = response.stats
    return BatchResult(
        batch_size=batch_size,
        generated_tokens=stats.generation_tokens,
        wall_seconds=wall_seconds,
        effective_tps=stats.generation_tokens / wall_seconds,
        generation_tps=stats.generation_tps,
        peak_memory_gb=stats.peak_memory,
    )


def run_ollama_batch(base_url, model, prompt, batch_size, max_tokens):
    started = time.perf_counter()
    with ThreadPoolExecutor(max_workers=batch_size) as executor:
        results = list(
            executor.map(
                lambda _: run_ollama(base_url, model, prompt, max_tokens),
                range(batch_size),
            )
        )
    wall_seconds = time.perf_counter() - started
    generated_tokens = sum(result.generation_tokens for result in results)
    return BatchResult(
        batch_size=batch_size,
        generated_tokens=generated_tokens,
        wall_seconds=wall_seconds,
        effective_tps=generated_tokens / wall_seconds,
    )


def benchmark(run, batch_sizes, trials, warmup_tokens, max_tokens):
    for batch_size in batch_sizes:
        run(batch_size, warmup_tokens)
        results = [run(batch_size, max_tokens) for _ in range(trials)]
        generation_tps = [
            result.generation_tps
            for result in results
            if result.generation_tps is not None
        ]
        peak_memory = [
            result.peak_memory_gb
            for result in results
            if result.peak_memory_gb is not None
        ]
        details = (
            f" generation={median(generation_tps):.1f} tok/s"
            if generation_tps
            else ""
        )
        if peak_memory:
            details += f" peak_memory={max(peak_memory):.2f}GB"
        print(
            f"batch={batch_size} effective={median(r.effective_tps for r in results):.1f} "
            f"tok/s wall={median(r.wall_seconds for r in results):.2f}s{details}",
            flush=True,
        )


def main():
    parser = argparse.ArgumentParser(description="Measure aggregate batched throughput")
    parser.add_argument("backend", choices=("ollama", "mlx-lm"))
    parser.add_argument("--batch-sizes", default="1,2,4,8")
    parser.add_argument("--trials", type=int, default=3)
    parser.add_argument("--max-tokens", type=int, default=128)
    parser.add_argument("--warmup-tokens", type=int, default=32)
    parser.add_argument("--prompt", type=Path, default=ROOT / "prompts/speed-benchmark.md")
    parser.add_argument("--ollama-model", default="qwen3.5:9b")
    parser.add_argument("--ollama-url", default="http://127.0.0.1:11434")
    parser.add_argument("--mlx-model", default="mlx-community/Qwen3.5-9B-MLX-4bit")
    args = parser.parse_args()
    batch_sizes = [int(value) for value in args.batch_sizes.split(",")]
    prompt = args.prompt.read_text().strip()

    if args.backend == "ollama":
        run = lambda size, tokens: run_ollama_batch(
            args.ollama_url, args.ollama_model, prompt, size, tokens
        )
    else:
        runtime = load_mlx(args.mlx_model)
        prompt_tokens = render_mlx_prompt(runtime[3], prompt)
        run = lambda size, tokens: run_mlx(runtime, prompt_tokens, size, tokens)
    benchmark(run, batch_sizes, args.trials, args.warmup_tokens, args.max_tokens)


if __name__ == "__main__":
    main()

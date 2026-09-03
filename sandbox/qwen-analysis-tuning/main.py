#!/usr/bin/env python3
import argparse
from dataclasses import asdict, dataclass
import json
from pathlib import Path
from statistics import median
import time
import urllib.request

ROOT = Path(__file__).resolve().parent


@dataclass
class Result:
    backend: str
    prompt_tokens: int
    prompt_tps: float
    generation_tokens: int
    generation_tps: float
    inference_seconds: float
    wall_seconds: float
    peak_memory_gb: float | None = None


def run_ollama(base_url, model, prompt, max_tokens):
    body = json.dumps(
        {
            "model": model,
            "prompt": prompt,
            "stream": False,
            "think": False,
            "keep_alive": "10m",
            "options": {
                "temperature": 0,
                "num_ctx": 4096,
                "num_predict": max_tokens,
                "seed": 0,
            },
        }
    ).encode()
    request = urllib.request.Request(
        f"{base_url.rstrip('/')}/api/generate",
        data=body,
        headers={"Content-Type": "application/json"},
    )
    started = time.perf_counter()
    with urllib.request.urlopen(request, timeout=600) as response:
        output = json.load(response)
    wall_seconds = time.perf_counter() - started
    prompt_seconds = output["prompt_eval_duration"] / 1_000_000_000
    generation_seconds = output["eval_duration"] / 1_000_000_000
    return Result(
        backend="ollama",
        prompt_tokens=output["prompt_eval_count"],
        prompt_tps=output["prompt_eval_count"] / prompt_seconds,
        generation_tokens=output["eval_count"],
        generation_tps=output["eval_count"] / generation_seconds,
        inference_seconds=prompt_seconds + generation_seconds,
        wall_seconds=wall_seconds,
    )


def load_mlx(model_name):
    import mlx.core as mx
    from mlx_lm import load, stream_generate

    model, tokenizer = load(model_name)
    return mx, stream_generate, model, tokenizer


def render_mlx_prompt(tokenizer, prompt):
    return tokenizer.apply_chat_template(
        [{"role": "user", "content": prompt}],
        add_generation_prompt=True,
        tokenize=True,
        enable_thinking=False,
    )


def run_mlx(runtime, prompt_tokens, max_tokens):
    mx, stream_generate, model, tokenizer = runtime
    if hasattr(mx, "reset_peak_memory"):
        mx.reset_peak_memory()
    started = time.perf_counter()
    final = None
    for final in stream_generate(model, tokenizer, prompt_tokens, max_tokens=max_tokens):
        pass
    wall_seconds = time.perf_counter() - started
    if final is None:
        raise RuntimeError("MLX-LM produced no timing result")
    inference_seconds = (
        final.prompt_tokens / final.prompt_tps
        + final.generation_tokens / final.generation_tps
    )
    return Result(
        backend="mlx-lm",
        prompt_tokens=final.prompt_tokens,
        prompt_tps=final.prompt_tps,
        generation_tokens=final.generation_tokens,
        generation_tps=final.generation_tps,
        inference_seconds=inference_seconds,
        wall_seconds=wall_seconds,
        peak_memory_gb=final.peak_memory,
    )


def print_results(results):
    for index, result in enumerate(results, 1):
        memory = (
            f" peak_memory={result.peak_memory_gb:.2f}GB"
            if result.peak_memory_gb is not None
            else ""
        )
        print(
            f"trial {index}: prompt={result.prompt_tokens} tokens "
            f"{result.prompt_tps:.1f} tok/s generation={result.generation_tokens} tokens "
            f"{result.generation_tps:.1f} tok/s inference={result.inference_seconds:.2f}s "
            f"wall={result.wall_seconds:.2f}s{memory}",
            flush=True,
        )
    print(
        f"median: prompt={median(r.prompt_tps for r in results):.1f} tok/s "
        f"generation={median(r.generation_tps for r in results):.1f} tok/s "
        f"inference={median(r.inference_seconds for r in results):.2f}s "
        f"wall={median(r.wall_seconds for r in results):.2f}s",
        flush=True,
    )
    print(json.dumps([asdict(result) for result in results], separators=(",", ":")))


def main():
    parser = argparse.ArgumentParser(description="Benchmark Ollama or MLX-LM on Qwen 3.5 9B")
    parser.add_argument("backend", choices=("ollama", "mlx-lm"))
    parser.add_argument("--trials", type=int, default=3)
    parser.add_argument("--max-tokens", type=int, default=256)
    parser.add_argument("--warmup-tokens", type=int, default=32)
    parser.add_argument("--prompt", type=Path, default=ROOT / "prompts/speed-benchmark.md")
    parser.add_argument("--ollama-model", default="qwen3.5:9b")
    parser.add_argument("--ollama-url", default="http://127.0.0.1:11434")
    parser.add_argument("--mlx-model", default="mlx-community/Qwen3.5-9B-MLX-4bit")
    args = parser.parse_args()
    prompt = args.prompt.read_text().strip()

    if args.backend == "ollama":
        run = lambda tokens: run_ollama(
            args.ollama_url, args.ollama_model, prompt, tokens
        )
    else:
        runtime = load_mlx(args.mlx_model)
        prompt_tokens = render_mlx_prompt(runtime[3], prompt)
        run = lambda tokens: run_mlx(runtime, prompt_tokens, tokens)

    print(f"warming {args.backend} with {args.warmup_tokens} generated tokens", flush=True)
    run(args.warmup_tokens)
    results = [run(args.max_tokens) for _ in range(args.trials)]
    print_results(results)


if __name__ == "__main__":
    main()

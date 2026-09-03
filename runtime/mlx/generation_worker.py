#!/usr/bin/env python3
import json
import sys


def patch_gemma4_loader():
    from mlx_lm.models import gemma4_text

    original_sanitize = gemma4_text.Model.sanitize

    def sanitize(model, weights):
        weights = original_sanitize(model, weights)
        first_shared = model.args.num_hidden_layers - model.args.num_kv_shared_layers
        projections = (".self_attn.k_proj", ".self_attn.v_proj", ".self_attn.k_norm")
        return {
            key: value
            for key, value in weights.items()
            if not (
                any(projection in key for projection in projections)
                and int(key.split("layers.")[1].split(".")[0]) >= first_shared
            )
        }

    gemma4_text.Model.sanitize = sanitize


def render_gemma4(messages):
    turns = []
    for message in messages:
        role = "model" if message["role"] == "assistant" else message["role"]
        turns.append(f"<|turn>{role}\n{message['content']}<turn|>\n")
    turns.append("<|turn>model\n")
    return "".join(turns)


def tokenize(tokenizer, messages):
    if tokenizer.chat_template is not None:
        return tokenizer.apply_chat_template(
            messages,
            add_generation_prompt=True,
            tokenize=True,
            enable_thinking=False,
        )
    return tokenizer.encode(render_gemma4(messages), add_special_tokens=True)


def respond(value):
    print(json.dumps(value, ensure_ascii=False, separators=(",", ":")), flush=True)


def prompt_length(prompt):
    tokens = prompt.get("input_ids") if hasattr(prompt, "get") else prompt
    shape = getattr(tokens, "shape", None)
    if shape:
        return shape[-1]
    if tokens and isinstance(tokens[0], list):
        return len(tokens[0])
    return len(tokens)


def prompt_batches(prompts, max_batch_size, max_batch_tokens, max_output_tokens):
    batch = []
    longest = 0
    for prompt in prompts:
        candidate_longest = max(longest, prompt_length(prompt))
        padded_tokens = (candidate_longest + max_output_tokens) * (len(batch) + 1)
        if batch and (
            len(batch) == max_batch_size or padded_tokens > max_batch_tokens
        ):
            yield batch
            batch = []
            longest = 0
        batch.append(prompt)
        longest = max(longest, prompt_length(prompt))
    if batch:
        yield batch


def main():
    if len(sys.argv) != 5:
        raise SystemExit(
            "usage: generation_worker.py MODEL BATCH_SIZE MAX_BATCH_TOKENS MAX_TOKENS"
        )

    model_name = sys.argv[1]
    batch_size = int(sys.argv[2])
    max_batch_tokens = int(sys.argv[3])
    max_tokens = int(sys.argv[4])
    patch_gemma4_loader()
    import mlx.core as mx
    from mlx_lm import batch_generate, load

    mx.set_cache_limit(512 * 1024 * 1024)
    model, tokenizer = load(model_name)
    tokenizer.eos_token_ids.add(tokenizer.convert_tokens_to_ids("<turn|>"))

    for line in sys.stdin:
        try:
            request = json.loads(line)
            messages = request["inputs"]
            prompts = [tokenize(tokenizer, item) for item in messages]
            outputs = []
            for batch in prompt_batches(
                prompts, batch_size, max_batch_tokens, max_tokens
            ):
                try:
                    result = batch_generate(
                        model,
                        tokenizer,
                        batch,
                        max_tokens=max_tokens,
                        completion_batch_size=len(batch),
                        prefill_batch_size=len(batch),
                    )
                    outputs.extend(result.texts)
                finally:
                    mx.clear_cache()
            respond({"outputs": outputs})
        except Exception as error:
            respond({"error": f"{type(error).__name__}: {error}"})


if __name__ == "__main__":
    main()

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


def main():
    if len(sys.argv) != 4:
        raise SystemExit("usage: generation_worker.py MODEL BATCH_SIZE MAX_TOKENS")

    model_name, batch_size, max_tokens = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
    patch_gemma4_loader()
    from mlx_lm import batch_generate, load

    model, tokenizer = load(model_name)
    tokenizer.eos_token_ids.add(tokenizer.convert_tokens_to_ids("<turn|>"))

    for line in sys.stdin:
        try:
            request = json.loads(line)
            messages = request["inputs"]
            prompts = [tokenize(tokenizer, item) for item in messages]
            result = batch_generate(
                model,
                tokenizer,
                prompts,
                max_tokens=max_tokens,
                completion_batch_size=batch_size,
                prefill_batch_size=batch_size,
            )
            respond({"outputs": result.texts})
        except Exception as error:
            respond({"error": f"{type(error).__name__}: {error}"})


if __name__ == "__main__":
    main()


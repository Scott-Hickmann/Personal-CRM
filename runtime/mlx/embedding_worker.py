#!/usr/bin/env python3
import json
import sys


def respond(value):
    print(json.dumps(value, ensure_ascii=False, separators=(",", ":")), flush=True)


def main():
    if len(sys.argv) != 2:
        raise SystemExit("usage: embedding_worker.py MODEL")

    import mlx.core as mx
    from mlx_embeddings import load

    model, tokenizer = load(sys.argv[1])
    for line in sys.stdin:
        try:
            request = json.loads(line)
            encoded = tokenizer(
                request["inputs"],
                padding=True,
                truncation=True,
                return_tensors="mlx",
            )
            output = model(encoded["input_ids"], encoded["attention_mask"])
            embeddings = output.text_embeds
            mx.eval(embeddings)
            respond({"outputs": embeddings.tolist()})
        except Exception as error:
            respond({"error": f"{type(error).__name__}: {error}"})


if __name__ == "__main__":
    main()


import time

from main import load_mlx


def render_gemma4_messages(messages):
    turns = []
    for message in messages:
        role = "model" if message["role"] == "assistant" else message["role"]
        turns.append(f"<|turn>{role}\n{message['content']}<turn|>\n")
    turns.append("<|turn>model\n")
    return "".join(turns)


def tokenize_messages(tokenizer, messages):
    if tokenizer.chat_template is not None:
        return tokenizer.apply_chat_template(
            messages,
            add_generation_prompt=True,
            tokenize=True,
            enable_thinking=False,
        )
    return tokenizer.encode(render_gemma4_messages(messages), add_special_tokens=True)


class MlxCaller:
    def __init__(self, model_name, max_tokens):
        self.runtime = load_mlx(model_name)
        self.max_tokens = max_tokens
        tokenizer = self.runtime[3]
        tokenizer.eos_token_ids.add(tokenizer.convert_tokens_to_ids("<turn|>"))

    def __call__(self, _base_url, _model, _schema, messages):
        _, stream_generate, model, tokenizer = self.runtime
        prompt_tokens = tokenize_messages(tokenizer, messages)
        started = time.perf_counter()
        chunks = [
            response.text
            for response in stream_generate(
                model, tokenizer, prompt_tokens, max_tokens=self.max_tokens
            )
        ]
        return "".join(chunks), time.perf_counter() - started

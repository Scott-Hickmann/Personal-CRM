#!/usr/bin/env python3
import argparse
from concurrent.futures import ThreadPoolExecutor
from copy import deepcopy
import json
from pathlib import Path
import time
import urllib.request

from live_fixture import load_live
from validation import SCORES, semantic_errors, validate_content, validate_relationship

ROOT = Path(__file__).resolve().parent


def load_json(path):
    return json.loads(Path(path).read_text())


def render_prompt(path, schema):
    template = Path(path).read_text()
    if template.count("{{json_schema}}") != 1:
        raise ValueError(f"{path} must contain one JSON schema marker")
    return template.replace("{{json_schema}}", json.dumps(schema, separators=(",", ":")))


def relationship_schema(template, participant_id):
    schema = deepcopy(template)
    schema["properties"]["participant_id"]["const"] = participant_id
    return schema


def render_repair(template, errors, required_id):
    manifest = json.dumps({"required_id": required_id}, separators=(",", ":"))
    return template.replace("{{validation_error}}", "; ".join(errors)).replace(
        "{{required_manifest}}", manifest
    )


def call_ollama(base_url, model, schema, messages):
    body = json.dumps(
        {
            "model": model,
            "stream": False,
            "think": False,
            "options": {"temperature": 0},
            "format": schema,
            "messages": messages,
        }
    ).encode()
    request = urllib.request.Request(
        f"{base_url.rstrip('/')}/api/chat",
        data=body,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=300) as response:
        envelope = json.load(response)
    return envelope["message"]["content"], envelope.get("total_duration", 0) / 1_000_000_000


def run_stage(
    payload,
    prompt,
    schema,
    validator,
    required_id,
    repair,
    base_url,
    model,
    call_model=call_ollama,
):
    started = time.perf_counter()
    messages = [
        {"role": "system", "content": prompt},
        {"role": "user", "content": json.dumps(payload, ensure_ascii=False, separators=(",", ":"))},
    ]
    server_seconds = 0.0
    for attempt in (1, 2):
        raw, duration = call_model(base_url, model, schema, messages)
        server_seconds += duration
        try:
            output = json.loads(raw)
            errors = validator(output)
        except json.JSONDecodeError as error:
            output = None
            errors = [f"invalid JSON: {error}"]
        if not errors:
            return output, attempt, server_seconds, time.perf_counter() - started
        if attempt == 1:
            messages.extend(
                [
                    {"role": "assistant", "content": raw},
                    {"role": "user", "content": render_repair(repair, errors, required_id)},
                ]
            )
    raise ValueError("; ".join(errors))


def relationship_payload(interaction, participant):
    record = {key: value for key, value in interaction.items() if key != "participants"}
    return {"record": record, "participant": participant}


def merge(content, relationships):
    return {
        "items": [
            {
                "interaction_id": content["interaction_id"],
                "summary": content["summary"],
                "is_personal": content["is_personal"],
                "mentions": content["mentions"],
                "relationship_signals": relationships,
            }
        ]
    }


def compare_execution(participants, score, workers, parallel_first=False):
    def run_serial():
        started = time.perf_counter()
        result = [score(participant) for participant in participants]
        return result, time.perf_counter() - started

    def run_parallel():
        started = time.perf_counter()
        with ThreadPoolExecutor(max_workers=min(workers, len(participants))) as executor:
            result = list(executor.map(score, participants))
        return result, time.perf_counter() - started

    if parallel_first:
        parallel, parallel_wall = run_parallel()
        serial, serial_wall = run_serial()
    else:
        serial, serial_wall = run_serial()
        parallel, parallel_wall = run_parallel()
    return serial, serial_wall, parallel, parallel_wall


def benchmark_case(case, assets, args, parallel_first):
    interaction = case["input"]["interactions"][0]
    content, content_attempts, _, content_wall = run_stage(
        interaction,
        assets["content_prompt"],
        assets["content_schema"],
        lambda output: validate_content(interaction, output),
        interaction["interaction_id"],
        assets["content_repair"],
        args.base_url,
        args.model,
    )
    if args.content_only:
        expected = case["expect"].get("is_personal")
        errors = [] if expected is None or content["is_personal"] is expected else [
            f"is_personal should be {expected}"
        ]
        print(
            f"{case['name']}: content={content_wall:.2f}s attempts={content_attempts} "
            f"semantic={'pass' if not errors else 'FAIL'}",
            flush=True,
        )
        for error in errors:
            print(f"  - {error}", flush=True)
        return None

    def score(participant):
        schema = relationship_schema(assets["relationship_schema"], participant["participant_id"])
        prompt = render_prompt(ROOT / "prompts/relationship.md", schema)
        return run_stage(
            relationship_payload(interaction, participant),
            prompt,
            schema,
            lambda output: validate_relationship(participant, output),
            participant["participant_id"],
            assets["relationship_repair"],
            args.base_url,
            args.model,
        )

    participants = interaction["participants"]
    serial, serial_wall, parallel, parallel_wall = compare_execution(
        participants, score, args.workers, parallel_first
    )
    serial_output = merge(content, [result[0] for result in serial])
    parallel_output = merge(content, [result[0] for result in parallel])
    errors = semantic_errors(serial_output, case["expect"]) + semantic_errors(parallel_output, case["expect"])
    speedup = serial_wall / parallel_wall
    print(
        f"{case['name']}: participants={len(participants)} content_attempts={content_attempts} "
        f"serial={serial_wall:.2f}s parallel={parallel_wall:.2f}s speedup={speedup:.2f}x "
        f"serial_server={sum(result[2] for result in serial):.2f}s "
        f"parallel_server={sum(result[2] for result in parallel):.2f}s "
        f"parallel_calls={[round(result[3], 2) for result in parallel]} "
        f"semantic={'pass' if not errors else 'FAIL'}",
        flush=True,
    )
    for error in errors:
        print(f"  - {error}", flush=True)
    return serial_wall, parallel_wall, errors


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--case", action="append", help="only cases containing this text")
    parser.add_argument("--trials", type=int, default=1)
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--content-only", action="store_true")
    parser.add_argument("--model", default="qwen3.5:9b")
    parser.add_argument("--base-url", default="http://127.0.0.1:11434")
    parser.add_argument("--live-db")
    parser.add_argument("--live-id")
    parser.add_argument("--only-live", action="store_true")
    args = parser.parse_args()
    if bool(args.live_db) != bool(args.live_id):
        parser.error("--live-db and --live-id must be supplied together")
    cases = load_json(ROOT / "fixtures.json")
    cases = [case for case in cases if len(case["input"]["interactions"]) == 1]
    if args.only_live:
        cases = []
    if args.case:
        needles = [value.casefold() for value in args.case]
        cases = [case for case in cases if any(value in case["name"].casefold() for value in needles)]
    if args.live_db:
        cases.append(load_live(args.live_db, args.live_id))
    if not cases:
        parser.error("no cases selected")
    content_schema = load_json(ROOT / "schemas/content.schema.json")
    assets = {
        "content_schema": content_schema,
        "content_prompt": render_prompt(ROOT / "prompts/content.md", content_schema),
        "relationship_schema": load_json(ROOT / "schemas/relationship.schema.json"),
        "content_repair": (ROOT / "prompts/content-repair.md").read_text(),
        "relationship_repair": (ROOT / "prompts/relationship-repair.md").read_text(),
    }
    totals = []
    for trial in range(1, args.trials + 1):
        for case in cases:
            print(f"trial {trial}: ", end="", flush=True)
            result = benchmark_case(case, assets, args, parallel_first=trial % 2 == 0)
            if result is not None:
                totals.append(result)
    if not totals:
        return
    serial = sum(result[0] for result in totals)
    parallel = sum(result[1] for result in totals)
    failures = sum(bool(result[2]) for result in totals)
    print(
        f"total: serial={serial:.2f}s parallel={parallel:.2f}s "
        f"speedup={serial / parallel:.2f}x semantic_failures={failures}",
        flush=True,
    )


if __name__ == "__main__":
    main()

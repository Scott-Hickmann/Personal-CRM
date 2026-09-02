SCORES = (
    "intimacy",
    "emotional_support",
    "practical_support",
    "affection",
    "shared_activity",
    "conflict_repair",
)


def validate_content(interaction, output):
    errors = []
    if not isinstance(output, dict):
        return ["content response must be an object"]
    if output.get("interaction_id") != interaction["interaction_id"]:
        errors.append(f"interaction_id must be {interaction['interaction_id']}")
    if not isinstance(output.get("summary"), str):
        errors.append("summary must be a string")
    if not isinstance(output.get("is_personal"), bool):
        errors.append("is_personal must be a boolean")
    mentions = output.get("mentions")
    if not isinstance(mentions, list):
        errors.append("mentions must be an array")
    else:
        for mention in mentions:
            if not isinstance(mention, dict) or not isinstance(mention.get("name"), str):
                errors.append("each mention must contain a name")
    return errors


def validate_relationship(participant, output):
    errors = []
    participant_id = participant["participant_id"]
    if not isinstance(output, dict):
        return ["relationship response must be an object"]
    if output.get("participant_id") != participant_id:
        errors.append(f"participant_id must be {participant_id}")
    for score in SCORES:
        value = output.get(score)
        if isinstance(value, bool) or not isinstance(value, int) or not 0 <= value <= 3:
            errors.append(f"{score} must be an integer 0..3")
    confidence = output.get("confidence")
    if isinstance(confidence, bool) or not isinstance(confidence, (int, float)) or not 0 <= confidence <= 1:
        errors.append("confidence must be 0..1")
    if not isinstance(output.get("evidence"), str):
        errors.append("evidence must be a string")
    return errors


def semantic_errors(output, expected):
    errors = []
    item = output["items"][0]
    if "is_personal" in expected and item["is_personal"] is not expected["is_personal"]:
        errors.append(f"is_personal should be {expected['is_personal']}")
    signals = {signal["participant_id"]: signal for signal in item["relationship_signals"]}
    for participant_id in expected.get("zero_scores", []):
        signal = signals[participant_id]
        if any(signal[score] for score in SCORES) or signal["evidence"] != "":
            errors.append(f"{participant_id} should have zero scores and empty evidence")
    for participant_id, minimums in expected.get("minimum_scores", {}).items():
        for score, minimum in minimums.items():
            if signals[participant_id][score] < minimum:
                errors.append(f"{participant_id} {score} should be at least {minimum}")
    names = {mention.get("name", "").casefold() for mention in item["mentions"]}
    for name in expected.get("required_mentions", []):
        if name.casefold() not in names:
            errors.append(f"expected mention {name}")
    return errors

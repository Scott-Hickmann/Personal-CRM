import json
import time
import unittest

import mlx_backend
import two_pass


class TwoPassTests(unittest.TestCase):
    def relationship_output(self, participant_id):
        output = {score: 0 for score in two_pass.SCORES}
        output.update({"participant_id": participant_id, "confidence": 1, "evidence": ""})
        return output

    def test_relationship_schema_requires_exact_participant(self):
        template = two_pass.load_json(two_pass.ROOT / "schemas/relationship.schema.json")

        schema = two_pass.relationship_schema(template, "participant-7")

        self.assertEqual(schema["properties"]["participant_id"]["const"], "participant-7")
        self.assertEqual(
            template["properties"]["participant_id"]["const"], "__PARTICIPANT_ID__"
        )

    def test_relationship_validator_rejects_wrong_participant(self):
        participant = {"participant_id": "participant-0"}
        output = {score: 0 for score in two_pass.SCORES}
        output.update({"participant_id": "participant-1", "confidence": 1, "evidence": ""})

        errors = two_pass.validate_relationship(participant, output)

        self.assertIn("participant_id must be participant-0", errors)

    def test_prompt_contains_compact_exact_schema(self):
        schema = {"type": "object", "additionalProperties": False}

        rendered = two_pass.render_prompt(two_pass.ROOT / "prompts/content.md", schema)

        self.assertIn('{"type":"object","additionalProperties":false}', rendered)
        self.assertNotIn("{{json_schema}}", rendered)

    def test_gemma4_messages_use_model_role_for_repairs(self):
        rendered = mlx_backend.render_gemma4_messages(
            [
                {"role": "system", "content": "rules"},
                {"role": "user", "content": "record"},
                {"role": "assistant", "content": "bad output"},
                {"role": "user", "content": "repair"},
            ]
        )

        self.assertEqual(
            rendered,
            "<|turn>system\nrules<turn|>\n"
            "<|turn>user\nrecord<turn|>\n"
            "<|turn>model\nbad output<turn|>\n"
            "<|turn>user\nrepair<turn|>\n"
            "<|turn>model\n",
        )

    def test_parallel_harness_overlaps_calls(self):
        participants = list(range(3))

        def score(participant):
            time.sleep(0.03)
            return participant

        serial, serial_wall, parallel, parallel_wall = two_pass.compare_execution(
            participants, score, 3, parallel_first=True
        )

        self.assertEqual(serial, parallel)
        self.assertLess(parallel_wall, serial_wall * 0.7)

    def test_stage_repairs_once_with_exact_id(self):
        participant = {"participant_id": "participant-0"}
        responses = [
            self.relationship_output("participant-wrong"),
            self.relationship_output("participant-0"),
        ]
        calls = []

        def fake(_base_url, _model, _schema, messages):
            calls.append(messages)
            return json.dumps(responses.pop(0)), 0

        output, attempts, _, _ = two_pass.run_stage(
            {"participant": participant},
            two_pass.render_prompt(
                two_pass.ROOT / "prompts/relationship.md",
                two_pass.relationship_schema(
                    two_pass.load_json(two_pass.ROOT / "schemas/relationship.schema.json"),
                    "participant-0",
                ),
            ),
            {},
            lambda value: two_pass.validate_relationship(participant, value),
            "participant-0",
            (two_pass.ROOT / "prompts/relationship-repair.md").read_text(),
            "unused",
            "unused",
            fake,
        )

        self.assertEqual(output["participant_id"], "participant-0")
        self.assertEqual(attempts, 2)
        self.assertIn("participant-0", calls[1][-1]["content"])

    def test_stage_stops_after_one_repair(self):
        participant = {"participant_id": "participant-0"}
        calls = 0

        def fake(_base_url, _model, _schema, _messages):
            nonlocal calls
            calls += 1
            return json.dumps(self.relationship_output("participant-wrong")), 0

        with self.assertRaises(ValueError):
            two_pass.run_stage(
                {"participant": participant},
                "unused",
                {},
                lambda value: two_pass.validate_relationship(participant, value),
                "participant-0",
                (two_pass.ROOT / "prompts/relationship-repair.md").read_text(),
                "unused",
                "unused",
                fake,
            )

        self.assertEqual(calls, 2)


if __name__ == "__main__":
    unittest.main()

import unittest

from generation_worker import prompt_batches, prompt_length


class PromptBatchTests(unittest.TestCase):
    def test_counts_batch_encoding_tokens(self):
        prompt = {"input_ids": [[1, 2, 3]], "attention_mask": [[1, 1, 1]]}

        self.assertEqual(prompt_length(prompt), 3)

    def test_keeps_short_prompts_in_maximum_batch(self):
        prompts = [[0] * 100 for _ in range(8)]

        batches = list(prompt_batches(prompts, 8, 800, 0))

        self.assertEqual([len(batch) for batch in batches], [8])

    def test_splits_on_padded_token_budget(self):
        prompts = [[0] * 2_000 for _ in range(8)]

        batches = list(prompt_batches(prompts, 8, 8_192, 0))

        self.assertEqual([len(batch) for batch in batches], [4, 4])

    def test_allows_one_oversized_prompt(self):
        prompts = [[0] * 9_000, [0] * 10]

        batches = list(prompt_batches(prompts, 8, 8_192, 0))

        self.assertEqual([len(batch) for batch in batches], [1, 1])

    def test_reserves_completion_tokens(self):
        prompts = [[0] * 800 for _ in range(8)]

        batches = list(prompt_batches(prompts, 8, 8_192, 256))

        self.assertEqual([len(batch) for batch in batches], [7, 1])


if __name__ == "__main__":
    unittest.main()

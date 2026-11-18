"""Acceptance checks for the documented pull request review process."""
from pathlib import Path
import re
import unittest


class ContributingReview(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        document = (Path(__file__).resolve().parents[2] / "CONTRIBUTING.md").read_text(
            encoding="utf-8"
        )
        # Scope assertions to contributor instructions, not mentions elsewhere.
        section = re.search(
            r"^## Sending a change\s*\n(.*?)(?=^## |\Z)",
            document,
            re.MULTILINE | re.DOTALL,
        )
        if section is None:
            raise AssertionError("CONTRIBUTING.md must explain how to send a change")
        cls.instructions = " ".join(section.group(1).split())

    def test_automated_reviewer_is_identified(self):
        self.assertRegex(
            self.instructions, r"CodeRabbit, an automatic reviewer,"
        )

    def test_contributors_are_told_to_expect_review_comments(self):
        self.assertRegex(
            self.instructions, r"CodeRabbit[^.]*reads it and leaves comments\."
        )

    def test_comments_receive_the_same_treatment_as_other_reviews(self):
        # Naming the bot alone must not replace guidance on handling its feedback.
        self.assertIn("Treat them like any other review.", self.instructions)


if __name__ == "__main__":
    unittest.main()

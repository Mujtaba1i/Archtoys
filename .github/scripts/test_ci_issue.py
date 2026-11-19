"""ci_issue.py: what counts as a failure, and what the issue says."""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(__file__))
import ci_issue  # noqa: E402


class CiIssue(unittest.TestCase):
    def test_only_failed_jobs_count(self):
        results = {"build": {"result": "success"}, "app-tests-x11": {"result": "failure"},
                   "benchmark": {"result": "skipped"}, "speed": {"result": "cancelled"}}
        self.assertEqual(ci_issue.failed_jobs(results), ["app-tests-x11"])
        self.assertEqual(ci_issue.failed_jobs({}), [])

    def test_the_log_lines_that_say_why(self):
        log = "\n".join([
            "2026-11-18T18:08:49.9Z [PASS] Pick: a click picks",
            "2026-11-18T18:08:50.0Z [FAIL] Settings: the Auto copy switch changes the setting (setting_autocopy False)",
            "2026-11-18T18:08:50.1Z ##[error]Process completed with exit code 1.",
            "2026-11-18T18:08:50.2Z ##[error]Process completed with exit code 1.",
            "2026-11-18T18:08:50.3Z installing fonts",
        ])
        self.assertEqual(ci_issue.why_lines(log), [
            "[FAIL] Settings: the Auto copy switch changes the setting (setting_autocopy False)",
            "##[error]Process completed with exit code 1.",
        ])

    def test_the_issue_names_each_failed_job_and_step(self):
        text = ci_issue.body("https://x/run/1", "a046681aaaa", [
            {"name": "App tests (X11)", "url": "https://x/job/2", "steps": ["End to end"], "why": ["[FAIL] Settings: x"]}])
        self.assertIn(ci_issue.MARKER, text)
        self.assertIn("`a046681`", text)
        self.assertIn("### ❌ [App tests (X11)](https://x/job/2)", text)
        self.assertIn("Failed step: **End to end**", text)
        self.assertIn("[FAIL] Settings: x", text)


if __name__ == "__main__":
    unittest.main()

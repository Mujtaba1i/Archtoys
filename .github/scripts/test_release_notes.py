"""Tests for release_notes.py: what goes in the release notes, and where."""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(__file__))
import release_notes  # noqa: E402

LOG = [
    "Mujtaba1i\tThe color picker no longer freezes\n",
    "github-actions[bot]\tAdd supporter: Sara\n",
    "dependabot[bot]\tBump zbus from 5.1.0 to 5.1.2\n",
    "Mujtaba1i\tRelease v0.7.2\n",
    "dependabot[bot]\tBump actions/checkout from 4.4.0 to 4.5.0\n",
    "Mujtaba1i\tThe color picker no longer freezes\n",
    "Mujtaba1i\t   \n",
]


class ReleaseNotes(unittest.TestCase):
    def test_users_read_only_real_changes_once(self):
        changes, _ = release_notes.split(LOG)
        self.assertEqual(changes, ["The color picker no longer freezes"])

    def test_dependabot_goes_to_the_technical_panel(self):
        _, technical = release_notes.split(LOG)
        self.assertEqual(technical, ["Bump zbus from 5.1.0 to 5.1.2", "Bump actions/checkout from 4.4.0 to 4.5.0"])

    def test_supporter_commits_are_left_out_entirely(self):
        changes, technical = release_notes.split(LOG)
        self.assertFalse(any("supporter" in c for c in changes + technical))


if __name__ == "__main__":
    unittest.main()

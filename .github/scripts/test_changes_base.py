"""Tests for changes-base.sh: where a release's changelog starts."""
import os
import subprocess
import tempfile
import unittest

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "changes-base.sh")
ENV = {
    **os.environ,
    "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "old@example.com",
    "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "old@example.com",
    "GIT_AUTHOR_DATE": "2026-01-01T00:00:00", "GIT_COMMITTER_DATE": "2026-01-01T00:00:00",
}


class ChangesBase(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.git("init", "-q", "-b", "main")

    def tearDown(self):
        self.dir.cleanup()

    def git(self, *args, env=ENV):
        return subprocess.run(["git", *args], cwd=self.dir.name, env=env, check=True,
                              capture_output=True, text=True).stdout.strip()

    def commit(self, name):
        with open(os.path.join(self.dir.name, name), "w") as f:
            f.write(name)
        self.git("add", name)
        self.git("commit", "-q", "-m", name)
        return self.git("rev-parse", "HEAD")

    def base(self, tag, commit=None):
        return subprocess.run(["bash", SCRIPT, tag, *([commit] if commit else [])], cwd=self.dir.name, check=True,
                              capture_output=True, text=True).stdout.strip()

    def test_first_release_starts_at_the_beginning(self):
        self.commit("a")
        self.git("tag", "v0.1.0")
        self.assertEqual(self.base("v0.1.0"), "")

    def test_starts_after_the_previous_release(self):
        self.commit("a")
        released = self.commit("Release v0.1.0")
        self.git("tag", "v0.1.0")
        self.commit("b")
        self.git("tag", "v0.1.1")
        self.assertEqual(self.base("v0.1.1"), released)
        self.assertEqual(self.base("v0.2.0", "HEAD"), self.git("rev-parse", "v0.1.1"))  # tag not made yet

    def test_finds_the_previous_release_after_history_was_rewritten(self):
        self.commit("a")
        self.commit("Release v0.4.1")
        self.git("tag", "v0.4.1")
        # every commit rewritten with another email: same files, new hashes,
        # and the old tag left pointing at the old copy
        new = {**ENV, "GIT_AUTHOR_EMAIL": "new@example.com", "GIT_COMMITTER_EMAIL": "new@example.com"}
        parent = None
        for old in self.git("rev-list", "--reverse", "main").split():
            args = ["commit-tree", old + "^{tree}", "-m", self.git("log", "-1", "--format=%s", old)]
            parent = self.git(*args, *(["-p", parent] if parent else []), env=new)
        self.git("reset", "-q", "--hard", parent)
        rewritten_release = parent
        self.commit("New look")
        self.git("tag", "v0.5.0")
        self.assertNotEqual(self.git("rev-parse", "v0.4.1"), rewritten_release)
        self.assertEqual(self.base("v0.5.0"), rewritten_release)


if __name__ == "__main__":
    unittest.main()

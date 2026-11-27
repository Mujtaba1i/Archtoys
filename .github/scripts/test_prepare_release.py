"""Runs prepare-release.sh end to end against a throwaway repo and remote."""
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "prepare-release.sh")
ENV = {
    **os.environ,
    "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.com",
    "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.com",
}
FILES = {
    "Cargo.toml": '[package]\nname = "archtoys"\nversion = "0.1.0"\n',
    "Cargo.lock": '[[package]]\nname = "archtoys"\nversion = "0.1.0"\n',
    "archtoys.spec": "Name: archtoys\nVersion: 0.1.0\nRelease: 1\n\n%changelog\n",
}


class PrepareRelease(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.origin = os.path.join(self.tmp.name, "origin.git")
        self.repo = os.path.join(self.tmp.name, "repo")
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", self.origin], check=True)
        subprocess.run(["git", "clone", "-q", self.origin, self.repo], check=True, capture_output=True)
        for name, text in FILES.items():
            self.write(name, text)
        self.commit("Start")
        self.git("push", "-q", "origin", "HEAD:main")

    def tearDown(self):
        self.tmp.cleanup()

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.repo, env=ENV, check=True,
                              capture_output=True, text=True).stdout.strip()

    def write(self, name, text):
        with open(os.path.join(self.repo, name), "w") as f:
            f.write(text)

    def commit(self, message):
        self.write("change.txt", message)
        self.git("add", "-A")
        self.git("commit", "-q", "-m", message)
        self.git("push", "-q", "origin", "HEAD:main")

    def release(self, tag):
        """Tags HEAD like the user does and runs the script; returns its outputs."""
        self.git("tag", "-f", tag)
        self.git("push", "-q", "-f", "origin", f"refs/tags/{tag}")
        out = os.path.join(self.tmp.name, "output")
        Path(out).write_text("")
        env = {**ENV, "GITHUB_REF_NAME": tag, "PACKAGER": "Test <t@example.com>",
               "GITHUB_OUTPUT": out, "GITHUB_REPOSITORY": "owner/archtoys"}
        run = subprocess.run(["bash", SCRIPT], cwd=self.repo, env=env, capture_output=True, text=True)
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.git("checkout", "-q", "main")
        self.git("pull", "-q", "--ff-only", "origin", "main")
        return Path(out).read_text()

    def test_release_notes_list_the_changes_since_the_previous_release(self):
        self.release("v0.1.0")
        self.commit("New HWB format")
        notes = self.release("v0.1.1")
        self.assertIn("version=0.1.1", notes)
        self.assertIn("## Changes since v0.1.0", notes)
        self.assertIn("- New HWB format", notes)
        self.assertNotIn("- Start", notes)
        self.assertIn("compare/v0.1.0...v0.1.1", notes)
        self.assertIn('version = "0.1.1"', Path(self.repo, "Cargo.toml").read_text())
        self.assertEqual(self.git("log", "-1", "--format=%s", "origin/main"), "Release v0.1.1")
        # CI passed on the tagged commit already: the release commit doesn't start it again
        self.assertIn("[skip ci]", self.git("log", "-1", "--format=%b", "origin/main"))
        self.assertEqual(self.git("rev-parse", "v0.1.1^{commit}"), self.git("rev-parse", "origin/main"))

    def test_retrying_a_tag_after_its_release_commit_was_pushed(self):
        # v0.5.1 stopped after pushing "Release v0.5.1"; the fix comes after it
        self.release("v0.1.0")
        self.commit("New HWB format")
        self.release("v0.1.1")
        self.commit("Fixed the release notes")
        notes = self.release("v0.1.1")
        self.assertIn("- New HWB format", notes)
        self.assertIn("- Fixed the release notes", notes)
        self.assertNotIn("Release v0.1.1", notes.split("NOTES_EOF")[1])
        spec = Path(self.repo, "archtoys.spec").read_text()
        self.assertEqual(spec.count("- 0.1.1-1"), 1)

    def test_the_metainfo_lists_each_release_with_its_changes(self):
        meta = "packaging/io.github.Mujtaba1i.Archtoys.metainfo.xml"
        os.makedirs(os.path.join(self.repo, "packaging"))
        self.write(meta, '<component>\n  <releases>\n    <release version="0.1.0" date="2026-01-01"/>\n  </releases>\n</component>\n')
        self.commit("Add metainfo")
        self.release("v0.1.0")
        self.commit("Colors & names <fast>")
        self.release("v0.1.1")
        text = Path(self.repo, meta).read_text()
        self.assertEqual(text.count('version="0.1.0"'), 1, text)
        self.assertLess(text.index('version="0.1.1"'), text.index('version="0.1.0"'), "newest first")
        self.assertIn("<li>Colors &amp; names &lt;fast&gt;</li>", text)
        # a retry of the same tag replaces its entry
        self.commit("One more fix")
        self.release("v0.1.1")
        text = Path(self.repo, meta).read_text()
        self.assertEqual(text.count('version="0.1.1"'), 1, text)
        self.assertIn("<li>One more fix</li>", text)

    def test_first_release_has_no_previous_one(self):
        notes = self.release("v0.1.0")
        self.assertIn("## Changes\n", notes)
        self.assertNotIn("compare/", notes)


if __name__ == "__main__":
    unittest.main()

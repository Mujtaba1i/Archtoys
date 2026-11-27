import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(__file__))
from flathub_manifest import rewrite  # noqa: E402

COMMIT = "0123456789abcdef0123456789abcdef01234567"

DIR_MANIFEST = """modules:
  - name: imagemagick
    sources:
      - type: archive
        url: https://example.invalid/im.tar.gz
  # the app itself
  - name: archtoys
    buildsystem: simple
    sources:
      - type: dir
        path: ../..
        skip:
          - target
          - dist
      - cargo-sources.json
"""


class FlathubManifest(unittest.TestCase):
    def test_the_local_dir_source_becomes_the_release_git_source(self):
        out = rewrite(DIR_MANIFEST, "v0.9.1", COMMIT)
        self.assertIn(
            "      - type: git\n"
            "        url: https://github.com/Mujtaba1i/Archtoys.git\n"
            "        tag: v0.9.1\n"
            f"        commit: {COMMIT}\n"
            "      - cargo-sources.json\n", out)
        self.assertNotIn("type: dir", out)
        self.assertNotIn("- target", out)
        # everything else, comments too, stays
        self.assertIn("  # the app itself\n", out)
        self.assertIn("url: https://example.invalid/im.tar.gz", out)

    def test_a_git_source_only_gets_a_new_tag_and_commit(self):
        once = rewrite(DIR_MANIFEST, "v0.9.1", COMMIT)
        newer = "f" * 40
        twice = rewrite(once, "v0.9.2", newer)
        self.assertIn("        tag: v0.9.2\n", twice)
        self.assertIn(f"        commit: {newer}\n", twice)
        self.assertEqual(twice.count("type: git"), 1)
        self.assertEqual(rewrite(twice, "v0.9.2", newer), twice)

    def test_bad_input_is_refused(self):
        with self.assertRaises(ValueError):
            rewrite(DIR_MANIFEST, "0.9.1", COMMIT)
        with self.assertRaises(ValueError):
            rewrite(DIR_MANIFEST, "v0.9.1", "abc123")
        with self.assertRaises(ValueError):
            rewrite("modules:\n  - name: other\n", "v0.9.1", COMMIT)

    def test_the_real_manifest_rewrites(self):
        path = os.path.join(os.path.dirname(__file__), "..", "..", "packaging", "flatpak", "io.github.Mujtaba1i.Archtoys.yml")
        with open(path, encoding="utf-8") as f:
            text = f.read()
        out = rewrite(text, "v0.9.1", COMMIT)
        self.assertIn(f"commit: {COMMIT}", out)
        self.assertIn("- cargo-sources.json", out)
        self.assertNotIn("type: dir", out)
        self.assertEqual(out.count("type: git"), 1)


if __name__ == "__main__":
    unittest.main()

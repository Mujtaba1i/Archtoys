"""Tests for report.py: what the failure issue says in each situation."""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(__file__))
import report  # noqa: E402

TAG, URL = "v0.3.1", "https://github.com/x/y/actions/runs/1"
ALL_OK = {j: "success" for j, _, _ in report.JOBS}


def results(**over):
    r = dict(ALL_OK)
    r.update({k.replace("_", "-"): v for k, v in over.items()})
    return r


def failure(name, step, errors="boom"):
    return {"name": name, "step": step, "url": "https://job", "errors": errors, "tail": "last lines"}


DEB = "Build + test the .deb (optional)"


class Report(unittest.TestCase):
    def test_everything_fine_means_no_issue_and_a_note_about_the_watcher(self):
        title, body, summary, code = report.build_report(TAG, URL, ALL_OK, [])
        self.assertEqual((title, body, code), (None, None, 0))
        self.assertIn("| Kind |", summary)
        self.assertIn("COPR watcher runs on its own", summary)

    def test_failing_before_the_bump_changes_nothing(self):
        r = results(prepare="failure", build_tarball="skipped", build_appimage="skipped", build_deb="skipped",
                    publish_release="skipped", attach_deb="skipped", trigger_copr="skipped", aur_bin="skipped",
                    aur_source="skipped", copr_watch="skipped")
        title, body, _, code = report.build_report(TAG, URL, r, [failure("Bump version + changelog", "Run prepare")])
        self.assertTrue(title.startswith("❌ Release v0.3.1 failed at: Bump version"))
        self.assertIn("version was **not** bumped", body)
        self.assertIn("git tag -d v0.3.1", body)
        self.assertEqual(code, 1)

    def test_build_failure_after_the_bump_says_to_pull_first(self):
        r = results(build_tarball="failure", publish_release="skipped", attach_deb="skipped", trigger_copr="success",
                    aur_bin="skipped", aur_source="skipped", copr_watch="skipped")
        _, body, _, _ = report.build_report(TAG, URL, r, [failure("Test + build binary tarball", "Run tests")])
        self.assertIn("git fetch --tags --force", body)
        self.assertIn("won't bump them again", body)

    def test_published_release_never_reuses_the_tag(self):
        r = results(aur_bin="failure")
        title, body, _, _ = report.build_report(TAG, URL, r, [failure("Deploy archtoys-bin to AUR", "Deploy")])
        self.assertIn("Re-run failed jobs", body)
        self.assertIn("git tag v0.3.2", body)
        self.assertNotIn("git tag -d", body)

    def test_only_the_deb_failing_says_the_release_is_published(self):
        r = results(build_deb="failure", attach_deb="skipped")
        title, body, summary, code = report.build_report(TAG, URL, r, [failure(DEB, "Create the .deb", "asset not found")])
        self.assertTrue(title.startswith("⚠️ Release v0.3.1 is published, but the .deb failed at"), title)
        self.assertIn("**The release is published. Only the .deb is missing.**", body)
        self.assertIn("Likely cause", body)                    # the per-step explanation
        self.assertIn("[package.metadata.deb]", body)
        self.assertIn("git tag v0.3.2", body)
        self.assertEqual(code, 1)                              # the run still turns red

    def test_every_deb_step_has_an_explanation(self):
        for step in report.STEP_ADVICE:
            _, body, _, _ = report.build_report(TAG, URL, results(build_deb="failure", attach_deb="skipped"),
                                                [failure(DEB, step)])
            self.assertIn("**Try:**", body, step)

    def test_attach_deb_failure_is_optional_too(self):
        r = results(attach_deb="failure")
        title, _, _, _ = report.build_report(TAG, URL, r, [failure("Attach the .deb to the release (optional)", "Attach to the release")])
        self.assertTrue(title.startswith("⚠️"))

    def test_required_and_optional_failing_together_counts_as_a_failed_release(self):
        r = results(aur_source="failure", build_deb="failure", attach_deb="skipped")
        title, body, _, _ = report.build_report(TAG, URL, r, [
            failure("Deploy archtoys to AUR", "Deploy"), failure(DEB, "Create the .deb")])
        self.assertTrue(title.startswith("❌ Release v0.3.1 failed at: Deploy archtoys to AUR"), title)
        self.assertIn("The optional part also failed", body)

    def test_watcher_start_failure_tells_how_to_start_it_by_hand(self):
        _, body, _, _ = report.build_report(TAG, URL, results(copr_watch="failure"),
                                            [failure("Start the COPR watcher", "Start the watcher")])
        self.assertIn("Actions → COPR watcher → Run workflow", body)

    def test_next_version(self):
        self.assertEqual(report.next_version("v0.3.9"), "v0.3.10")
        self.assertEqual(report.next_version("weird"), "weird")


if __name__ == "__main__":
    unittest.main()

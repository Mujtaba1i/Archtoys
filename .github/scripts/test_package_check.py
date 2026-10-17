"""Tests for package_check.py: the job matrix and what the issue says."""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(__file__))
import package_check as pc  # noqa: E402

ASSETS = [
    "archtoys-linux-x86_64.tar.gz", "Archtoys-0.3.2-x86_64.AppImage", "archtoys_0.3.2_amd64.deb",
    "fedora-44-x86_64-archtoys-0.3.2-1.fc44.x86_64.rpm", "fedora-rawhide-x86_64-archtoys-0.3.2-1.fc46.x86_64.rpm",
    "fedora-43-x86_64-archtoys-0.3.2-1.fc43.x86_64.rpm",
]


class Plan(unittest.TestCase):
    def test_one_fedora_job_per_attached_version_rawhide_last(self):
        self.assertEqual(pc.fedora_images(ASSETS), ["fedora:43", "fedora:44", "fedora:rawhide"])

    def test_every_channel_and_distro_is_its_own_job(self):
        names = [r["name"] for r in pc.plan(ASSETS)["include"]]
        for expected in ["AppImage on fedora:latest", ".deb on debian:12", "AUR archtoys on archlinux:latest",
                         "AUR archtoys-bin on archlinux:latest", "Fedora RPM on fedora:rawhide"]:
            self.assertIn(expected, names)
        self.assertEqual(len(names), len(set(names)))

    def test_no_rpms_means_no_fedora_jobs(self):
        self.assertFalse([r for r in pc.plan(ASSETS[:3])["include"] if r["channel"] == "rpm"])


class Issue(unittest.TestCase):
    def failure(self, name, step):
        return {"name": name, "step": step, "url": "https://job", "errors": "E: boom", "tail": "tail"}

    def test_install_failure_points_to_the_packaging_of_that_channel(self):
        _, body = pc.build_issue("v0.3.2", "https://run", [self.failure(".deb on debian:12", pc.INSTALL)])
        self.assertIn("| .deb on debian:12 | Install the package |", body)
        self.assertIn("points to the packaging", body)
        self.assertIn("[package.metadata.deb]", body)

    def test_start_failure_points_to_the_app(self):
        title, body = pc.build_issue("v0.3.2", "https://run", [self.failure("Fedora RPM on fedora:44", pc.START)])
        self.assertEqual(title, "Package check failed for v0.3.2")
        self.assertIn("points to the app", body)
        self.assertIn("Run workflow** with `v0.3.2`", body)

    def test_aur_source_is_not_mistaken_for_aur_bin(self):
        self.assertIn("makedepends", pc.advice_for("AUR archtoys on archlinux:latest", pc.INSTALL)[1])
        self.assertIn("aur-archtoys-bin", pc.advice_for("AUR archtoys-bin on archlinux:latest", pc.INSTALL)[1])


if __name__ == "__main__":
    unittest.main()

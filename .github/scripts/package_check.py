#!/usr/bin/env python3
"""Helper for package-check.yml, which installs the published packages of a
release on clean systems and starts the app.

  package_check.py plan     print the job matrix: every channel and distro,
                            plus one Fedora job per RPM attached to the release
  package_check.py report   summarise the run; on failure open (or update) ONE
                            issue per release, on success close it

A package that fails to build or install points to the pipeline; a package
that installs but does not start points to the app.
"""
import json
import os
import re
import subprocess
import sys

sys.path.insert(0, os.path.dirname(__file__))
from report import api_jobs, gh, log_excerpt  # noqa: E402

# channel id -> label used in the job name ("<label> on <image>")
CHANNELS = {
    "appimage": "AppImage",
    "deb": ".deb",
    "aur-bin": "AUR archtoys-bin",
    "aur": "AUR archtoys",
    "rpm": "Fedora RPM",
}
FIXED = [
    ("appimage", "ubuntu:22.04"), ("appimage", "fedora:latest"), ("appimage", "archlinux:latest"),
    ("deb", "ubuntu:22.04"), ("deb", "ubuntu:24.04"), ("deb", "debian:12"),
    ("aur-bin", "archlinux:latest"), ("aur", "archlinux:latest"),
]

INSTALL = "Install the package"
START = "Start the app"
# (likely cause, fix) per failed step; the install step depends on the channel
ADVICE = {
    "Start a clean container": (
        "Docker could not pull the distro image (a network hiccup).",
        "Re-run failed jobs."),
    (INSTALL, "appimage"): (
        "The AppImage could not be downloaded from the release. This points to the pipeline.",
        "Check that the release has `Archtoys-<version>-x86_64.AppImage`. If it is missing, look at the release run."),
    (INSTALL, "deb"): (
        "apt could not install the .deb: a dependency is missing or misnamed on this distro, or the .deb is "
        "not on the release. This points to the packaging.",
        "The apt error names the package. Fix `depends` in `[package.metadata.deb]` in Cargo.toml (and "
        ".github/scripts/check-deb.sh), then release the next patch version."),
    (INSTALL, "aur-bin"): (
        "makepkg failed, or the AUR still has the old version. This points to the packaging.",
        "If the error says the version is old, check the AUR jobs of the release run. Otherwise fix "
        "packaging/aur-archtoys-bin/PKGBUILD and release the next patch version."),
    (INSTALL, "aur"): (
        "Building from source with makepkg failed, or the AUR still has the old version. This points to the "
        "packaging (or a compile error).",
        "If the error says the version is old, check the AUR jobs of the release run. Otherwise fix "
        "packaging/aur-archtoys/PKGBUILD (a missing makedepends is common) and release the next patch version."),
    (INSTALL, "rpm"): (
        "The new version never showed up in COPR's repository, or dnf could not install it. This points to "
        "the pipeline or the spec file.",
        "Check the COPR project and any open \"COPR needs attention\" issue. A dnf dependency error means "
        "archtoys.spec needs fixing; then release the next patch version."),
    START: (
        "The package installs but the app exits within 10 seconds. This points to the app: a crash, or a "
        "library it loads at run time that the package does not pull in.",
        "Read the app's output in the log. Add a missing library to that package's dependencies, or fix the "
        "crash, then release the next patch version."),
}


def job_name(channel, image):
    return f"{CHANNELS[channel]} on {image}"


def fedora_images(asset_names):
    """fedora-43-x86_64-archtoys-....rpm -> fedora:43 (rawhide -> fedora:rawhide)."""
    found = {m.group(1) for n in asset_names if (m := re.match(r"^fedora-(\d+|rawhide)-.*\.rpm$", n))}
    return [f"fedora:{v}" for v in sorted(found, key=lambda v: (v == "rawhide", int(v) if v.isdigit() else 0))]


def plan(asset_names):
    rows = FIXED + [("rpm", image) for image in fedora_images(asset_names)]
    return {"include": [{"channel": c, "image": i, "name": job_name(c, i)} for c, i in rows]}


def advice_for(name, step):
    channel = next((c for c, label in CHANNELS.items() if name.startswith(label + " on ")), None)
    return ADVICE.get((step, channel)) or ADVICE.get(step)


def build_issue(tag, run_url, failures):
    """failures: [{name, step, url, errors, tail}] -> (title, body)."""
    title = f"Package check failed for {tag}"
    parts = [f"After the release, installing the published **{tag}** packages on clean systems failed.\n\n"
             f"**Run:** {run_url}\n\n| Channel and distro | Failed step |\n|---|---|"]
    parts += [f"| {f['name']} | {f['step']} |" for f in failures]
    parts.append("\nThis issue closes itself when a later package check passes. To check again after a fix: "
                 f"**Actions, Package check, Run workflow** with `{tag}`.\n")
    for f in failures:
        advice = advice_for(f["name"], f["step"])
        parts.append(f"### ❌ {f['name']} → {f['step']}\n\n[Open this job's full log]({f['url']})\n\n"
                     + (f"**Likely cause:** {advice[0]}\n\n**Fix:** {advice[1]}\n\n" if advice else "")
                     + (f"**Error lines:**\n```\n{f['errors']}\n```\n\n" if f.get("errors") else "")
                     + f"<details><summary>Last 60 log lines</summary>\n\n```\n{f.get('tail', '')}\n```\n</details>\n")
    return title, "\n".join(parts)[:60000]


def open_issue_number(title):
    out = gh("issue", "list", "--state", "open", "--limit", "100", "--json", "number,title", check=False)
    return next((i["number"] for i in json.loads(out or "[]") if i["title"] == title), None)


def main(argv):
    if len(argv) < 2 or argv[1] not in ("plan", "report"):
        print(__doc__)
        return 2
    tag = os.environ["TAG"]
    if argv[1] == "plan":
        names = subprocess.run(["gh", "release", "view", tag, "--json", "assets", "--jq", ".assets[].name"],
                               capture_output=True, text=True, check=True).stdout.split()
        matrix = plan(names)
        if not any(r["channel"] == "rpm" for r in matrix["include"]):
            print(f"::warning::{tag} has no RPMs attached yet, so Fedora is not checked.")
        print(json.dumps(matrix))
        return 0

    repo, run_id = os.environ["GITHUB_REPOSITORY"], os.environ["GITHUB_RUN_ID"]
    run_url = f"{os.environ['GITHUB_SERVER_URL']}/{repo}/actions/runs/{run_id}"
    failures = []
    for job in api_jobs(repo, run_id):
        if job.get("conclusion") != "failure":
            continue
        step = next((s["name"] for s in job.get("steps", []) if s.get("conclusion") == "failure"), "(unknown step)")
        errors, tail = log_excerpt(repo, job["id"])
        failures.append({"name": job["name"], "step": step, "url": job["html_url"], "errors": errors, "tail": tail})

    title = f"Package check failed for {tag}"
    number = open_issue_number(title)
    if not failures:
        print(f"Every package of {tag} installs and starts.")
        if number:
            gh("issue", "close", str(number), "--comment", f"Fixed: {run_url} passed.", check=False)
        return 0
    title, body = build_issue(tag, run_url, failures)
    if number:
        gh("issue", "comment", str(number), "--body", body)
    else:
        gh("issue", "create", "--title", title, "--body", body)
    print(title)
    return 1  # keep the run red


if __name__ == "__main__":
    sys.exit(main(sys.argv))

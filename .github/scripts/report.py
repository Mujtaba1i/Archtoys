#!/usr/bin/env python3
"""Last job of release.yml: summarise every job; on failure, open ONE issue that
says where it stopped, what was already changed, and the exact commands to
recover. Required parts failing means "the release failed"; optional parts (the
.deb) failing means "the release is published, only that part is missing". On
success, close any failure issue left open for this tag."""
import json
import os
import re
import subprocess
import sys

ICON = {"success": "✅", "failure": "❌", "cancelled": "⚪", "skipped": "⏭️"}

# job id in release.yml, label, required or optional
JOBS = [
    ("wait-for-ci",     "Wait for CI to pass",            "required"),
    ("prepare",         "Bump version + changelog",       "required"),
    ("build-tarball",   "Build binary tarball",           "required"),
    ("build-appimage",  "Build + smoke-test AppImage",    "required"),
    ("build-deb",       "Build + test the .deb",          "optional"),
    ("publish-release", "Publish GitHub release",         "required"),
    ("attach-deb",      "Attach the .deb to the release", "optional"),
    ("trigger-copr",    "Start COPR build",               "required"),
    ("aur-bin",         "Deploy archtoys-bin to AUR",     "required"),
    ("aur-source",      "Deploy archtoys to AUR",         "required"),
    ("copr-watch",      "Start the COPR watcher",         "required"),
]
LABEL = {j: l for j, l, _ in JOBS}
OPTIONAL = {j for j, _, k in JOBS if k == "optional"}

WHAT_HAPPENED = {
    "wait-for-ci":     "CI didn't pass for the commit you tagged (or never ran for it), so the release stopped at the very start.",
    "prepare":         "The release stopped while checking the tag and bumping the version. The error lines below say exactly why (for example, the tag isn't on your newest commit).",
    "build-tarball":   "The app failed to build (or failed the glibc check).",
    "build-appimage":  "The AppImage failed to build, failed the glibc check, or crashed in the smoke test.",
    "publish-release": "Creating the GitHub release failed.",
    "trigger-copr":    "COPR refused the webhook (usually a wrong or missing COPR_WEBHOOK_URL secret).",
    "aur-bin":         "Pushing archtoys-bin to the AUR failed (usually the AUR SSH key or a PKGBUILD problem).",
    "aur-source":      "Pushing archtoys to the AUR failed (usually the AUR SSH key or a PKGBUILD problem).",
    "copr-watch":      "The COPR watcher could not be started (usually a GitHub permission problem). COPR's build itself was already requested.",
    "build-deb":       "The .deb file failed to build or failed its tests. It is an optional part, so nothing else was held back.",
    "attach-deb":      "The .deb was built, but adding it to the release failed. It is an optional part.",
}

# What each step of the optional .deb job means when it fails.
STEP_ADVICE = {
    "Install the packaging tool": (
        "A network hiccup, or the pinned cargo-deb version needs a newer Rust.",
        "Re-run failed jobs. If it fails again, change the cargo-deb version in .github/workflows/release.yml."),
    "Build the app": (
        "A compile error, so the other builds usually fail too.",
        "Read the error lines and fix the code the same way as for the other builds."),
    "Check required glibc (max 2.35)": (
        "Something now needs a newer glibc than Ubuntu 22.04 has.",
        "Find what changed (usually a new dependency) and avoid it, as for the AppImage."),
    "Create the .deb": (
        "A packaging setting is wrong: a file path, icon or the desktop file.",
        "Check [package.metadata.deb] in Cargo.toml: the assets paths must exist, and the error names the first one that doesn't."),
    "Check dependencies and files": (
        "The .deb is missing a dependency or a file the check expects.",
        "The error line names it. Add it under depends or assets in [package.metadata.deb] in Cargo.toml, and keep .github/scripts/check-deb.sh in step."),
    "Install on a clean Ubuntu": (
        "A dependency is missing or misnamed on Ubuntu.",
        "The apt error in the log names the package. Fix its name in depends in Cargo.toml."),
    "Start the installed app": (
        "The app crashes right after starting when installed from the .deb.",
        "Read the crash output in the log. It is usually a library that must be added to depends in Cargo.toml."),
    "Attach to the release": (
        "A hiccup while uploading to GitHub.",
        "Re-run failed jobs."),
}


def next_version(tag):
    m = re.match(r"^v(\d+)\.(\d+)\.(\d+)$", tag)
    return f"v{m.group(1)}.{m.group(2)}.{int(m.group(3)) + 1}" if m else tag


def retry_same_tag(tag, bumped):
    """Commands to delete the tag, fix things and release the same tag again."""
    pull = (
        "# 1. Get the \"Release " + tag + "\" commit the bot already made\n"
        "git pull\n"
        "git fetch --tags --force\n\n"
    ) if bumped else (
        "# 1. Make sure you're up to date\n"
        "git pull\n\n"
    )
    return (
        "```bash\n"
        "cd ~/Archtoys\n\n"
        + pull +
        "# 2. Delete the tag on your PC and on GitHub (so you can create it again)\n"
        f"git tag -d {tag}\n"
        f"git push origin :refs/tags/{tag}\n\n"
        "# 3. Fix the problem, then save and upload the fix\n"
        "git add -A\n"
        "git commit -m \"Describe your fix here\"\n"
        "git push\n\n"
        "# 4. Create the same tag again on your newest commit and push it\n"
        f"git tag {tag}\n"
        f"git push origin {tag}\n"
        "```\n"
    )


def next_version_commands(tag):
    return (
        "```bash\n"
        "cd ~/Archtoys\n"
        "git pull\n"
        "git fetch --tags --force\n"
        "# fix the problem, then:\n"
        "git add -A\n"
        "git commit -m \"Describe your fix here\"\n"
        f"git tag {next_version(tag)}\n"
        f"git push origin main {next_version(tag)}\n"
        "```"
    )


RERUN = ("Open this run (link at the top) and click **Re-run failed jobs**. It continues from where it "
         "stopped and doesn't publish anything twice.")


def recovery_section(tag, failed_required, failed_optional, results):
    bumped = results.get("prepare") == "success"
    published = results.get("publish-release") == "success"
    first = (failed_required or failed_optional)[0]

    out = ["## 🛠️ How to fix it and retry", "", f"**What happened:** {WHAT_HAPPENED.get(first, 'A step failed.')}", ""]
    if failed_required and failed_optional:
        out += [f"The optional part also failed: {', '.join(LABEL[j] for j in failed_optional)}. See the details below.", ""]

    state = []
    state.append("✅ the version bump commit (\"Release " + tag + "\") was made and the tag moved onto it" if bumped
                 else "❌ the version was **not** bumped, nothing was committed")
    state.append("⚠️ the GitHub release **is published** (people can download it)" if published
                 else "❌ nothing was published: no GitHub release, AUR or RPM update")
    out += ["**What was already changed:**", ""] + [f"- {line}" for line in state] + [""]

    # Only the optional .deb failed: everything else is fine.
    if not failed_required:
        out += [
            "**What to do:**", "",
            f"1. If it looks temporary (a network or upload hiccup): {RERUN} Only the .deb is rebuilt and attached.",
            "2. If a packaging setting needs fixing (the table above the details says which): fix it, commit and "
            "push, then release the **next** patch version. A re-run would still use the old settings.",
            "", "**Next version:**", "", next_version_commands(tag), "",
            "The .deb is optional, so you can also just leave it for the next release.", "",
        ]
        return "\n".join(out)

    if not published:
        if first == "wait-for-ci":
            out += [
                "**First:** open the CI run (link in the error below) and read why it failed. "
                "If CI never ran, you probably pushed only the tag: just push your commits (`git push`), "
                "wait for CI to go green, then click **Re-run all jobs** on this release run. "
                "Otherwise, fix the problem and retry:", "",
            ]
        elif first == "prepare":
            out += ["**First:** read the error lines below; they say what to change. Then retry:", ""]
        else:
            out += [
                "**First:** read the error lines below. If it looks like a one-off hiccup (network, a "
                "download timing out), just click **Re-run failed jobs** on the run. If something in "
                "the code needs fixing, retry with the same tag:", "",
            ]
        out += [f"**Retry with the same tag ({tag}):**", "", retry_same_tag(tag, bumped)]
        if bumped:
            out += [
                "The version files already say the new version, so the retry won't bump them again, "
                "and it rewrites this version's changelog entry so your fix is listed too.", "",
            ]
        out += ["Then watch the new run in the **Actions** tab. This issue closes itself when it succeeds.", ""]
        return "\n".join(out)

    # Already published: don't reuse the tag, people may have downloaded it.
    if first == "trigger-copr":
        steps = [
            "1. Check the **COPR_WEBHOOK_URL** secret: GitHub → Settings → Secrets and variables → Actions. "
            "It must be COPR's *custom* webhook URL, ending in `/archtoys/`.",
            f"2. {RERUN}",
        ]
    elif first in ("aur-bin", "aur-source"):
        steps = [
            "1. If the error mentions `Permission denied (publickey)`: the AUR key in the secret "
            "**AUR_SSH_PRIVATE_KEY** isn't registered on your AUR account. Add the matching public key "
            "on aur.archlinux.org → My Account, then go to step 2.",
            f"2. {RERUN}",
            "3. If the **PKGBUILD itself** needs fixing: fix it, commit and push, then release the **next** "
            "version (see below). A re-run would still use the old PKGBUILD.",
        ]
    elif first == "copr-watch":
        steps = [
            f"1. {RERUN}",
            f"2. Or start the watcher by hand: **Actions → COPR watcher → Run workflow**, type `{tag}`.",
        ]
    else:
        steps = [f"1. {RERUN}"]
    out += ["**What to do:**", ""] + steps + [""]
    out += [
        "**If the code needs changing:** because this version is already published, don't reuse "
        f"{tag}. Fix it and release the next version instead:", "", next_version_commands(tag), "",
    ]
    return "\n".join(out)


def build_report(tag, run_url, results, failures):
    """Returns (title, body, summary_markdown, exit_code).
    results: {job id: result}; failures: [{id, name, step, url, errors, tail}]."""
    failed = [j for j, _, _ in JOBS if results.get(j) == "failure"]
    failed_required = [j for j in failed if j not in OPTIONAL]
    failed_optional = [j for j in failed if j in OPTIONAL]

    table = ["| | Step | Kind | Result |", "|---|---|---|---|"]
    for jid, label, kind in JOBS:
        r = results.get(jid, "skipped")
        table.append(f"| {ICON.get(r, '❔')} | {label} | {kind} | {r} |")
    table = "\n".join(table)
    watcher_note = ("The COPR watcher runs on its own, so the RPMs may still be on their way. It attaches them "
                    "by itself and opens an issue only if COPR needs your attention.")

    if not failed:
        return None, None, f"# ✅ Release {tag} finished\n\n{table}\n\n{watcher_note}\n", 0

    details = []
    for f in failures:
        advice = STEP_ADVICE.get(f.get("step", ""))
        advice_text = (f"**Likely cause:** {advice[0]}\n\n**Try:** {advice[1]}\n\n" if advice and "(optional)" in f["name"] else "")
        details.append(
            f"### ❌ {f['name']} → {f['step']}\n\n[Open this job's full log]({f['url']})\n\n{advice_text}"
            + (f"**Error lines:**\n```\n{f['errors']}\n```\n\n" if f.get("errors") else "")
            + f"<details><summary>Last 60 log lines</summary>\n\n```\n{f.get('tail', '')}\n```\n</details>\n")

    where = ""
    for f in failures:
        if ("(optional)" in f["name"]) == (not failed_required):
            where = f"{f['name']} → {f['step']}"
            break
    if not where:
        where = LABEL[(failed_required or failed_optional)[0]]

    if failed_required:
        title = f"❌ Release {tag} failed at: {where}"
        lead = f"The release workflow for **{tag}** stopped."
    else:
        title = f"⚠️ Release {tag} is published, but the .deb failed at: {where}"
        lead = "**The release is published. Only the .deb is missing.** Everything else (the AppImage, the tarball, the AUR packages and the COPR build) is fine."

    body = (f"{lead}\n\n**Run:** {run_url}\n\n{table}\n\n"
            + recovery_section(tag, failed_required, failed_optional, results)
            + "\n---\n## 📄 Error details\n\n" + "\n".join(details))[:60000]
    summary = (f"# {'❌ Release ' + tag + ' failed' if failed_required else '⚠️ Release ' + tag + ' is published; the .deb failed'}"
               f"\n\n{table}\n\n" + "\n".join(details))
    return title, body, summary, 1


# ---------------------------------------------------------------- GitHub side

def gh(*args, check=True):
    r = subprocess.run(["gh", *args], capture_output=True, text=True)
    if check and r.returncode != 0:
        print(f"gh {' '.join(args)} failed: {r.stderr}", file=sys.stderr)
        return ""
    return r.stdout


TS = re.compile(r"^\d{4}-\d\d-\d\dT[\d:.]+Z ")


def log_excerpt(repo, job_id):
    text = gh("api", f"repos/{repo}/actions/jobs/{job_id}/logs", check=False)
    lines = [TS.sub("", l) for l in text.splitlines()]
    errors = [l for l in lines if "##[error]" in l or l.startswith("::error") or "error:" in l.lower()][:15]
    clean = lambda ls: "\n".join(l.replace("```", "'''") for l in ls)  # noqa: E731
    return clean(errors), clean(lines[-60:])


def api_jobs(repo, run_id):
    out, page = [], 1
    while True:
        data = gh("api", f"repos/{repo}/actions/runs/{run_id}/jobs?per_page=100&page={page}")
        if not data:
            break
        jobs = json.loads(data).get("jobs", [])
        out += jobs
        if len(jobs) < 100:
            break
        page += 1
    return out


def main():
    repo = os.environ["GITHUB_REPOSITORY"]
    run_id = os.environ["GITHUB_RUN_ID"]
    tag = os.environ["GITHUB_REF_NAME"]
    run_url = f"{os.environ['GITHUB_SERVER_URL']}/{repo}/actions/runs/{run_id}"
    needs = json.loads(os.environ["NEEDS_JSON"])
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY", "/dev/stdout")
    results = {jid: info.get("result", "skipped") for jid, info in needs.items()}

    failures = []
    if any(r == "failure" for r in results.values()):
        for job in api_jobs(repo, run_id):
            if job.get("conclusion") != "failure":
                continue
            step = next((s["name"] for s in job.get("steps", []) if s.get("conclusion") == "failure"), "(unknown step)")
            errors, tail = log_excerpt(repo, job["id"])
            failures.append({"name": job["name"], "step": step, "url": job["html_url"], "errors": errors, "tail": tail})

    title, body, summary, code = build_report(tag, run_url, results, failures)
    with open(summary_path, "a") as f:
        f.write(summary)

    if code == 0:
        # close failure issues from an earlier attempt at this tag, if any
        issues = gh("issue", "list", "--state", "open", "--search", f'"Release {tag}" in:title',
                    "--json", "number,title", check=False)
        for i in json.loads(issues or "[]"):
            if i["title"].startswith((f"❌ Release {tag} failed", f"⚠️ Release {tag} is published")):
                gh("issue", "close", str(i["number"]), "--comment", f"Fixed: {run_url} succeeded.", check=False)
        return 0
    gh("issue", "create", "--title", title, "--body", body)
    print(title)
    return code  # keep the run red


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Last job of release.yml: summarise every job; on failure, open an issue
naming the job + step that broke, with the error lines and the end of its log.
On success, close any failure issue left open for this tag."""
import json, os, re, subprocess, sys

REPO   = os.environ["GITHUB_REPOSITORY"]
RUN_ID = os.environ["GITHUB_RUN_ID"]
TAG    = os.environ["GITHUB_REF_NAME"]
RUN_URL = f"{os.environ['GITHUB_SERVER_URL']}/{REPO}/actions/runs/{RUN_ID}"
NEEDS  = json.loads(os.environ["NEEDS_JSON"])
SUMMARY = os.environ.get("GITHUB_STEP_SUMMARY", "/dev/stdout")
ICON = {"success": "✅", "failure": "❌", "cancelled": "⚪", "skipped": "⏭️"}
ORDER = {  # job id in release.yml -> what it means
    "wait-for-ci":     "Wait for CI to pass",
    "prepare":         "Bump version + changelog",
    "build-tarball":   "Test + build binary tarball",
    "build-appimage":  "Build + smoke-test AppImage",
    "publish-release": "Publish GitHub release",
    "trigger-copr":    "Start COPR build",
    "aur-bin":         "Deploy archtoys-bin to AUR",
    "aur-source":      "Deploy archtoys to AUR",
    "rpms":            "Attach COPR RPMs",
}

def gh(*args, check=True):
    r = subprocess.run(["gh", *args], capture_output=True, text=True)
    if check and r.returncode != 0:
        print(f"gh {' '.join(args)} failed: {r.stderr}", file=sys.stderr)
        return ""
    return r.stdout

def api_jobs():
    out, page = [], 1
    while True:
        data = gh("api", f"repos/{REPO}/actions/runs/{RUN_ID}/jobs?per_page=100&page={page}")
        if not data:
            break
        jobs = json.loads(data).get("jobs", [])
        out += jobs
        if len(jobs) < 100:
            break
        page += 1
    return out

TS = re.compile(r"^\d{4}-\d\d-\d\dT[\d:.]+Z ")
def log_excerpt(job_id):
    text = gh("api", f"repos/{REPO}/actions/jobs/{job_id}/logs", check=False)
    lines = [TS.sub("", l) for l in text.splitlines()]
    errors = [l for l in lines if "##[error]" in l or l.startswith("::error") or "error:" in l.lower()][:15]
    tail = lines[-60:]
    clean = lambda ls: "\n".join(l.replace("```", "'''") for l in ls)
    return clean(errors), clean(tail)

# ---- step-by-step recovery instructions for the issue ----------------------

WHAT_HAPPENED = {
    "wait-for-ci":     "CI didn't pass for the commit you tagged (or never ran for it), so the release stopped at the very start.",
    "prepare":         "The release stopped while checking the tag and bumping the version. The error lines below say exactly why (for example, the tag isn't on your newest commit).",
    "build-tarball":   "The app failed to test or build (or failed the glibc check).",
    "build-appimage":  "The AppImage failed to build, failed the glibc check, or crashed in the smoke test.",
    "publish-release": "Creating the GitHub release failed.",
    "trigger-copr":    "COPR refused the webhook (usually a wrong or missing COPR_WEBHOOK_URL secret).",
    "aur-bin":         "Pushing archtoys-bin to the AUR failed (usually the AUR SSH key or a PKGBUILD problem).",
    "aur-source":      "Pushing archtoys to the AUR failed (usually the AUR SSH key or a PKGBUILD problem).",
    "rpms":            "Attaching the COPR RPMs failed (usually the COPR build failed, or took longer than 90 minutes).",
}


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


def recovery_section(tag, failed, results):
    first = failed[0]
    bumped = results.get("prepare") == "success"
    published = results.get("publish-release") == "success"

    out = ["## 🛠️ How to fix it and retry", "", f"**What happened:** {WHAT_HAPPENED.get(first, 'A step failed.')}", ""]

    state = []
    state.append("✅ the version bump commit (\"Release " + tag + "\") was made and the tag moved onto it" if bumped
                 else "❌ the version was **not** bumped, nothing was committed")
    state.append("⚠️ the GitHub release **is published** (people can download it)" if published
                 else "❌ nothing was published: no GitHub release, AUR or RPM update")
    out += ["**What was already changed:**", ""] + [f"- {line}" for line in state] + [""]

    if not published:
        if first == "wait-for-ci":
            out += [
                "**First:** open the CI run (link in the error below) and read why it failed. "
                "If CI never ran, you probably pushed only the tag: just push your commits (`git push`), "
                "wait for CI to go green, then click **Re-run all jobs** on this release run. "
                "Otherwise, fix the problem and retry:",
                "",
            ]
        elif first == "prepare":
            out += ["**First:** read the error lines below; they say what to change. Then retry:", ""]
        else:
            out += [
                "**First:** read the error lines below. If it looks like a one-off hiccup (network, a "
                "download timing out), just click **Re-run failed jobs** on the run. If something in "
                "the code needs fixing, retry with the same tag:",
                "",
            ]
        out += [f"**Retry with the same tag ({tag}):**", "", retry_same_tag(tag, bumped)]
        if bumped:
            out += [
                "The version files already say the new version, so the retry won't bump them again, "
                "and it rewrites this version's changelog entry so your fix is listed too.",
                "",
            ]
        out += ["Then watch the new run in the **Actions** tab. This issue closes itself when it succeeds.", ""]
        return "\n".join(out)

    # Already published: don't reuse the tag, people may have downloaded it.
    rerun = "Open this run (link at the top) and click **Re-run failed jobs**. It continues from where it stopped and doesn't publish anything twice."
    if first == "trigger-copr":
        steps = [
            "1. Check the **COPR_WEBHOOK_URL** secret: GitHub → Settings → Secrets and variables → Actions. "
            "It must be COPR's *custom* webhook URL, ending in `/archtoys/`.",
            f"2. {rerun}",
        ]
    elif first in ("aur-bin", "aur-source"):
        steps = [
            "1. If the error mentions `Permission denied (publickey)`: the AUR key in the secret "
            "**AUR_SSH_PRIVATE_KEY** isn't registered on your AUR account. Add the matching public key "
            "on aur.archlinux.org → My Account, then go to step 2.",
            f"2. {rerun}",
            "3. If the **PKGBUILD itself** needs fixing: fix it, commit and push, then release the **next** "
            "version (see below). A re-run would still use the old PKGBUILD.",
        ]
    elif first == "rpms":
        steps = [
            "1. Open your COPR project → **Builds** and look at the build for this version.",
            "2. If it **failed**: open its log, fix the problem (often in `archtoys.spec`), commit and push, then "
            "click **Rebuild** on COPR.",
            "3. If it's **still running** or succeeded later: just wait for it to finish.",
            f"4. Then attach the RPMs by hand: **Actions → Upload COPR RPMs to GitHub Release → Run workflow**, "
            f"type `{tag}`, and click **Run workflow**.",
        ]
    else:
        steps = [f"1. {rerun}"]

    out += ["**What to do:**", ""] + steps + [""]
    next_ver = tag
    m = re.match(r"^v(\d+)\.(\d+)\.(\d+)$", tag)
    if m:
        next_ver = f"v{m.group(1)}.{m.group(2)}.{int(m.group(3)) + 1}"
    out += [
        "**If the code needs changing:** because this version is already published, don't reuse "
        f"{tag}. Fix it and release the next version instead:",
        "",
        "```bash\n"
        "cd ~/Archtoys\n"
        "git pull\n"
        "git fetch --tags --force\n"
        "# fix the problem, then:\n"
        "git add -A\n"
        "git commit -m \"Describe your fix here\"\n"
        f"git tag {next_ver}\n"
        f"git push origin main {next_ver}\n"
        "```",
        "",
    ]
    return "\n".join(out)


results = {jid: info.get("result", "skipped") for jid, info in NEEDS.items()}
failed  = [j for j in ORDER if results.get(j) == "failure"]
version = (NEEDS.get("prepare", {}).get("outputs") or {}).get("version") or TAG.lstrip("v")

table = ["| | Step | Result |", "|---|---|---|"]
for jid, label in ORDER.items():
    r = results.get(jid, "skipped")
    table.append(f"| {ICON.get(r, '❔')} | {label} | {r} |")
table = "\n".join(table)

if not failed:
    with open(SUMMARY, "a") as f:
        f.write(f"# ✅ Release {TAG} finished\n\n{table}\n")
    # close a failure issue from an earlier attempt at this tag, if any
    issues = gh("issue", "list", "--state", "open", "--search", f'"Release {TAG} failed" in:title',
                "--json", "number", check=False)
    for i in json.loads(issues or "[]"):
        gh("issue", "close", str(i["number"]), "--comment", f"Fixed: {RUN_URL} succeeded.", check=False)
    sys.exit(0)

# ---- failure: find the exact job/step and pull its log ----------------------
jobs = api_jobs()
details, first_where = [], None
for job in jobs:
    if job.get("conclusion") != "failure":
        continue
    step = next((s["name"] for s in job.get("steps", []) if s.get("conclusion") == "failure"), "(unknown step)")
    where = f"{job['name']} → {step}"
    first_where = first_where or where
    errors, tail = log_excerpt(job["id"])
    details.append(
        f"### ❌ {where}\n\n[Open this job's full log]({job['html_url']})\n\n"
        + (f"**Error lines:**\n```\n{errors}\n```\n\n" if errors else "")
        + f"<details><summary>Last 60 log lines</summary>\n\n```\n{tail}\n```\n</details>\n"
    )

title = f"❌ Release {TAG} failed at: {first_where or failed[0]}"
body = (
    f"The release workflow for **{TAG}** stopped.\n\n"
    f"**Run:** {RUN_URL}\n\n{table}\n\n"
    + recovery_section(TAG, failed, results)
    + "\n---\n## 📄 Error details\n\n" + "\n".join(details)
)
body = body[:60000]
with open(SUMMARY, "a") as f:
    f.write(f"# ❌ Release {TAG} failed\n\n{table}\n\n" + "\n".join(details))
gh("issue", "create", "--title", title, "--body", body)
print(title)
sys.exit(1)  # keep the run red

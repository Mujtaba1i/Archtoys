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
    f"**Run:** {RUN_URL}\n\n{table}\n\n" + "\n".join(details) +
    "\n---\n**What next:** fix the problem, then either click **Re-run failed jobs** on the run "
    "(if nothing in the code needs to change), or commit the fix and release the next version. "
    "This issue closes itself when a run for this tag succeeds."
)
body = body[:60000]
with open(SUMMARY, "a") as f:
    f.write(f"# ❌ Release {TAG} failed\n\n{table}\n\n" + "\n".join(details))
gh("issue", "create", "--title", title, "--body", body)
print(title)
sys.exit(1)  # keep the run red

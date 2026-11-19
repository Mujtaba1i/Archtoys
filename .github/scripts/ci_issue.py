#!/usr/bin/env python3
"""CI's last job on a push to main (and the last step of other automations,
with ISSUE_TITLE set): when a job failed, opens an issue that
says which jobs and steps failed, the lines of the log that say why, and
links to them. One issue at a time: a later failure adds a comment to it,
and the next run where everything passes closes it.

    RESULTS='${{ toJSON(needs) }}' GH_TOKEN=... python3 .github/scripts/ci_issue.py

Needs the gh CLI (on GitHub's runners) and the job's permissions to read
Actions and write issues.
"""
import json
import os
import re
import subprocess
import sys

TITLE = os.environ.get("ISSUE_TITLE") or "CI failed on main"
MARKER = "<!-- archtoys-ci-failure -->"
# the lines of a failed job's log worth putting in the issue
WHY = re.compile(r"\[FAIL\]|##\[error\]|error(\[E\d+\])?:|panicked at|FAILED|Traceback|Error:")


def gh(*args, check=True):
    return subprocess.run(["gh", *args], capture_output=True, text=True, check=check).stdout


def failed_jobs(results):
    """The jobs (by id) that failed. Cancelled ones (a newer push) and skipped ones aren't failures."""
    return sorted(job for job, r in results.items() if r.get("result") == "failure")


def why_lines(log, keep=12):
    """The log lines that say what failed, without timestamps, at most `keep`."""
    lines = []
    for line in log.splitlines():
        text = re.sub(r"^\S+Z ", "", line).strip()
        if WHY.search(text) and text not in lines:
            lines.append(text[:300])
    return lines[:keep]


def body(run_url, sha, jobs):
    """The issue (or comment): each failed job, its failed steps and why."""
    out = [MARKER, f"{TITLE}, commit `{sha[:7]}`: [the run]({run_url}).", ""]
    for job in jobs:
        out.append(f"### ❌ [{job['name']}]({job['url']})")
        steps = job.get("steps") or []
        if steps:
            out.append("Failed step" + ("s" if len(steps) > 1 else "") + ": " + ", ".join(f"**{s}**" for s in steps))
        if job.get("why"):
            out += ["", "```", *job["why"], "```"]
        out.append("")
    out.append("This issue closes itself when it passes again.")
    return "\n".join(out)


def main():
    repo, run_id, sha = os.environ["GITHUB_REPOSITORY"], os.environ["GITHUB_RUN_ID"], os.environ["GITHUB_SHA"]
    run_url = f"{os.environ.get('GITHUB_SERVER_URL', 'https://github.com')}/{repo}/actions/runs/{run_id}"
    results = json.loads(os.environ.get("RESULTS", "{}"))
    open_issue = gh("issue", "list", "--repo", repo, "--state", "open", "--search", f'"{TITLE}" in:title',
                    "--json", "number,title", "--jq", f'.[] | select(.title == "{TITLE}") | .number').split()

    if not failed_jobs(results):
        if open_issue:
            gh("issue", "close", open_issue[0], "--repo", repo, "--comment", f"It passed again (`{sha[:7]}`, [the run]({run_url})).")
            print(f"passed: closed issue #{open_issue[0]}")
        return 0

    jobs = []
    for job in json.loads(gh("api", f"repos/{repo}/actions/runs/{run_id}/jobs?per_page=100"))["jobs"]:
        steps = [s["name"] for s in job.get("steps", []) if s.get("conclusion") == "failure"]
        # a finished job that failed, or (reporting from inside it) one still running with a failed step
        if job.get("conclusion") != "failure" and not (job.get("status") == "in_progress" and steps):
            continue
        log = gh("api", f"repos/{repo}/actions/jobs/{job['id']}/logs", check=False)
        jobs.append({"name": job["name"], "url": job["html_url"], "steps": steps, "why": why_lines(log)})
    text = body(run_url, sha, jobs)
    if open_issue:
        gh("issue", "comment", open_issue[0], "--repo", repo, "--body", text)
        print(f"CI failed again: comment on issue #{open_issue[0]}")
    else:
        url = gh("issue", "create", "--repo", repo, "--title", TITLE, "--body", text).strip()
        print(f"CI failed: opened {url}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

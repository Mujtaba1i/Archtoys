#!/usr/bin/env python3
"""COPR helper for the release workflow.

  copr_watch.py trigger   check COPR's queue, then send the build webhook
  copr_watch.py watch     follow COPR's builds for a tag until the RPMs are
                          attached to the GitHub release (or give up)
  copr_watch.py crashed   tell the user the watcher itself crashed

COPR cannot call us back, so we ask it (its public API needs no login).
Everything the watcher needs to remember lives in COPR, in the GitHub release
(which RPMs are attached) and in one GitHub issue per tag, so it can be
restarted at any time. A long wait is a relay of runs that each last under
an hour; every run starts the next one for the same tag.

Settings come from environment variables (see Cfg); the time limits can be
shortened for tests.
"""
import datetime
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

ACTIVE = {"pending", "starting", "running", "importing", "waiting"}
GOOD = {"succeeded", "forked"}
RELAY_WORKFLOW = "copr-watch.yml"
CHECK_WORKFLOW = "package-check.yml"


class Cfg:
    def __init__(self, env=None):
        g = (env if env is not None else os.environ).get
        self.api = g("COPR_API", "https://copr.fedorainfracloud.org").rstrip("/")
        self.owner = g("COPR_OWNER", "mujtaba1i")
        self.project = g("COPR_PROJECT", "archtoys")
        self.package = g("COPR_PACKAGE", "archtoys")
        self.repo = g("GITHUB_REPOSITORY", "Mujtaba1i/Archtoys")
        self.tag = g("TAG", "")
        self.version = g("VERSION") or self.tag.lstrip("v")
        self.sha = g("SHA", "")
        self.poll = float(g("POLL_SECONDS", 120))              # ask COPR every 2 minutes
        self.run_budget = float(g("RUN_BUDGET_SECONDS", 50 * 60))  # one run lasts under an hour
        self.max_total = float(g("MAX_TOTAL_SECONDS", 7 * 3600))   # then give up
        self.grace_no_build = float(g("GRACE_NO_BUILD_SECONDS", 15 * 60))
        self.grace_rebuild = float(g("GRACE_REBUILD_SECONDS", 45 * 60))
        self.outage_limit = float(g("OUTAGE_LIMIT_SECONDS", 3600))
        self.optional_re = g("OPTIONAL_CHROOTS", "rawhide")
        self.started = float(g("STARTED") or time.time())      # when watching began
        self.retry_sleep = float(g("HTTP_RETRY_SLEEP", 2))
        self.issue_title = f"COPR needs attention for {self.tag}"

    @property
    def hours(self):
        h = self.max_total / 3600
        return f"{h:g} hours"

    def build_url(self, build_id):
        return f"{self.api}/coprs/{self.owner}/{self.project}/build/{build_id}/"

    def project_url(self):
        return f"{self.api}/coprs/{self.owner}/{self.project}/"

    def is_optional(self, chroot):
        return bool(re.search(self.optional_re, chroot))


class CoprUnavailable(Exception):
    pass


class GhError(Exception):
    pass


# ----------------------------------------------------------- talking to COPR

def http_json(cfg, url, retries=3):
    last = None
    for attempt in range(retries):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "archtoys-release-watcher"})
            with urllib.request.urlopen(req, timeout=30) as r:
                return json.load(r)
        except Exception as err:  # network errors, 5xx, bad JSON
            last = err
            if attempt + 1 < retries:
                time.sleep(cfg.retry_sleep * (attempt + 1))
    raise CoprUnavailable(str(last))


def list_builds(cfg):
    q = urllib.parse.urlencode({
        "ownername": cfg.owner, "projectname": cfg.project, "packagename": cfg.package,
        "limit": 30, "order": "id", "order_type": "DESC",
    })
    return http_json(cfg, f"{cfg.api}/api_3/build/list?{q}").get("items", [])


def list_build_chroots(cfg, build_id):
    """[{name, state, ended_on}] for one build; [] if COPR doesn't tell us."""
    q = urllib.parse.urlencode({"build_id": build_id, "limit": 50})
    try:
        items = http_json(cfg, f"{cfg.api}/api_3/build-chroot/list?{q}").get("items", [])
    except CoprUnavailable:
        return []
    out = []
    for item in items:
        name = item.get("name") or item.get("chroot_name") or item.get("mock_chroot")
        if name and item.get("state"):
            out.append({"name": name, "state": item["state"], "ended_on": item.get("ended_on")})
    return out


# ------------------------------------------------------- reading the builds

def build_version(b):
    v = (b.get("source_package") or {}).get("version") or ""
    return v.split(":")[-1]  # drop an epoch


def version_matches(v, version):
    return v == version or v.startswith(version + "-")


def submitted(b):
    return float(b.get("submitted_on") or 0)


def split_builds(builds, version, since):
    """(this release's builds, builds for another version), oldest first.

    Builds submitted before the release commit existed belong to an earlier
    attempt (even for the same version number) and are ignored. A brand-new
    build whose version COPR doesn't know yet counts as ours until it does."""
    mine, wrong = [], []
    for b in sorted(builds, key=lambda b: b["id"]):
        if submitted(b) < since:
            continue
        v = build_version(b)
        if not v or version_matches(v, version):
            mine.append(b)
        else:
            wrong.append(b)
    return mine, wrong


def kind_of(state):
    if state in GOOD:
        return "succeeded"
    if state in ACTIVE:
        return "in_flight"
    if state == "failed":
        return "failed"
    if state == "canceled":
        return "canceled"
    return "other"  # skipped, unknown


def _rank(kind):
    return {"succeeded": 3, "in_flight": 2}.get(kind, 1)


def consolidate(per_build):
    """per_build: [(build, [{name,state,ended_on}])], oldest first.
    -> {chroot: {kind, build, ended, state}}  (the best outcome per chroot)."""
    out = {}
    for b, chroots in per_build:
        if not chroots:  # COPR gave no per-chroot detail: use the build as a whole
            chroots = [{"name": n, "state": b.get("state", "unknown"), "ended_on": b.get("ended_on")}
                       for n in (b.get("chroots") or [])]
        for c in chroots:
            cand = {"kind": kind_of(c["state"]), "build": b["id"],
                    "ended": float(c.get("ended_on") or b.get("ended_on") or 0), "state": c["state"]}
            cur = out.get(c["name"])
            if cur is None or (_rank(cand["kind"]), cand["build"]) >= (_rank(cur["kind"]), cur["build"]):
                out[c["name"]] = cand
    return out


# ------------------------------------------------------------ the decisions

def event(cfg, key, headline, body):
    return {"key": key, "headline": headline, "body": body}


REBUILD_HELP = (
    "To rebuild: open [the COPR project]({project}) and use **Rebuild** (or **Resubmit** on the failed "
    "build). COPR builds your newest `main`, so push a fix first if the code needs one. "
    "I keep watching and attach the RPMs by myself."
)


def completion(cfg, chroots, attached):
    """(complete, all_attached). Complete = nothing left to wait for: every
    chroot is attached, or is optional and failed (we only tell you about it)."""
    if not chroots:
        return False, False
    all_attached = all(n in attached for n in chroots)
    complete = all(
        n in attached or (cfg.is_optional(n) and c["kind"] in ("failed", "canceled", "other"))
        for n, c in chroots.items())
    return complete, all_attached


def decide(cfg, now, since, mine, wrong, chroots, attached):
    """Pure function: what to do now.

    mine/wrong: build lists from split_builds; chroots: from consolidate;
    attached: chroots whose RPMs are already on the release.
    Returns {attach: [(chroot, build)], events: [...], complete, all_attached}."""
    plan = {"attach": [], "events": [], "complete": False, "all_attached": False}
    project = cfg.project_url()
    elapsed = now - since

    if not mine:
        if elapsed >= cfg.grace_no_build:
            minutes = int(elapsed // 60)
            plan["events"].append(event(
                cfg, "no-build",
                f"No COPR build for {cfg.version} is queued or running",
                f"{minutes} minutes after the release, COPR has no build for {cfg.version}. Either it was "
                f"cancelled, or COPR never received the webhook.\n\n" + REBUILD_HELP.format(project=project)))
        return plan

    waiting_for_rebuild = False
    for name, c in sorted(chroots.items()):
        optional = cfg.is_optional(name)
        label = f"{name} (optional)" if optional else name
        url = cfg.build_url(c["build"])
        age = now - c["ended"] if c["ended"] else 0
        if c["kind"] == "succeeded" and name not in attached:
            plan["attach"].append((name, c["build"]))
        elif c["kind"] in ("failed", "other"):
            waiting_for_rebuild = waiting_for_rebuild or not optional
            tail = ("This one is optional, so the release still counts as complete. "
                    if optional else "")
            plan["events"].append(event(
                cfg, f"failed:{name}:{c['build']}",
                f"COPR build failed for {label}",
                f"The build for **{name}** ended as `{c['state']}`: {url}\n\n{tail}"
                + REBUILD_HELP.format(project=project)))
            if c["ended"] and age >= cfg.grace_rebuild:
                plan["events"].append(event(
                    cfg, f"no-rebuild:{name}:{c['build']}",
                    f"No rebuild found for {label}",
                    f"{int(age // 60)} minutes after it failed, there is still no new build for **{name}**. "
                    + REBUILD_HELP.format(project=project)))
        elif c["kind"] == "canceled":
            waiting_for_rebuild = waiting_for_rebuild or not optional
            if c["ended"] and age >= cfg.grace_rebuild:
                plan["events"].append(event(
                    cfg, f"no-rebuild:{name}:{c['build']}",
                    f"No build is queued or running for {label}",
                    f"The build for **{name}** was cancelled {int(age // 60)} minutes ago and nothing "
                    f"replaced it: {url}\n\n" + REBUILD_HELP.format(project=project)))

    newest_mine = max(b["id"] for b in mine)
    for b in wrong:
        if b["id"] > newest_mine and (waiting_for_rebuild or not chroots):
            plan["events"].append(event(
                cfg, f"wrong-version:{b['id']}",
                f"COPR built {build_version(b)}, not {cfg.version}",
                f"The newer build #{b['id']} is for version **{build_version(b)}**, so I did not use it: "
                f"{cfg.build_url(b['id'])}\n\nRebuild the version {cfg.version} release. I keep waiting."))

    # what it will look like once the planned attachments succeed
    after = set(attached) | {n for n, _ in plan["attach"]}
    plan["complete"], plan["all_attached"] = completion(cfg, chroots, after)
    return plan


# ------------------------------------------------------------- GitHub side

def gh(args, check=True):
    r = subprocess.run(["gh", *args], capture_output=True, text=True)
    if check and r.returncode != 0:
        raise GhError(f"gh {' '.join(args[:3])} failed: {r.stderr.strip()[:300]}")
    return r.stdout


def iso_to_epoch(text):
    return datetime.datetime.fromisoformat(text.strip().replace("Z", "+00:00")).timestamp()


def tag_sha(cfg):
    out = gh(["api", f"repos/{cfg.repo}/git/ref/tags/{cfg.tag}", "--jq", ".object.sha"], check=False).strip()
    return out or None


def commit_epoch(cfg, sha):
    return iso_to_epoch(gh(["api", f"repos/{cfg.repo}/commits/{sha}", "--jq", ".commit.committer.date"]))


def attached_chroots(cfg, chroot_names):
    out = gh(["release", "view", cfg.tag, "--repo", cfg.repo, "--json", "assets", "--jq", ".assets[].name"])
    names = out.split()
    return {c for c in chroot_names if any(n.startswith(c + "-") and n.endswith(".rpm") for n in names)}


def attach_chroot(cfg, chroot, build_id):
    """Download one chroot's RPMs from COPR and upload them to the release."""
    work = tempfile.mkdtemp(prefix="copr-")
    r = subprocess.run(["copr-cli", "download-build", str(build_id), "--chroot", chroot, "--rpms", "--dest", work],
                       capture_output=True, text=True)
    if r.returncode != 0:
        raise GhError(f"copr-cli failed for {chroot}: {r.stderr.strip()[:300]}")
    files = []
    for p in sorted(Path(work).rglob("*.rpm")):
        n = p.name
        if "debuginfo" in n or "debugsource" in n or n.endswith(".src.rpm"):
            continue
        target = Path(work) / f"{chroot}-{n}"
        shutil.copy(p, target)
        files.append(str(target))
    if not files:
        raise GhError(f"the build for {chroot} had no binary RPMs")
    gh(["release", "upload", cfg.tag, *files, "--repo", cfg.repo, "--clobber"])
    return [Path(f).name for f in files]


class Issue:
    """The single issue for this tag. Events are comments; each carries a
    hidden marker so it is never posted twice, even across relay runs."""

    def __init__(self, cfg):
        self.cfg = cfg
        self.number = None
        self.state = None
        self._loaded = False

    def _find(self):
        # A plain list, not a search: GitHub's search index can lag by minutes.
        out = gh(["issue", "list", "--repo", self.cfg.repo, "--state", "all", "--limit", "100",
                  "--json", "number,state,title"], check=False)
        try:
            items = [i for i in json.loads(out or "[]") if i["title"] == self.cfg.issue_title]
        except ValueError:
            items = []
        items.sort(key=lambda i: (i["state"] == "OPEN", i["number"]), reverse=True)
        if items:
            self.number, self.state = items[0]["number"], items[0]["state"]

    def load(self):
        if not self._loaded:
            self._find()
            self._loaded = True

    def text(self):
        if self.number is None:
            return ""
        out = gh(["issue", "view", str(self.number), "--repo", self.cfg.repo, "--json", "body,comments"])
        data = json.loads(out)
        return (data.get("body") or "") + "\n" + "\n".join(c.get("body") or "" for c in data.get("comments", []))

    def post(self, ev):
        self.load()
        marker = f"<!-- copr-event:{ev['key']} -->"
        if marker in self.text():
            return False
        body = f"**{ev['headline']}**\n\n{ev['body']}\n\n{marker}"
        if self.number is None:
            intro = (f"The COPR watcher for **{self.cfg.tag}** needs your attention. Later events are added "
                     f"as comments, and this issue closes itself when every RPM is attached.\n\n"
                     f"<!-- copr-watch:{self.cfg.tag} -->\n\n")
            out = gh(["issue", "create", "--repo", self.cfg.repo, "--title", self.cfg.issue_title,
                      "--body", intro + body]).strip()
            m = re.search(r"/issues/(\d+)", out)
            self.number = int(m.group(1)) if m else None
            self.state = "OPEN"
        else:
            if self.state != "OPEN":
                gh(["issue", "reopen", str(self.number), "--repo", self.cfg.repo])
                self.state = "OPEN"
            gh(["issue", "comment", str(self.number), "--repo", self.cfg.repo, "--body", body])
        return True

    def close(self, headline, body):
        self.load()
        if self.number is None or self.state != "OPEN":
            return
        self.post({"key": "done", "headline": headline, "body": body})
        gh(["issue", "close", str(self.number), "--repo", self.cfg.repo])
        self.state = "CLOSED"

    def exists_open(self):
        self.load()
        return self.number is not None and self.state == "OPEN"


def summary(line):
    path = os.environ.get("GITHUB_STEP_SUMMARY")
    print(line)
    if path:
        with open(path, "a") as f:
            f.write(line + "\n")


# ---------------------------------------------------------------- trigger

def post_webhook(cfg, url):
    last = ""
    for attempt in range(3):
        try:
            req = urllib.request.Request(url, method="POST", data=b"")
            with urllib.request.urlopen(req, timeout=30) as r:
                return r.status, r.read(300).decode("utf-8", "replace")
        except urllib.error.HTTPError as err:
            last = f"HTTP {err.code}: {err.read(300).decode('utf-8', 'replace')}"
            if err.code < 500:
                return err.code, last
        except Exception as err:
            last = str(err)
        time.sleep(cfg.retry_sleep * (attempt + 1))
    return 599, last


def trigger(cfg):
    url = os.environ.get("COPR_WEBHOOK_URL", "")
    if not url:
        print("::error title=COPR not started::Secret COPR_WEBHOOK_URL is missing "
              "(Settings, Secrets and variables, Actions).")
        return 1
    note = ""
    try:
        since = commit_epoch(cfg, cfg.sha) - 60 if cfg.sha else time.time() - 3600
        builds = list_builds(cfg)
        active = [b for b in builds if b.get("state") in ACTIVE]
        # a build of THIS release: made after the release commit, same version
        active_mine = [b for b in active if submitted(b) >= since and build_version(b)
                       and version_matches(build_version(b), cfg.version)]
        # anything else in the queue (an older version, an earlier attempt, a
        # build whose version COPR doesn't know yet): ours waits behind it
        active_other = [b for b in active if b not in active_mine]
        if active_mine:
            b = active_mine[-1]
            summary(f"COPR: build #{b['id']} for {cfg.version} is already {b['state']}. "
                    f"No second build is started; the watcher follows that one. {cfg.build_url(b['id'])}")
            return 0
        if active_other:
            b = max(active_other, key=lambda b: b["id"])
            note = (f" It will wait in COPR's queue behind build #{b['id']} "
                    f"({build_version(b) or 'version not known yet'}, {b['state']}).")
    except (CoprUnavailable, GhError, ValueError) as err:
        print(f"::warning::Could not check COPR's queue first ({err}); sending the webhook anyway.")
    status, text = post_webhook(cfg, url)
    if 200 <= status < 300:
        summary(f"COPR: build requested (HTTP {status}).{note}")
        return 0
    print(f"::error title=COPR refused the webhook::HTTP {status}: {text}. Check the COPR_WEBHOOK_URL secret.")
    return 1


# ------------------------------------------------------------------ watch

def dispatch_next(cfg, sha):
    args = ["workflow", "run", RELAY_WORKFLOW, "--repo", cfg.repo, "--ref", os.environ.get("RELAY_REF", "main"),
            "-f", f"tag={cfg.tag}", "-f", f"sha={sha}", "-f", f"started={int(cfg.started)}"]
    gh(args)


def start_package_check(cfg):
    """Every package exists now, so test them on clean systems."""
    try:
        gh(["workflow", "run", CHECK_WORKFLOW, "--repo", cfg.repo, "--ref", os.environ.get("RELAY_REF", "main"),
            "-f", f"tag={cfg.tag}"])
        summary(f"Started the package check for {cfg.tag}.")
    except GhError as err:
        summary(f"Could not start the package check ({err}). Run **Actions, Package check, Run workflow** "
                f"with `{cfg.tag}`.")


def watch(cfg):
    run_start = time.time()
    issue = Issue(cfg)
    sha = cfg.sha or tag_sha(cfg)
    if not sha:
        summary(f"The tag {cfg.tag} no longer exists, so there is nothing to follow.")
        return 0
    try:
        since = commit_epoch(cfg, sha) - 60
    except (GhError, ValueError):
        since = cfg.started - 3600
    down_since = None
    attach_failures = {}

    while True:
        now = time.time()
        current = tag_sha(cfg)
        if current != sha:
            summary(f"The tag {cfg.tag} was moved or deleted (it was {sha[:7]}, now "
                    f"{(current or 'gone')[:7]}). A newer release owns it, so I stop.")
            return 0

        try:
            builds = list_builds(cfg)
            mine, wrong = split_builds(builds, cfg.version, since)
            per_build = [(b, list_build_chroots(cfg, b["id"])) for b in mine]
            chroots = consolidate(per_build)
            down_since = None
            if mine and not chroots:
                # COPR answered, but not in the shape we expect: keep the raw data in the log
                print("COPR's build data has no chroot details; newest build: " + json.dumps(mine[-1])[:800])
        except CoprUnavailable as err:
            down_since = down_since or now
            print(f"COPR could not be reached ({err}); retrying.")
            if now - down_since >= cfg.outage_limit:
                issue.post({"key": "copr-down", "headline": "COPR cannot be reached",
                            "body": f"COPR's website or API has been unreachable for {int((now - down_since) // 60)} "
                                    f"minutes. Check https://status.fedoraproject.org. I keep trying."})
            builds, mine, wrong, chroots = [], None, [], None

        if chroots is not None:
            attached = attached_chroots(cfg, chroots)
            plan = decide(cfg, now, since, mine, wrong, chroots, attached)

            for name, build_id in plan["attach"]:
                try:
                    files = attach_chroot(cfg, name, build_id)
                    attached.add(name)
                    attach_failures.pop(name, None)
                    summary(f"Attached {len(files)} RPM(s) for {name} from build #{build_id}.")
                except (GhError, OSError) as err:
                    attach_failures[name] = attach_failures.get(name, 0) + 1
                    print(f"Could not attach {name}: {err}")
                    if attach_failures[name] >= 3:
                        issue.post({"key": f"download-failed:{name}:{build_id}",
                                    "headline": f"Could not attach the RPMs for {name}",
                                    "body": f"The build succeeded ({cfg.build_url(build_id)}), but attaching "
                                            f"its RPMs failed 3 times: {err}\n\nI keep trying."})
            for ev in plan["events"]:
                if issue.post(ev):
                    summary(f"Told you: {ev['headline']}")

            # decide() assumed every download works; check what really happened
            complete, all_attached = completion(cfg, chroots, attached)
            if complete:
                after = set(attached)
                if all_attached:
                    issue.close("All RPMs are attached", "Every Fedora build is on the release now, so this issue closes.")
                    summary(f"All RPMs for {cfg.tag} are attached. Done.")
                else:
                    missing = ", ".join(sorted(n for n in chroots if n not in after))
                    issue.post({"key": "complete-optional", "headline": "The release is complete",
                                "body": f"Only the optional part is missing: {missing}. This issue stays open "
                                        f"as a reminder. After a good rebuild, run **Actions, COPR watcher, "
                                        f"Run workflow** with `{cfg.tag}` to attach it and close this."})
                    summary(f"Required RPMs for {cfg.tag} are attached; optional missing: {missing}.")
                start_package_check(cfg)
                return 0

            states = ", ".join(f"{n}={c['kind']}" for n, c in sorted(chroots.items())) or "no build yet"
            print(f"[{int(now - cfg.started)}s] {states}; attached: {sorted(attached) or 'none'}")

        if now - cfg.started >= cfg.max_total:
            unfinished = ", ".join(sorted(n for n, c in (chroots or {}).items() if n not in (attached if chroots else set()))) or "all"
            issue.post({"key": "gave-up", "headline": f"Gave up after {cfg.hours}",
                        "body": f"COPR did not finish within {cfg.hours}. Still missing: {unfinished}.\n\n"
                                f"Check [the COPR project]({cfg.project_url()}). When the build is done, run "
                                f"**Actions, COPR watcher, Run workflow** with `{cfg.tag}`: it attaches the RPMs "
                                f"and closes this issue."})
            summary(f"Gave up after {cfg.hours}.")
            return 0

        if time.time() - run_start >= cfg.run_budget:
            dispatch_next(cfg, sha)
            summary("COPR is still busy; started a fresh watcher run for the same tag.")
            return 0
        time.sleep(cfg.poll)


def crashed(cfg):
    run_url = f"{os.environ.get('GITHUB_SERVER_URL', 'https://github.com')}/{cfg.repo}/actions/runs/{os.environ.get('GITHUB_RUN_ID', '')}"
    Issue(cfg).post({"key": f"crashed:{os.environ.get('GITHUB_RUN_ID', '')}",
                     "headline": "The COPR watcher itself crashed",
                     "body": f"The watcher run stopped unexpectedly: {run_url}\n\nOpen it for the error. To "
                             f"start again: **Actions, COPR watcher, Run workflow** with `{cfg.tag}`."})
    return 0


def main(argv):
    if len(argv) < 2 or argv[1] not in ("trigger", "watch", "crashed"):
        print(__doc__)
        return 2
    cfg = Cfg()
    return {"trigger": trigger, "watch": watch, "crashed": crashed}[argv[1]](cfg)


if __name__ == "__main__":
    sys.exit(main(sys.argv))

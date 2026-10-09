"""Tests for copr_watch.py: every situation in the v0.3.1 plan, with no network.

Run:  python3 -m unittest discover -v -s .github/scripts -p 'test_*.py'
"""
import contextlib
import io
import os
import sys
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(__file__))
import copr_watch as cw  # noqa: E402

SINCE = 1_000_000.0          # when the release commit was made (minus slack)
NOW = SINCE + 3600.0         # "now" in most tests: an hour later
ENV = {"TAG": "v0.3.1", "VERSION": "0.3.1", "COPR_API": "http://copr.test"}
CFG = cw.Cfg(ENV)
F43, F44, F45, RAW = "fedora-43-x86_64", "fedora-44-x86_64", "fedora-45-x86_64", "fedora-rawhide-x86_64"
ALL = [F43, F44, F45, RAW]


def build(bid, state="succeeded", version="0.3.1-1", sub=SINCE + 60, ended=None, chroots=None):
    return {"id": bid, "state": state, "source_package": {"version": version} if version else {},
            "submitted_on": sub, "ended_on": ended, "chroots": chroots or ALL}


def ch(name, state, ended=None):
    return {"name": name, "state": state, "ended_on": ended}


def plan_for(per_build, attached=(), now=NOW, wrong=(), cfg=CFG):
    mine = [b for b, _ in per_build]
    chroots = cw.consolidate(per_build)
    return cw.decide(cfg, now, SINCE, mine, list(wrong), chroots, set(attached))


def quiet(fn, *args):
    """Run fn without letting its output reach the log. The script prints GitHub
    workflow commands such as ::error::, and GitHub would show those as red
    annotations on the CI run even though the test passes."""
    with contextlib.redirect_stdout(io.StringIO()):
        return fn(*args)


def keys(plan):
    return [e["key"] for e in plan["events"]]


class Reading(unittest.TestCase):
    def test_versions_and_epochs(self):
        self.assertEqual(cw.build_version({"source_package": {"version": "1:0.3.1-1"}}), "0.3.1-1")
        self.assertTrue(cw.version_matches("0.3.1-1", "0.3.1"))
        self.assertFalse(cw.version_matches("0.3.10-1", "0.3.1"))   # prefix must end at the dash
        self.assertFalse(cw.version_matches("0.3.2-1", "0.3.1"))

    def test_stale_builds_are_ignored_even_for_the_same_version(self):
        old = build(1, sub=SINCE - 5000)          # an earlier attempt at this version
        new = build(2, sub=SINCE + 30)
        mine, wrong = cw.split_builds([old, new], "0.3.1", SINCE)
        self.assertEqual([b["id"] for b in mine], [2])
        self.assertEqual(wrong, [])

    def test_new_build_with_unknown_version_counts_as_ours(self):
        fresh = build(3, state="importing", version=None, sub=SINCE + 10)
        mine, wrong = cw.split_builds([fresh], "0.3.1", SINCE)
        self.assertEqual([b["id"] for b in mine], [3])

    def test_other_version_goes_to_wrong(self):
        other = build(4, version="0.3.2-1")
        mine, wrong = cw.split_builds([other], "0.3.1", SINCE)
        self.assertEqual((mine, [b["id"] for b in wrong]), ([], [4]))


class Situations(unittest.TestCase):
    """The numbered situations of the plan."""

    def test_01_build_succeeds_attach_everything_and_finish(self):
        p = plan_for([(build(1), [ch(c, "succeeded") for c in ALL])])
        self.assertEqual(sorted(c for c, _ in p["attach"]), sorted(ALL))
        self.assertTrue(p["complete"] and p["all_attached"])
        self.assertEqual(p["events"], [])

    def test_02_still_busy_keeps_waiting_quietly(self):
        p = plan_for([(build(1, "running"), [ch(c, "running") for c in ALL])])
        self.assertEqual((p["attach"], p["events"], p["complete"]), ([], [], False))

    def test_03_cancel_then_rebuild_follows_the_new_build(self):
        p = plan_for([(build(1, "canceled", ended=NOW - 600), [ch(F44, "canceled", NOW - 600)]),
                      (build(2, "pending"), [ch(F44, "pending")])])
        self.assertEqual((p["events"], p["complete"]), ([], False))

    def test_04_cancel_without_rebuild_speaks_up_after_45_minutes(self):
        early = plan_for([(build(1, "canceled"), [ch(F44, "canceled", NOW - 600)])])
        late = plan_for([(build(1, "canceled"), [ch(F44, "canceled", NOW - 2800)])])
        self.assertEqual(keys(early), [])
        self.assertEqual(keys(late), ["no-rebuild:fedora-44-x86_64:1"])
        self.assertIn("No build is queued or running", late["events"][0]["headline"])
        self.assertFalse(late["complete"])

    def test_05_rebuild_after_the_issue_attaches_and_completes(self):
        p = plan_for([(build(1, "canceled"), [ch(c, "canceled", NOW - 3000) for c in ALL]),
                      (build(2), [ch(c, "succeeded") for c in ALL])])
        self.assertTrue(p["complete"] and p["all_attached"])
        self.assertEqual(p["events"], [])

    def test_06_no_build_ever_appeared(self):
        cfg = CFG
        quiet = cw.decide(cfg, SINCE + 300, SINCE, [], [], {}, set())
        loud = cw.decide(cfg, SINCE + 1000, SINCE, [], [], {}, set())
        self.assertEqual(quiet["events"], [])
        self.assertEqual(keys(loud), ["no-build"])
        self.assertFalse(loud["complete"])

    def test_07_failed_build_speaks_up_at_once_with_the_log_link(self):
        p = plan_for([(build(7, "failed"), [ch(F43, "succeeded"), ch(F44, "failed", NOW - 30),
                                            ch(F45, "succeeded"), ch(RAW, "succeeded")])])
        self.assertEqual(keys(p), ["failed:fedora-44-x86_64:7"])
        self.assertIn("http://copr.test/coprs/mujtaba1i/archtoys/build/7/", p["events"][0]["body"])
        self.assertFalse(p["complete"])

    def test_08_partial_failure_attaches_what_worked_and_reports_the_rest(self):
        p = plan_for([(build(8, "failed"), [ch(F43, "succeeded"), ch(F44, "failed", NOW - 3000),
                                            ch(F45, "succeeded"), ch(RAW, "succeeded")])])
        self.assertEqual(sorted(c for c, _ in p["attach"]), sorted([F43, F45, RAW]))
        self.assertEqual(keys(p), ["failed:fedora-44-x86_64:8", "no-rebuild:fedora-44-x86_64:8"])
        self.assertFalse(p["complete"])

    def test_08b_rawhide_alone_failing_is_optional_but_still_reported(self):
        p = plan_for([(build(9, "failed"), [ch(F43, "succeeded"), ch(F44, "succeeded"),
                                            ch(F45, "succeeded"), ch(RAW, "failed", NOW - 30)])])
        self.assertEqual(keys(p), ["failed:fedora-rawhide-x86_64:9"])
        self.assertIn("optional", p["events"][0]["headline"])
        self.assertTrue(p["complete"])          # the release counts as complete
        self.assertFalse(p["all_attached"])     # but the issue stays open as a reminder

    def test_08c_rawhide_still_building_is_waited_for(self):
        p = plan_for([(build(9, "running"), [ch(F43, "succeeded"), ch(F44, "succeeded"),
                                             ch(F45, "succeeded"), ch(RAW, "running")])])
        self.assertFalse(p["complete"])

    def test_09_several_builds_for_one_version_any_success_counts(self):
        p = plan_for([(build(1, "failed"), [ch(F44, "failed", NOW - 5000)]),
                      (build(2, "succeeded"), [ch(F44, "succeeded")])])
        self.assertEqual(p["attach"], [(F44, 2)])
        self.assertEqual(p["events"], [])

    def test_10_rpms_already_attached_means_nothing_to_do(self):
        p = plan_for([(build(1), [ch(c, "succeeded") for c in ALL])], attached=ALL)
        self.assertEqual((p["attach"], p["events"]), ([], []))
        self.assertTrue(p["complete"] and p["all_attached"])

    def test_18_a_rebuild_of_another_version_is_pointed_out(self):
        other = build(20, "running", version="0.3.2-1", sub=SINCE + 500)
        p = plan_for([(build(1, "canceled"), [ch(F44, "canceled", NOW - 300)])], wrong=[other])
        self.assertEqual(keys(p), ["wrong-version:20"])
        self.assertIn("0.3.2-1", p["events"][0]["headline"])

    def test_18b_no_complaint_about_other_versions_when_nothing_is_missing(self):
        other = build(20, "running", version="0.3.2-1", sub=SINCE + 500)
        p = plan_for([(build(1), [ch(c, "succeeded") for c in ALL])], wrong=[other])
        self.assertEqual(p["events"], [])

    def test_completion_is_judged_by_what_is_really_attached(self):
        """A failed download must not count as done (found by the end-to-end run)."""
        chroots = cw.consolidate([(build(1), [ch(c, "succeeded") for c in ALL])])
        planned = {c for c in ALL}                       # decide() assumes every download works
        self.assertEqual(cw.completion(CFG, chroots, planned), (True, True))
        really = {F43, F45, RAW}                          # but fedora-44's download failed
        self.assertEqual(cw.completion(CFG, chroots, really), (False, False))

    def test_unknown_chroot_detail_falls_back_to_the_whole_build(self):
        b = build(1, "succeeded", chroots=[F43, F44])
        chroots = cw.consolidate([(b, [])])
        self.assertEqual({n: c["kind"] for n, c in chroots.items()}, {F43: "succeeded", F44: "succeeded"})


class Trigger(unittest.TestCase):
    """Situations 16 and 17: look at COPR's queue before sending the webhook."""

    def run_trigger(self, builds, webhook_status=200):
        env = dict(ENV, COPR_WEBHOOK_URL="http://hook.test/x", SHA="abc", HTTP_RETRY_SLEEP="0")
        with mock.patch.dict(os.environ, env), \
             mock.patch.object(cw, "commit_epoch", return_value=SINCE + 60), \
             mock.patch.object(cw, "list_builds", return_value=builds), \
             mock.patch.object(cw, "post_webhook", return_value=(webhook_status, "")) as hook:
            code = quiet(cw.trigger, cw.Cfg(os.environ))
        return code, hook.called

    def test_nothing_running_sends_the_webhook(self):
        self.assertEqual(self.run_trigger([build(1, "succeeded", sub=SINCE - 9000)]), (0, True))

    def test_16_same_version_already_building_sends_nothing(self):
        self.assertEqual(self.run_trigger([build(5, "running", sub=SINCE + 100)]), (0, False))

    def test_17_older_version_in_the_queue_still_sends_and_says_why(self):
        code, sent = self.run_trigger([build(5, "running", version="0.3.0-1", sub=SINCE - 600)])
        self.assertEqual((code, sent), (0, True))

    def test_stale_attempt_of_the_same_version_does_not_block_a_new_build(self):
        # an earlier attempt (before this release commit) is still running:
        # a fresh build must still be requested
        self.assertEqual(self.run_trigger([build(5, "running", sub=SINCE - 4000)]), (0, True))

    def test_webhook_refused_fails_the_job(self):
        self.assertEqual(self.run_trigger([], webhook_status=404), (1, True))

    def test_cannot_read_the_queue_still_sends(self):
        env = dict(ENV, COPR_WEBHOOK_URL="http://hook.test/x", SHA="abc", HTTP_RETRY_SLEEP="0")
        with mock.patch.dict(os.environ, env), \
             mock.patch.object(cw, "commit_epoch", return_value=SINCE), \
             mock.patch.object(cw, "list_builds", side_effect=cw.CoprUnavailable("down")), \
             mock.patch.object(cw, "post_webhook", return_value=(200, "")) as hook:
            self.assertEqual(quiet(cw.trigger, cw.Cfg(os.environ)), 0)
        self.assertTrue(hook.called)


if __name__ == "__main__":
    unittest.main()

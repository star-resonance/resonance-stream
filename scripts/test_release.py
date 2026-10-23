"""Tests for release.py. Run: python3 -m unittest discover -s scripts -p 'test_*.py'"""
import json
import os
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import release as r  # noqa: E402

HERE = Path(__file__).parent


def cargo(version):
    return f'[workspace]\nmembers = ["a"]\n\n[workspace.package]\nversion = "{version}"\nedition = "2021"\n'


class Pure(unittest.TestCase):
    def test_version_of_reads_workspace_package(self):
        self.assertEqual(r.version_of(cargo("0.6.2")), "0.6.2")
        self.assertEqual(
            r.version_of('[package]\nversion = "9.9.9"\n[workspace.package]\nversion = "1.2.3"\n'), "1.2.3"
        )
        with self.assertRaisesRegex(r.ReleaseError, r"no \[workspace\.package\] version"):
            r.version_of('[package]\nversion = "9.9.9"\n')

    def test_check_next_wants_a_new_version_above_the_latest_release(self):
        self.assertEqual(r.check_next("0.6.3", ["v0.6.2"]), "v0.6.3")
        self.assertEqual(r.check_next("0.6.3", []), "v0.6.3")
        self.assertEqual(r.check_next("0.6.3", ["v0.6.2", "v0.6.3-rc.1", "v0.6.0-rc.main"]), "v0.6.3")
        self.assertEqual(r.check_next("0.10.0", ["v0.9.9"]), "v0.10.0", "numeric, not text, order")
        with self.assertRaisesRegex(r.ReleaseError, r"v0\.6\.2 is already released"):
            r.check_next("0.6.2", ["v0.6.2"])
        with self.assertRaisesRegex(r.ReleaseError, r"not above the latest release v0\.6\.2"):
            r.check_next("0.6.1", ["v0.6.2"])

    def test_check_next_refuses_a_version_that_is_not_x_y_z(self):
        for v in ["0.6", "0.6.2-rc.1", "v0.6.2", "", "0.6.x"]:
            with self.subTest(v=v), self.assertRaisesRegex(r.ReleaseError, r"not X\.Y\.Z"):
                r.check_next(v, [])

    def test_rc_tag_is_the_next_candidate_number_of_a_version(self):
        self.assertEqual(r.rc_tag("v0.6.3", []), "v0.6.3-rc.1")
        tags = ["v0.6.3-rc.1", "v0.6.3-rc.2", "v0.6.30-rc.5", "v0.6.3-rc.main"]
        self.assertEqual(r.rc_tag("v0.6.3", tags), "v0.6.3-rc.3")
        with self.assertRaisesRegex(r.ReleaseError, "not a release tag"):
            r.rc_tag("v0.6.3-rc.1", [])

    def test_check_candidate_wants_the_branch_to_name_head_on_main_and_no_other(self):
        head = "0123456789abcdef0123456789abcdef01234567"
        r.check_candidate("candidate/0123456789ab", head, True, [])
        with self.assertRaisesRegex(r.ReleaseError, r"candidate/<12-character sha>"):
            r.check_candidate("candidate/v0.6.3", head, True, [])
        with self.assertRaisesRegex(r.ReleaseError, "not HEAD"):
            r.check_candidate("candidate/fedcba987654", head, True, [])
        with self.assertRaisesRegex(r.ReleaseError, r"not on origin/main"):
            r.check_candidate("candidate/0123456789ab", head, False, [])
        with self.assertRaisesRegex(r.ReleaseError, r"candidate/aaaaaaaaaaaa is left over"):
            r.check_candidate("candidate/0123456789ab", head, True, ["candidate/aaaaaaaaaaaa"])

    def test_checks_passed_wants_a_finished_check_run_none_failed_own_run_left_out(self):
        def run(name, status, conclusion, run_id=1):
            return {"name": name, "status": status, "conclusion": conclusion,
                    "details_url": f"https://github.com/o/r/actions/runs/{run_id}/job/9"}

        r.checks_passed([run("test", "completed", "success"), run("guard", "completed", "skipped")], "7")
        r.checks_passed([run("test", "completed", "success"), run("something", "in_progress", None, 7)], "7")
        with self.assertRaisesRegex(r.ReleaseError, "no check run"):
            r.checks_passed([run("something", "in_progress", None, 7)], "7")
        with self.assertRaisesRegex(r.ReleaseError, "test: in_progress"):
            r.checks_passed([run("test", "in_progress", None)], "7")
        for c in ["failure", "timed_out", "cancelled", "action_required"]:
            with self.subTest(c=c), self.assertRaisesRegex(r.ReleaseError, f"test: {c}"):
                r.checks_passed([run("test", "completed", c)], "7")

    # A candidate that failed leaves its check runs on the main commit; a new candidate branch at
    # the same commit must not be refused for them, nor for a guard run of an earlier push there.
    def test_checks_passed_leaves_out_the_candidate_workflows_jobs_called_ones_too(self):
        def run(name, conclusion, run_id):
            return {"name": name, "status": "completed", "conclusion": conclusion,
                    "details_url": f"https://github.com/o/r/actions/runs/{run_id}/job/9"}

        own = sorted(r.CANDIDATE_JOBS)
        r.checks_passed(
            [run("test", "success", 1)]
            + [run(n, "failure", 2) for n in own]
            + [run("real-app smoke test / bridge smoke (Windows)", "failure", 2)],
            "7",
        )
        with self.assertRaisesRegex(r.ReleaseError, "no check run"):
            r.checks_passed([run(own[0], "success", 2)], "7")

    # Auto-candidate's two jobs post check runs on the main commit too: a failed push must never
    # make Candidate refuse the commit (nor its own next decision).
    def test_checks_passed_leaves_out_the_auto_candidate_jobs(self):
        def run(name, conclusion, status="completed"):
            return {"name": name, "status": status, "conclusion": conclusion,
                    "details_url": "https://github.com/o/r/actions/runs/2/job/9"}

        r.checks_passed(
            [dict(run("test", "success"), details_url="https://github.com/o/r/actions/runs/1/job/9"),
             run("decide on a candidate", "failure"), run("push the candidate branch", "failure"),
             run("push the candidate branch", None, "in_progress")],
            "7",
        )

    def test_should_candidate_wants_an_unreleased_version_above_the_latest_with_notes(self):
        self.assertTrue(r.should_candidate("0.6.3", ["v0.6.2"], True))
        self.assertTrue(r.should_candidate("0.6.3", [], True))
        self.assertFalse(r.should_candidate("0.6.2", ["v0.6.2"], True), "already tagged final")
        self.assertFalse(r.should_candidate("0.6.1", ["v0.6.2"], True), "below the latest final tag")
        self.assertFalse(r.should_candidate("0.6.3", ["v0.6.2"], False), "no release notes")
        self.assertTrue(r.should_candidate("0.6.3", ["v0.6.2", "v0.6.3-rc.1"], True), "an rc is not a release")

    # auto-merge.yml is a workflow_run of the same workflows: its runs are still going when the last
    # trigger's decide runs, so only the named workflows' push/dispatch runs count.
    def test_ci_done_reads_only_the_named_workflows_push_and_dispatch_runs(self):
        def run(name, status, conclusion=None, event="push"):
            return {"name": name, "event": event, "status": status, "conclusion": conclusion}

        names = ["CI", "CLAUDE.md check", "Line endings"]
        ok = [run("CI", "completed", "success"), run("Line endings", "completed", "success"),
              run("CLAUDE.md check", "completed", "skipped", "workflow_dispatch")]
        self.assertEqual(r.ci_done(ok, names), "ready")
        self.assertEqual(r.ci_done([run("CI", "completed", "neutral")], names), "ready")
        self.assertEqual(r.ci_done(ok + [run("CI", "in_progress")], names), "waiting")
        self.assertEqual(r.ci_done(ok + [run("CI", "queued", None, "workflow_dispatch")], names), "waiting")
        self.assertEqual(
            r.ci_done(ok + [run("Auto-merge", "in_progress", None, "workflow_run"),
                            run("Main red", "completed", "failure", "workflow_run"),
                            run("CI", "in_progress", None, "pull_request"),
                            run("Auto-candidate", "in_progress", None, "workflow_run")], names),
            "ready",
        )
        self.assertEqual(r.ci_done(ok + [run("CI", "completed", "failure")], names), "red")
        for c in ["cancelled", "timed_out", "action_required"]:
            self.assertEqual(r.ci_done([run("CI", "completed", c)], names), "red", c)
        self.assertEqual(r.ci_done([], names), "waiting")
        self.assertEqual(r.ci_done([run("Auto-merge", "completed", "success", "workflow_run")], names), "waiting")

    def test_the_job_names_left_out_are_the_candidate_workflows_own_jobs(self):
        text = "\n".join(
            (HERE.parent / ".github" / "workflows" / f).read_text(encoding="utf-8")
            for f in ["candidate.yml", "candidate-guard.yml", "auto-candidate.yml"]
        )
        names = re.findall(r"^ {4}name: (.+)$", text, re.M)
        self.assertEqual(sorted(r.CANDIDATE_JOBS), sorted(names))

    def test_a_failed_candidate_keeps_its_branch_and_only_a_tagged_one_deletes_it(self):
        text = (HERE.parent / ".github" / "workflows" / "candidate.yml").read_text(encoding="utf-8")
        job = re.search(r"^  cleanup:\n(.*?)(?=^  \S|\Z)", text, re.M | re.S).group(1)
        self.assertIn("name: delete the candidate branch", job)
        cond = re.search(r"^    if: (.+)$", job, re.M).group(1)
        self.assertNotIn("always()", cond)
        self.assertIn("needs.tag.result == 'success'", cond)
        self.assertIn("tag", re.search(r"^    needs: \[(.+)\]$", job, re.M).group(1).split(", "))

    def test_check_promote_wants_the_highest_rc_of_a_new_version_with_that_cargo_version(self):
        tags = ["v0.6.2", "v0.6.3-rc.1", "v0.6.3-rc.2"]
        self.assertEqual(r.check_promote("v0.6.3-rc.2", tags, "0.6.3"), "v0.6.3")
        with self.assertRaisesRegex(r.ReleaseError, "not a candidate tag"):
            r.check_promote("v0.6.3", tags, "0.6.3")
        with self.assertRaisesRegex(r.ReleaseError, r"v0\.6\.3-rc\.2 is newer"):
            r.check_promote("v0.6.3-rc.1", tags, "0.6.3")
        with self.assertRaisesRegex(r.ReleaseError, "already released"):
            r.check_promote("v0.6.3-rc.2", tags + ["v0.6.3"], "0.6.3")
        with self.assertRaisesRegex(r.ReleaseError, r"not above the latest release v0\.7\.0"):
            r.check_promote("v0.6.3-rc.2", tags + ["v0.7.0"], "0.6.3")
        with self.assertRaisesRegex(r.ReleaseError, r"Cargo\.toml at v0\.6\.3-rc\.2 is 0\.6\.4, not 0\.6\.3"):
            r.check_promote("v0.6.3-rc.2", tags, "0.6.4")

    # Promote runs from a branch rc/<rc tag>, which Candidate makes at the rc tag's commit: the
    # GitHub app lists branches only. The tag stays the record; the branch is a handle.
    def test_rc_of_branch_reads_the_rc_tag_and_refuses_anything_else(self):
        self.assertEqual(r.rc_of_branch("rc/v0.6.3-rc.1"), "v0.6.3-rc.1")
        self.assertEqual(r.rc_of_branch("rc/v0.0.1-rc.12"), "v0.0.1-rc.12")
        for b in ["candidate/v0.6.3-rc.1", "candidate/0123456789ab", "rc/v0.6.3", "main", "rc", "rc/v1-rc.x",
                  "rc/v0.6.0-rc.main"]:
            with self.subTest(b=b), self.assertRaisesRegex(r.ReleaseError, r"not rc/<rc tag>"):
                r.rc_of_branch(b)

    def test_superseded_branches_are_the_same_versions_lower_rc_branches(self):
        branches = ["rc/v1.2.0-rc.1", "rc/v1.2.0-rc.2", "rc/v1.1.0-rc.4", "candidate/0123456789ab", "rc/v1.2.0-rc.3"]
        self.assertEqual(r.superseded_branches(branches, "v1.2.0-rc.3"), ["rc/v1.2.0-rc.1", "rc/v1.2.0-rc.2"])
        self.assertEqual(r.superseded_branches([], "v1.2.0-rc.1"), [])

    def test_released_branches_are_the_released_version_and_every_older_one(self):
        branches = ["rc/v1.2.0-rc.1", "rc/v1.2.0-rc.2", "rc/v1.1.0-rc.4", "rc/v1.3.0-rc.1", "rc/v1.10.0-rc.1",
                    "candidate/0123456789ab", "main"]
        self.assertEqual(r.released_branches(branches, "v1.2.0"),
                         ["rc/v1.2.0-rc.1", "rc/v1.2.0-rc.2", "rc/v1.1.0-rc.4"])


def git(cwd, *args):
    return subprocess.run(["git", "-C", str(cwd), *args], check=True, capture_output=True, text=True).stdout.strip()


def release(cwd, *args, stdin=None):
    return subprocess.run([sys.executable, "-I", str(HERE / "release.py"), *args], cwd=cwd, input=stdin,
                          capture_output=True, text=True)


class Repo(unittest.TestCase):
    """One origin, one clone on main, Cargo.toml at 0.6.2, released as v0.6.2."""

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        origin, self.dev = root / "origin.git", root / "dev"
        git(root, "init", "-q", "--bare", "-b", "main", str(origin))
        git(root, "clone", "-q", str(origin), str(self.dev))
        git(self.dev, "symbolic-ref", "HEAD", "refs/heads/main")
        git(self.dev, "config", "user.email", "t@t")
        git(self.dev, "config", "user.name", "t")
        (self.dev / "Cargo.toml").write_text(cargo("0.6.2"))
        git(self.dev, "add", "-A")
        git(self.dev, "commit", "-q", "-m", "first")
        git(self.dev, "tag", "v0.6.2")
        git(self.dev, "push", "-q", "origin", "main", "v0.6.2")

    def bump(self, version):
        (self.dev / "Cargo.toml").write_text(cargo(version))
        git(self.dev, "commit", "-q", "-am", f"version {version}")

    def ok(self, *args, **kw):
        p = release(self.dev, *args, **kw)
        self.assertEqual(p.returncode, 0, p.stderr)
        return p.stdout.strip()

    def refuses(self, pattern, *args, **kw):
        p = release(self.dev, *args, **kw)
        self.assertNotEqual(p.returncode, 0, p.stdout)
        self.assertRegex(p.stderr, pattern)

    def test_next_prints_the_tag_of_the_cargo_version_and_refuses_a_released_one(self):
        self.refuses(r"v0\.6\.2 is already released", "next")
        self.bump("0.6.3")
        self.assertEqual(self.ok("next"), "v0.6.3")

    def test_next_refuses_a_commit_that_already_carries_a_candidate_tag(self):
        self.bump("0.6.3")
        git(self.dev, "tag", "v0.6.3-rc.1")
        self.refuses(r"already a candidate as v0\.6\.3-rc\.1", "next")

    def test_latest_ignores_rc_and_other_v_tags(self):
        for t in ["v0.0.0-probe", "v0.6.10", "v0.6.11-rc.1"]:
            git(self.dev, "tag", t)
        self.assertEqual(self.ok("latest"), "v0.6.10")

    def test_rc_and_check_promote_read_the_tags_and_cargo_toml_at_the_rc_tag(self):
        self.bump("0.6.3")
        self.assertEqual(self.ok("rc", "v0.6.3"), "v0.6.3-rc.1")
        git(self.dev, "tag", "v0.6.3-rc.1")
        self.assertEqual(self.ok("rc", "v0.6.3"), "v0.6.3-rc.2")
        self.assertEqual(self.ok("check-promote", "v0.6.3-rc.1"), "v0.6.3")
        self.bump("0.6.4")
        git(self.dev, "tag", "v0.6.3-rc.2")
        self.refuses(r"is 0\.6\.4, not 0\.6\.3", "check-promote", "v0.6.3-rc.2")

    def test_check_candidate_reads_head_origin_main_and_the_candidate_branches(self):
        short = git(self.dev, "rev-parse", "HEAD")[:12]
        git(self.dev, "fetch", "-q", "origin")
        git(self.dev, "push", "-q", "origin", f"HEAD:refs/heads/candidate/{short}")
        self.ok("check-candidate", f"candidate/{short}")
        git(self.dev, "push", "-q", "origin", "HEAD:refs/heads/candidate/aaaaaaaaaaaa")
        self.refuses("left over", "check-candidate", f"candidate/{short}")
        git(self.dev, "push", "-q", "origin", "--delete", "candidate/aaaaaaaaaaaa")
        self.bump("0.6.3")
        off = git(self.dev, "rev-parse", "HEAD")[:12]
        self.refuses(r"not on origin/main", "check-candidate", f"candidate/{off}")

    def test_checks_passed_reads_the_api_json_on_stdin(self):
        runs = {"check_runs": [{"name": "test", "status": "completed", "conclusion": "success",
                                "details_url": "https://github.com/o/r/actions/runs/1/job/9"}]}
        self.ok("checks-passed", "7", stdin=json.dumps(runs))
        runs["check_runs"][0]["conclusion"] = "failure"
        self.refuses("test: failure", "checks-passed", "7", stdin=json.dumps(runs))

    def test_rc_branch_superseded_and_released_read_the_origins_rc_branches(self):
        git(self.dev, "fetch", "-q", "origin")
        git(self.dev, "push", "-q", "origin", "HEAD:refs/heads/rc/v0.6.3-rc.1", "HEAD:refs/heads/rc/v0.6.2-rc.1",
            "HEAD:refs/heads/rc/v0.7.0-rc.1")
        self.assertEqual(self.ok("rc-branch", "rc/v0.6.3-rc.1"), "v0.6.3-rc.1")
        self.assertEqual(self.ok("superseded", "v0.6.3-rc.2"), "rc/v0.6.3-rc.1")
        self.assertEqual(self.ok("released", "v0.6.3"), "rc/v0.6.2-rc.1 rc/v0.6.3-rc.1")
        short = git(self.dev, "rev-parse", "HEAD")[:12]
        self.ok("check-candidate", f"candidate/{short}")

    def test_should_candidate_reads_cargo_toml_the_tags_and_the_notes_file(self):
        self.assertEqual(self.ok("should-candidate"), "false", "0.6.2 is released")
        self.bump("0.6.3")
        self.assertEqual(self.ok("should-candidate"), "false", "no release-notes/v0.6.3.md")
        (self.dev / "release-notes").mkdir()
        (self.dev / "release-notes" / "v0.6.3.md").write_text("x\n")
        self.assertEqual(self.ok("should-candidate"), "true")
        git(self.dev, "tag", "v0.6.3-rc.1")
        self.assertEqual(self.ok("should-candidate"), "false", "HEAD is a candidate already")
        self.bump("0.6.1")
        (self.dev / "release-notes" / "v0.6.1.md").write_text("x\n")
        self.assertEqual(self.ok("should-candidate"), "false", "below v0.6.2")

    def test_ci_done_reads_the_actions_runs_json_on_stdin(self):
        runs = {"workflow_runs": [{"name": "CI", "event": "push", "status": "completed", "conclusion": "success"}]}
        self.assertEqual(self.ok("ci-done", "CI", "Line endings", stdin=json.dumps(runs)), "ready")
        runs["workflow_runs"][0]["status"] = "in_progress"
        self.assertEqual(self.ok("ci-done", "CI", stdin=json.dumps(runs)), "waiting")
        runs["workflow_runs"] = [{"name": "CI", "event": "push", "status": "completed", "conclusion": "failure"}]
        self.assertEqual(self.ok("ci-done", "CI", stdin=json.dumps(runs)), "red")

    def test_the_commands_of_the_old_flow_and_of_agent_configs_stamp_are_gone(self):
        for cmd in ["stamp", "prepare", "tag"]:
            self.refuses(r"usage: release\.py next \| rc <tag> \| latest \|", cmd, "x")


if __name__ == "__main__":
    unittest.main()

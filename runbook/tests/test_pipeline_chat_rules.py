"""Dry run of the `chat-rules` pipeline against tests/fake_app.py, a stand-in that models the dedupe, block list and chat log the way
the backend is understood to. It proves the pipeline drives the commands and judges the events; the real app's answer is Kade's run
(or the smoke run's)."""
import stat
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from runbook import bridge, common, mockfeed  # noqa: E402
from runbook.common import Recorder  # noqa: E402
from runbook.pipelines.chat_rules import ChatRules  # noqa: E402

HERE = Path(__file__).resolve().parent
pytestmark = [
    pytest.mark.skipif(sys.platform == "win32", reason="the stand-in app is a shell script"),
    pytest.mark.skipif(bridge.node_path() is None or not (bridge.BRIDGE_DIR / "node_modules" / "aedes").is_dir(),
                       reason="needs node and `npm ci` in runbook/bridge"),
]


@pytest.fixture(autouse=True)
def env(monkeypatch, tmp_path):
    monkeypatch.setattr(common, "RUNS", tmp_path)
    monkeypatch.setenv("RUNBOOK_DRYRUN", "1")
    exe = tmp_path / "local" / "resonance-stream.exe"
    exe.parent.mkdir()
    exe.write_text('#!/bin/sh\nFAKE_APP_EXE="$0" exec python3 "$FAKE_APP_PY" "$@"\n', encoding="utf-8")
    exe.chmod(exe.stat().st_mode | stat.S_IXUSR)
    monkeypatch.setenv("FAKE_APP_PY", str(HERE / "fake_app.py"))
    monkeypatch.setenv("FAKE_APP_LIFETIME", "90")
    monkeypatch.setenv("FAKE_APP_LEAD_IN_MS", "50")
    yield exe


def run(exe, tmp_path):
    rec = Recorder("chat-rules")
    ChatRules(rec, exe, tmp_path).run()
    assert [p for p in mockfeed.posix_processes() if str(tmp_path) in p["Path"]] == [], "no app may be left running"
    return rec, {r.check: r for r in rec.rows}


CHECKS = ("CR-dedupe", "CR-history", "CR-block", "CR-block-later", "CR-unblock", "CR-clear",
          "CR-ruby-answer", "CR-ruby-join", "CR-ruby-plain", "CR-ruby-reading")


def test_a_backend_that_follows_the_rules_passes_every_row(env, tmp_path):
    rec, got = run(env, tmp_path)
    assert rec.summary()["fail"] == 0, rec.report()
    for check in CHECKS:
        assert got[check].status == "pass", rec.report()


@pytest.mark.parametrize("bug, failing", [
    ("no-dedupe", {"CR-dedupe"}),
    ("no-retro", {"CR-block"}),
    ("block-later", {"CR-block-later"}),
    ("clear-keeps", {"CR-clear"}),
    ("ruby-merged", {"CR-ruby-answer"}),
    ("ruby-lossy", {"CR-ruby-join"}),
    ("ruby-marks-plain", {"CR-ruby-plain"}),
    ("ruby-katakana", {"CR-ruby-reading"}),
])
def test_each_broken_rule_is_caught_by_its_own_row(env, tmp_path, monkeypatch, bug, failing):
    monkeypatch.setenv("FAKE_APP_CHAT_BUG", bug)
    rec, got = run(env, tmp_path)
    for check in failing:
        assert got[check].status == "fail", f"{bug}: {check}\n{rec.report()}"
    assert {c for c in CHECKS if got.get(c) and got[c].status == "fail"} >= failing

import os
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from runbook import common  # noqa: E402


@pytest.mark.parametrize("text,expected", [
    ("pass", ("pass", "")), ("p", ("pass", "")), ("Y", ("pass", "")), ("ok fine", ("pass", "fine")),
    ("fail: bar stuck at 0%", ("fail", "bar stuck at 0%")), ("f", ("fail", "")), ("no", ("fail", "")),
    ("skip", ("skip", "")), ("s: no VPN here", ("skip", "no VPN here")),
    ("", ("skip", "")), ("   ", ("skip", "")),
    ("pasd", ("skip", "pasd")),  # a typo is never a pass
])
def test_parse_answer(text, expected):
    assert common.parse_answer(text) == expected


def test_a_note_keeps_everything_after_the_first_separator():
    assert common.parse_answer("fail: a: b") == ("fail", "a: b")


def test_the_recorder_refuses_a_made_up_status():
    with pytest.raises(ValueError):
        common.Recorder("j").record("1", "t", "maybe")


def test_auto_is_pass_or_fail_only():
    rec = common.Recorder("j")
    assert rec.auto("1", "a", True).status == "pass"
    assert rec.auto("2", "b", False, "why").status == "fail"
    assert rec.summary() == {"pass": 1, "fail": 1, "skip": 0}


def test_manual_reads_answers_from_the_queue(monkeypatch, capsys):
    monkeypatch.setenv("RUNBOOK_ANSWERS", "pass,fail: broke")
    rec = common.Recorder("j")
    assert rec.manual("A", "first", "do x", "y").status == "pass"
    second = rec.manual("B", "second", "do z", "w")
    assert (second.status, second.evidence) == ("fail", "broke")
    assert os.environ["RUNBOOK_ANSWERS"] == ""


def test_an_empty_queue_is_an_error_not_a_silent_skip(monkeypatch):
    monkeypatch.setenv("RUNBOOK_ANSWERS", "")
    with pytest.raises(RuntimeError):
        common.Recorder("j").manual("A", "t", "s", "e")


def test_the_report_lists_every_row_and_escapes_pipes(monkeypatch):
    monkeypatch.setenv("RUNBOOK_DRYRUN", "1")
    rec = common.Recorder("x")
    rec.record("1", "one", "pass", "a|b")
    rec.record("2", "two", "skip")
    text = rec.report()
    assert "Runbook report: x" in text
    assert "pass 1 / fail 0 / skip 1" in text
    assert "a\\|b" in text
    assert "| 2 two | skip |  |" in text


def test_a_dry_run_report_saves_nothing(monkeypatch, tmp_path):
    monkeypatch.setenv("RUNBOOK_DRYRUN", "1")
    monkeypatch.setattr(common, "RUNS", tmp_path)
    common.Recorder("j").report()
    assert list(tmp_path.iterdir()) == []


def test_capture_returns_the_fixture_in_a_dry_run(monkeypatch):
    monkeypatch.setenv("RUNBOOK_DRYRUN", "1")
    code, text = common.capture(["netsh"], fixture="hello.txt")
    assert (code, text.strip()) == (0, "hello")


def test_capture_without_a_fixture_in_a_dry_run_is_an_error(monkeypatch):
    monkeypatch.setenv("RUNBOOK_DRYRUN", "1")
    with pytest.raises(RuntimeError):
        common.capture(["netsh"])


def test_capture_runs_a_real_command_outside_a_dry_run(monkeypatch):
    monkeypatch.delenv("RUNBOOK_DRYRUN", raising=False)
    code, text = common.capture([sys.executable, "-c", "print('hi')"])
    assert (code, text.strip()) == (0, "hi")


def test_a_missing_program_is_code_127(monkeypatch):
    monkeypatch.delenv("RUNBOOK_DRYRUN", raising=False)
    assert common.capture(["definitely-not-a-program-xyz"])[0] == 127


def test_ask_text_reads_the_named_variable_in_a_dry_run(monkeypatch):
    monkeypatch.setenv("RUNBOOK_DRYRUN", "1")
    monkeypatch.setenv("RUNBOOK_TEXT_DEV_EXE", r"C:\dev\app.exe")
    assert common.ask_text("DEV_EXE", "path? ") == r"C:\dev\app.exe"


def test_ask_text_in_a_dry_run_without_the_variable_is_an_error(monkeypatch):
    monkeypatch.setenv("RUNBOOK_DRYRUN", "1")
    monkeypatch.delenv("RUNBOOK_TEXT_NOPE", raising=False)
    with pytest.raises(RuntimeError):
        common.ask_text("NOPE", "?")


def test_ask_text_strips_quotes_and_spaces(monkeypatch):
    monkeypatch.delenv("RUNBOOK_DRYRUN", raising=False)
    monkeypatch.setattr("builtins.input", lambda _prompt: '  "C:\\Program Files\\a.exe"  ')
    assert common.ask_text("X", "?") == "C:\\Program Files\\a.exe"


def test_capture_starts_the_program_shutil_which_finds(monkeypatch, tmp_path):
    # On Windows `npx` is npx.cmd: only the resolved full path can be started without a shell.
    monkeypatch.delenv("RUNBOOK_DRYRUN", raising=False)
    seen = []
    real_run = common.subprocess.run

    def spy(cmd, **kw):
        seen.append(cmd[0])
        return real_run([sys.executable, "-c", "print('x')"], **kw)

    monkeypatch.setattr(common.subprocess, "run", spy)
    monkeypatch.setattr(common.shutil, "which", lambda name: r"C:\tools\npx.CMD" if name == "npx" else None)
    common.capture(["npx", "--version"])
    common.capture(["unknown-tool"])
    assert seen == [r"C:\tools\npx.CMD", "unknown-tool"]


def test_fetch_text_reads_the_fixture_in_a_dry_run(monkeypatch):
    monkeypatch.setenv("RUNBOOK_DRYRUN", "1")
    assert common.fetch_text("https://example.invalid/x", fixture="hello.txt").strip() == "hello"


def test_fetch_text_in_a_dry_run_needs_a_fixture(monkeypatch):
    monkeypatch.setenv("RUNBOOK_DRYRUN", "1")
    with pytest.raises(RuntimeError):
        common.fetch_text("https://example.invalid/x")


def test_runs_dir_can_be_moved_by_the_environment():
    import subprocess
    out = subprocess.run(
        [sys.executable, "-c", "from runbook import common; print(common.RUNS)"],
        cwd=Path(__file__).resolve().parent.parent, env={**os.environ, "RUNBOOK_RUNS_DIR": "/somewhere/else"},
        capture_output=True, text=True,
    ).stdout.strip()
    assert Path(out) == Path("/somewhere/else")


# --- an empty or unclear answer asks again instead of skipping (Enter pressed by accident) -----------------
@pytest.mark.parametrize("text,clear", [
    ("pass", True), ("p", True), ("fail: why", True), ("f", True), ("skip", True), ("s: no VPN", True),
    ("", False), ("   ", False), ("pasd", False), ("hmm", False),
])
def test_answer_is_clear(text, clear):
    assert common.answer_is_clear(text) is clear


def test_manual_asks_again_until_the_answer_is_clear(monkeypatch, capsys):
    monkeypatch.delenv("RUNBOOK_ANSWERS", raising=False)
    answers = iter(["", "pasd", "fail: bar stuck"])
    calls = []
    monkeypatch.setattr("builtins.input", lambda _p: (calls.append(1), next(answers))[1])
    row = common.Recorder("j").manual("A", "t", "do it", "expect")
    assert (row.status, row.evidence) == ("fail", "bar stuck") and len(calls) == 3
    assert "type pass, fail or skip" in capsys.readouterr().out


def test_a_queued_empty_answer_is_still_a_skip(monkeypatch):
    # dry runs feed answers from RUNBOOK_ANSWERS; an empty item there must not loop forever
    monkeypatch.setenv("RUNBOOK_ANSWERS", ",pass")
    rec = common.Recorder("j")
    assert rec.manual("A", "t", "s", "e").status == "skip"
    assert rec.manual("B", "t", "s", "e").status == "pass"


def test_the_default_exe_is_the_release_build_in_the_repo():
    exe = common.default_exe()
    assert exe.parts[-3:] == ("target", "release", "resonance-stream.exe")
    assert exe == common.ROOT.parent / "target" / "release" / "resonance-stream.exe"


def test_exe_choice_prefers_the_variable_then_the_default_when_it_exists(monkeypatch, tmp_path):
    built = tmp_path / "resonance-stream.exe"
    built.write_text("x")
    monkeypatch.setattr(common, "default_exe", lambda: built)
    monkeypatch.delenv("RUNBOOK_TEXT_LOCAL_EXE", raising=False)
    assert common.ask_exe() == built  # nothing asked
    monkeypatch.setenv("RUNBOOK_TEXT_LOCAL_EXE", r"C:\other\app.exe")
    assert str(common.ask_exe()) == r"C:\other\app.exe"


def test_exe_choice_asks_when_there_is_no_default_build(monkeypatch, tmp_path):
    monkeypatch.setattr(common, "default_exe", lambda: tmp_path / "missing.exe")
    monkeypatch.delenv("RUNBOOK_TEXT_LOCAL_EXE", raising=False)
    monkeypatch.setattr("builtins.input", lambda prompt="": r'"C:\typed\app.exe"')
    assert str(common.ask_exe()) == r"C:\typed\app.exe"

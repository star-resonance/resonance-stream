import hashlib
import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from runbook import updater  # noqa: E402

FIX = Path(__file__).resolve().parent / "fixtures"
OLD, NEW = "Resonance-Stream-v0.6.0.exe", "Resonance-Stream-v0.6.1.exe"


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


# --- SHA256SUMS.txt --------------------------------------------------------------------------
def test_parse_the_real_sums_file():
    sums = updater.parse_sums((FIX / "real_sums_v0.6.1.txt").read_text())
    assert sums == {NEW: "40c17e9087de8a3404f91cec41073e7ecfe648de9d9cc1c307e07e67dff7bc7b"}


def test_parse_sums_binary_marker_crlf_and_case():
    a, b = "AB" * 32, "12" * 32
    text = f"{a}  a.exe\r\n\r\n{b} *b.exe\r\n"
    assert updater.parse_sums(text) == {"a.exe": a.lower(), "b.exe": b}


def test_a_sums_file_with_no_hashes_is_an_error():
    with pytest.raises(ValueError):
        updater.parse_sums("<html>Not Found</html>")


# --- flipping one byte (A2) --------------------------------------------------------------------
def test_flip_one_byte_changes_exactly_one_byte(tmp_path):
    f = tmp_path / "update_temp.exe"
    original = bytes(range(256)) * 4
    f.write_bytes(original)
    offset, before, after = updater.flip_one_byte(f)
    changed = [i for i, (a, b) in enumerate(zip(original, f.read_bytes())) if a != b]
    assert changed == [offset] and before != after and len(f.read_bytes()) == len(original)


def test_flip_one_byte_twice_restores_the_file(tmp_path):
    f = tmp_path / "x"
    f.write_bytes(b"hello world")
    offset, _, _ = updater.flip_one_byte(f)
    updater.flip_one_byte(f, offset)
    assert f.read_bytes() == b"hello world"


def test_flip_one_byte_refuses_an_empty_file_and_a_bad_offset(tmp_path):
    f = tmp_path / "x"
    f.write_bytes(b"")
    with pytest.raises(ValueError):
        updater.flip_one_byte(f)
    f.write_bytes(b"abc")
    with pytest.raises(ValueError):
        updater.flip_one_byte(f, 3)


# --- the signature file (A5) -------------------------------------------------------------------
def test_the_real_signature_is_the_primary_key_for_0_6_1():
    sig = (FIX / "real_v0.6.1.exe.sig").read_text()
    assert updater.signature_key_id(sig) == "9005FB9491133B75"
    assert updater.signature_version(sig) == "0.6.1"


def test_a_signature_that_is_not_a_signature_is_an_error():
    with pytest.raises(ValueError):
        updater.signature_key_id("not a signature")


def test_the_backup_key_is_told_from_the_primary():
    assert updater.is_backup_key("0099CF71AABBCCDD")
    assert not updater.is_backup_key("9005FB9491133B75")


def test_verify_output_ok():
    ok, _ = updater.parse_verify(0, (FIX / "verify_update_ok.txt").read_text(), "0.6.0")
    assert ok


def test_verify_output_refused_or_wrong_version_is_not_ok():
    assert not updater.parse_verify(1, (FIX / "verify_update_refused.txt").read_text(), "0.6.9")[0]
    # exit 0 but for another version, or a code 127 (cargo missing): never ok
    assert not updater.parse_verify(0, "ok: x is signed by an app key for version 0.6.0", "0.6.9")[0]
    assert not updater.parse_verify(127, "not found: cargo", "0.6.0")[0]


# --- leftovers and the three outcomes ----------------------------------------------------------
def make_folder(tmp_path, exe_bytes, old=None, temp=None, part=None):
    d = tmp_path / "app"
    d.mkdir()
    (d / OLD).write_bytes(exe_bytes)
    if old is not None:
        (d / (OLD + ".old")).write_bytes(old)
    if temp is not None:
        (d / "update_temp.exe").write_bytes(temp)
    if part is not None:
        (d / "update_temp.exe.part").write_bytes(part)
    return d


def test_leftovers_lists_what_is_there(tmp_path):
    d = make_folder(tmp_path, b"x", old=b"o", part=b"p")
    assert updater.leftovers(d, OLD) == {"old": True, "temp": False, "part": True}


def test_a1_after_a_good_update_everything_passes(tmp_path):
    old = (FIX / "fake_app_v0.6.0.bin").read_bytes()
    new = (FIX / "fake_app_v0.6.1.bin").read_bytes()
    d = make_folder(tmp_path, new, old=old)
    checks = updater.check_a1(d, OLD, old_sha=hashlib.sha256(old).hexdigest(), new_sha=hashlib.sha256(new).hexdigest())
    assert all(ok for _, ok, _ in checks), checks


def test_a1_an_update_that_never_happened_fails(tmp_path):
    old = (FIX / "fake_app_v0.6.0.bin").read_bytes()
    new = (FIX / "fake_app_v0.6.1.bin").read_bytes()
    d = make_folder(tmp_path, old)  # still the old exe, no .old
    checks = updater.check_a1(d, OLD, hashlib.sha256(old).hexdigest(), hashlib.sha256(new).hexdigest())
    assert sum(1 for _, ok, _ in checks if not ok) >= 2


def test_a1_a_temp_file_left_behind_fails(tmp_path):
    old = b"old"
    new = b"new"
    d = make_folder(tmp_path, new, old=old, temp=b"zzz")
    checks = updater.check_a1(d, OLD, hashlib.sha256(old).hexdigest(), hashlib.sha256(new).hexdigest())
    assert any(not ok and "update_temp.exe" in t for t, ok, _ in checks)


def test_a2_a_refused_update_leaves_the_old_exe_and_no_old_file(tmp_path):
    old = b"old exe"
    d = make_folder(tmp_path, old, temp=b"flipped")
    assert all(ok for _, ok, _ in updater.check_a2(d, OLD, hashlib.sha256(old).hexdigest()))


def test_a2_a_swapped_exe_or_an_old_file_fails(tmp_path):
    old = b"old exe"
    d = make_folder(tmp_path, b"other", old=b"old exe", temp=b"flipped")
    bad = [t for t, ok, _ in updater.check_a2(d, OLD, hashlib.sha256(old).hexdigest()) if not ok]
    assert len(bad) == 2, bad


def test_a3_a_half_file_named_update_temp_exe_fails_but_a_part_file_is_fine(tmp_path):
    ok_dir = make_folder(tmp_path, b"x", part=b"half")
    assert all(ok for _, ok, _ in updater.check_a3(ok_dir, OLD))
    bad_dir = tmp_path / "bad"
    bad_dir.mkdir()
    (bad_dir / OLD).write_bytes(b"x")
    (bad_dir / "update_temp.exe").write_bytes(b"half")
    assert any(not ok for _, ok, _ in updater.check_a3(bad_dir, OLD))


# --- download and a fresh copy -----------------------------------------------------------------
def test_fresh_copy_makes_a_new_folder_and_never_reuses_one(tmp_path):
    src = tmp_path / OLD
    src.write_bytes(b"exe")
    first = updater.fresh_copy(src, tmp_path / "runs", "A1")
    assert first.read_bytes() == b"exe" and first.parent.name.startswith("A1")
    second = updater.fresh_copy(src, tmp_path / "runs", "A1")
    assert second.parent != first.parent


def test_download_checks_the_hash(monkeypatch, tmp_path):
    monkeypatch.setenv("RUNBOOK_DRYRUN", "1")
    good = hashlib.sha256((FIX / "fake_app_v0.6.0.bin").read_bytes()).hexdigest()
    path = updater.download("https://example.invalid/a.exe", tmp_path / "a.exe", good, fixture="fake_app_v0.6.0.bin")
    assert sha(path) == good
    with pytest.raises(ValueError, match="hash"):
        updater.download("https://example.invalid/a.exe", tmp_path / "b.exe", "0" * 64, fixture="fake_app_v0.6.0.bin")
    assert not (tmp_path / "b.exe").exists()  # a file that fails the hash is not kept


# --- the dry-run stand-in for the app ---------------------------------------------------------
def test_the_fake_backup_signature_is_the_backup_key():
    sig = (FIX / "fake_backup.sig").read_text()
    assert updater.is_backup_key(updater.signature_key_id(sig))
    assert updater.signature_version(sig) == "0.6.0"


def test_simulate_refuses_to_run_outside_a_dry_run(monkeypatch, tmp_path):
    monkeypatch.delenv("RUNBOOK_DRYRUN", raising=False)
    with pytest.raises(RuntimeError):
        updater.simulate_app("updated", tmp_path / OLD, "v0.6.1")


def test_simulated_updated_state_passes_a1(monkeypatch, tmp_path):
    monkeypatch.setenv("RUNBOOK_DRYRUN", "1")
    exe = tmp_path / "app" / OLD
    exe.parent.mkdir()
    exe.write_bytes((FIX / "fake_app_v0.6.0.bin").read_bytes())
    updater.simulate_app("updated", exe, "v0.6.1")
    old_sha = sha(FIX / "fake_app_v0.6.0.bin")
    new_sha = sha(FIX / "fake_app_v0.6.1.bin")
    assert all(ok for _, ok, _ in updater.check_a1(exe.parent, exe.name, old_sha, new_sha))


def test_simulated_downloaded_state_has_an_update_temp_exe(monkeypatch, tmp_path):
    monkeypatch.setenv("RUNBOOK_DRYRUN", "1")
    exe = tmp_path / OLD
    exe.write_bytes(b"old")
    updater.simulate_app("downloaded", exe, "v0.6.1")
    assert updater.leftovers(tmp_path, OLD)["temp"]


def test_simulated_nothing_changes_nothing(monkeypatch, tmp_path):
    monkeypatch.setenv("RUNBOOK_DRYRUN", "1")
    exe = tmp_path / OLD
    exe.write_bytes(b"old")
    updater.simulate_app("nothing", exe, "v0.6.1")
    assert sorted(p.name for p in tmp_path.iterdir()) == [OLD]


def test_an_unknown_scenario_is_an_error(monkeypatch, tmp_path):
    monkeypatch.setenv("RUNBOOK_DRYRUN", "1")
    with pytest.raises(ValueError):
        updater.simulate_app("exploded", tmp_path / OLD, "v0.6.1")


# --- the key file (A5): contents go in TAURI_SIGNING_PRIVATE_KEY, as candidate.yml does ------------------
def test_read_key_file_returns_the_contents(tmp_path):
    key = tmp_path / "backup.key"
    key.write_text("untrusted comment: rsign encrypted secret key\nRWRT...\n", encoding="utf-8")
    assert updater.read_key_file(str(key)).startswith("untrusted comment: rsign encrypted secret key")


def test_read_key_file_with_no_path_says_so():
    with pytest.raises(ValueError, match="no key path"):
        updater.read_key_file("")


def test_read_key_file_for_a_missing_file_names_the_path(tmp_path):
    with pytest.raises(ValueError, match="does not exist"):
        updater.read_key_file(str(tmp_path / "nope.key"))


def test_read_key_file_for_an_empty_file_is_an_error(tmp_path):
    key = tmp_path / "empty.key"
    key.write_text("  \n")
    with pytest.raises(ValueError, match="empty"):
        updater.read_key_file(str(key))


def test_a_folder_is_not_a_key_file(tmp_path):
    with pytest.raises(ValueError, match="folder"):
        updater.read_key_file(str(tmp_path))


def test_simulated_stalled_download_leaves_only_a_part_file(monkeypatch, tmp_path):
    monkeypatch.setenv("RUNBOOK_DRYRUN", "1")
    exe = tmp_path / OLD
    exe.write_bytes(b"old")
    updater.simulate_app("stalled", exe, "v0.6.1")
    assert updater.leftovers(tmp_path, OLD) == {"old": False, "temp": False, "part": True}
    assert all(ok for _, ok, _ in updater.check_a3(tmp_path, OLD))


def test_a_refusal_shows_the_apps_own_message_not_cargos(monkeypatch):
    cargo_noise = (
        "    Finished `release` profile [optimized] target(s) in 0.4s\n"
        "     Running `target\\release\\examples\\verify_update.exe sign-me.bin sign-me.bin.sig 0.6.9`\n"
        "verify_update: signature is for version 0.6.0, not 0.6.9\n"
        "error: process didn't exit successfully: `verify_update.exe ...` (exit code: 1)\n"
    )
    ok, evidence = updater.parse_verify(1, cargo_noise, "0.6.9")
    assert not ok
    assert evidence == "verify_update: signature is for version 0.6.0, not 0.6.9"


# --- start the copy ourselves, and find where an update really landed ------------------------------------
def test_launch_starts_the_program_in_its_own_folder(tmp_path):
    import subprocess
    marker = tmp_path / "ran-here.txt"
    code = f"import os; open({str(marker)!r}, 'w').write(os.getcwd())"
    pid = updater.launch(Path(sys.executable), args=["-c", code], cwd=tmp_path)
    assert isinstance(pid, int)
    for _ in range(100):
        if marker.exists():
            break
        import time; time.sleep(0.05)
    assert Path(marker.read_text()).resolve() == tmp_path.resolve()


def test_launch_in_a_dry_run_starts_nothing(monkeypatch, tmp_path):
    monkeypatch.setenv("RUNBOOK_DRYRUN", "1")
    assert updater.launch(tmp_path / "app.exe") is None


def test_launch_of_a_missing_exe_is_a_plain_error(tmp_path):
    with pytest.raises(ValueError, match="does not exist"):
        updater.launch(tmp_path / "nope.exe")


def test_parse_process_paths_reads_powershell_json():
    one = '{"Name": "Resonance-Stream-v0.6.0", "Path": "C:\\\\a\\\\A1-4\\\\Resonance-Stream-v0.6.0.exe", "Id": 11}'
    assert updater.parse_process_paths(one) == [r"C:\a\A1-4\Resonance-Stream-v0.6.0.exe"]
    many = '[{"Name":"x","Path":"C:\\\\a\\\\x.exe","Id":1},{"Name":"y","Path":"C:\\\\b\\\\y.exe","Id":2}]'
    assert updater.parse_process_paths(many) == [r"C:\a\x.exe", r"C:\b\y.exe"]
    assert updater.parse_process_paths("") == []


def test_parse_process_paths_garbage_is_an_error():
    with pytest.raises(ValueError):
        updater.parse_process_paths("Get-Process : access denied")


def test_same_file_ignores_case_and_slash_style():
    assert updater.same_file(r"C:\Users\Kade\a\x.exe", "c:/users/kade/a/X.EXE")
    assert not updater.same_file(r"C:\a\x.exe", r"C:\b\x.exe")


def test_scan_copies_shows_where_an_update_landed(tmp_path):
    runs = tmp_path / "updater"
    for name, exe, old, temp in [("A1-3", b"old", False, False), ("A1-4", b"new", True, False), ("A2-4", b"old", False, True)]:
        d = runs / name
        d.mkdir(parents=True)
        (d / OLD).write_bytes(exe)
        if old:
            (d / (OLD + ".old")).write_bytes(b"old")
        if temp:
            (d / "update_temp.exe").write_bytes(b"x")
    rows = {r["folder"]: r for r in updater.scan_copies(runs)}
    assert set(rows) == {"A1-3", "A1-4", "A2-4"}
    assert rows["A1-4"]["old"] and not rows["A1-3"]["old"] and rows["A2-4"]["temp"]
    assert rows["A1-4"]["sha"] == hashlib.sha256(b"new").hexdigest()[:12]


def test_scan_copies_of_a_missing_folder_is_empty(tmp_path):
    assert updater.scan_copies(tmp_path / "nope") == []


# --- the app only offers an update once setup is done (init_done) ------------------------------------------
def test_the_config_path_is_under_appdata_with_the_apps_identifier():
    p = updater.app_config_path({"APPDATA": r"C:\Users\kade\AppData\Roaming"})
    assert p.name == "config.json" and p.parent.name == "com.enjay.bpsr.resonance-stream"


def test_the_config_path_needs_appdata():
    with pytest.raises(ValueError, match="APPDATA"):
        updater.app_config_path({})


def test_setup_state(tmp_path):
    cfg = tmp_path / "config.json"
    assert updater.setup_state(cfg) == "missing"
    cfg.write_text('{"init_done": false, "theme": "light"}')
    assert updater.setup_state(cfg) == "not_done"
    cfg.write_text('{"theme": "light"}')
    assert updater.setup_state(cfg) == "not_done"
    cfg.write_text('{"init_done": true}')
    assert updater.setup_state(cfg) == "ready"


def test_an_unreadable_config_is_an_error_not_a_guess(tmp_path):
    cfg = tmp_path / "config.json"
    cfg.write_text("{not json")
    with pytest.raises(ValueError, match="not valid JSON"):
        updater.setup_state(cfg)


def test_mark_setup_done_creates_a_minimal_config_and_undo_removes_it(tmp_path):
    cfg = tmp_path / "app" / "config.json"
    undo = updater.mark_setup_done(cfg)
    assert json.loads(cfg.read_text()) == {"init_done": True}
    updater.undo_setup(undo)
    assert not cfg.exists()


def test_mark_setup_done_keeps_the_other_keys_and_undo_restores_the_exact_file(tmp_path):
    cfg = tmp_path / "config.json"
    original = '{\n  "init_done": false,\n  "theme": "light",\n  "favorite_messages": []\n}\n'
    cfg.write_text(original, encoding="utf-8")
    undo = updater.mark_setup_done(cfg)
    now = json.loads(cfg.read_text())
    assert now["init_done"] is True and now["theme"] == "light" and now["favorite_messages"] == []
    updater.undo_setup(undo)
    assert cfg.read_text(encoding="utf-8") == original
    assert not (tmp_path / "config.json.runbook-backup").exists()


def test_a_ready_config_is_left_alone(tmp_path):
    cfg = tmp_path / "config.json"
    cfg.write_text('{"init_done": true, "theme": "dark"}')
    assert updater.mark_setup_done(cfg) is None
    assert cfg.read_text() == '{"init_done": true, "theme": "dark"}'
    updater.undo_setup(None)  # nothing to undo is fine


def test_an_existing_backup_is_never_overwritten(tmp_path):
    cfg = tmp_path / "config.json"
    cfg.write_text('{"init_done": false}')
    (tmp_path / "config.json.runbook-backup").write_text("precious")
    with pytest.raises(ValueError, match="backup"):
        updater.mark_setup_done(cfg)
    assert (tmp_path / "config.json.runbook-backup").read_text() == "precious"
    assert cfg.read_text() == '{"init_done": false}'


def test_an_unreadable_existing_config_is_not_touched(tmp_path):
    cfg = tmp_path / "config.json"
    cfg.write_text("{broken")
    with pytest.raises(ValueError):
        updater.mark_setup_done(cfg)
    assert cfg.read_text() == "{broken"


# --- closing the copies a step started ------------------------------------------------------------
def test_parse_processes_keeps_path_and_id():
    one = '{"Name":"Resonance-Stream-v0.6.0","Path":"C:\\\\a\\\\A1-4\\\\x.exe","Id":123}'
    assert updater.parse_processes(one) == [{"Path": "C:\\a\\A1-4\\x.exe", "Id": 123}]
    many = '[{"Path":"C:\\\\a\\\\x.exe","Id":1},{"Path":"C:\\\\b\\\\y.exe","Id":2},{"Path":null,"Id":3},{"Path":"C:\\\\c.exe"}]'
    assert [p["Id"] for p in updater.parse_processes(many)] == [1, 2]
    assert updater.parse_processes("  ") == []
    with pytest.raises(ValueError):
        updater.parse_processes("Get-Process : access denied")


def test_processes_in_a_folder_means_the_folder_and_below_not_a_lookalike():
    procs = [
        {"Path": r"C:\runs\A1-9\Resonance-Stream-v0.6.0.exe", "Id": 1},
        {"Path": r"C:\runs\A1-9\sub\deep.exe", "Id": 2},
        {"Path": r"C:\runs\A1-90\Resonance-Stream-v0.6.0.exe", "Id": 3},
        {"Path": r"C:\runs\A1-9.old\x.exe", "Id": 4},
        {"Path": r"C:\runs\A2-1\x.exe", "Id": 5},
    ]
    assert [p["Id"] for p in updater.processes_in(r"C:\runs\A1-9", procs)] == [1, 2]
    assert [p["Id"] for p in updater.processes_in("c:/RUNS/a1-9/", procs)] == [1, 2]
    assert [p["Id"] for p in updater.processes_in(r"C:\runs", procs)] == [1, 2, 3, 4, 5]


class _Time:
    def __init__(self):
        self.now = 0.0

    def clock(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds


def test_stop_copies_kills_what_runs_in_the_folder_and_waits_for_it_to_go():
    inside = [{"Path": r"C:\runs\A1-9\x.exe", "Id": 11}, {"Path": r"C:\runs\A1-9\y.exe", "Id": 12}]
    outside = [{"Path": r"C:\runs\A2-1\x.exe", "Id": 13}]
    state = {"alive": inside + outside}
    killed = []

    def kill(pid):
        killed.append(pid)
        state["alive"] = [p for p in state["alive"] if p["Id"] != pid]

    t = _Time()
    stopped = updater.stop_copies(r"C:\runs\A1-9", list_processes=lambda: list(state["alive"]), kill=kill,
                                  sleep=t.sleep, clock=t.clock)
    assert sorted(stopped) == [11, 12] and sorted(killed) == [11, 12]
    assert [p["Id"] for p in state["alive"]] == [13], "the other copy is left alone"


def test_stop_copies_with_nothing_running_kills_nothing():
    t = _Time()
    stopped = updater.stop_copies(r"C:\runs\A1-9", list_processes=lambda: [{"Path": r"C:\other\x.exe", "Id": 1}],
                                  kill=lambda pid: pytest.fail("nothing to kill"), sleep=t.sleep, clock=t.clock)
    assert stopped == []


def test_stop_copies_gives_up_waiting_but_does_not_raise():
    stuck = [{"Path": r"C:\runs\A1-9\x.exe", "Id": 11}]
    t = _Time()
    stopped = updater.stop_copies(r"C:\runs\A1-9", list_processes=lambda: stuck, kill=lambda pid: None,
                                  sleep=t.sleep, clock=t.clock, timeout=5)
    assert stopped == [11] and 5 <= t.now <= 7


def test_stop_copies_survives_a_failing_process_list():
    def broken():
        raise ValueError("no powershell")

    assert updater.stop_copies(r"C:\runs\A1-9", list_processes=broken, kill=lambda pid: None) == []


def test_stop_copies_in_a_dry_run_touches_nothing(monkeypatch):
    monkeypatch.setenv("RUNBOOK_DRYRUN", "1")
    assert updater.stop_copies(r"C:\runs\A1-9") == []


def _rec():
    from runbook import common

    return common.Recorder("t")


def test_a_step_closes_the_app_when_it_goes_well():
    rec, calls = _rec(), []
    with updater.step(rec, "A1", r"C:\runs\A1-9", stop=calls.append):
        calls.append("body")
    assert calls == ["body", r"C:\runs\A1-9"] and rec.rows == []


def test_a_step_that_raises_is_a_failed_check_and_still_closes_the_app():
    rec, calls = _rec(), []
    with updater.step(rec, "A1", r"C:\runs\A1-9", stop=calls.append):
        raise RuntimeError("boom")
    # the notebook goes on: the exception did not escape the with block
    assert calls == [r"C:\runs\A1-9"]
    assert [(r.check, r.status) for r in rec.rows] == [("A1-error", "fail")]
    assert "RuntimeError: boom" in rec.rows[0].evidence


def test_ctrl_c_still_closes_the_app_and_is_not_swallowed():
    rec, calls = _rec(), []
    with pytest.raises(KeyboardInterrupt):
        with updater.step(rec, "A1", r"C:\runs\A1-9", stop=calls.append):
            raise KeyboardInterrupt
    assert calls == [r"C:\runs\A1-9"]


def test_a_step_first_closes_copies_an_earlier_step_left_running():
    rec, calls = _rec(), []
    with updater.step(rec, "A2", r"C:\runs\A2-1", strays=r"C:\runs", stop=calls.append):
        calls.append("body")
    assert calls == [r"C:\runs", "body", r"C:\runs\A2-1"]


def test_an_app_that_cannot_be_closed_does_not_hide_the_result():
    rec = _rec()

    def broken_stop(folder):
        raise OSError("access denied")

    with updater.step(rec, "A1", r"C:\runs\A1-9", stop=broken_stop):
        pass
    assert [(r.check, r.status) for r in rec.rows] == [("A1-close", "fail")]

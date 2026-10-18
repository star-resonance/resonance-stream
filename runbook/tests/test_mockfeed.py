import hashlib
import http.client
import json
import os
import socket
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from runbook import mockfeed  # noqa: E402


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


# --- the files the app reads ------------------------------------------------------------------
def test_feed_json_has_what_the_app_reads():
    feed = json.loads(mockfeed.feed_json("9.9.9", "http://127.0.0.1:1/update.exe", "c2ln", notes="n"))
    # src-tauri parse_feed: version, url, signature are required; notes is optional.
    assert feed["version"] == "9.9.9"
    assert feed["url"] == "http://127.0.0.1:1/update.exe"
    assert feed["signature"] == "c2ln"
    assert feed["notes"] == "n"


def test_metadata_json_has_the_gist_shape():
    meta = json.loads(mockfeed.metadata_json("http://127.0.0.1:1/model.gguf"))
    # resonance_types::GistMetadata: model is a VersionInfo, dictionary a RemoteDictionary.
    assert set(meta["model"]) >= {"latest_version", "download_url", "release_notes"}
    assert set(meta["dictionary"]) == {"version", "updated_at"}
    assert meta["model"]["download_url"] == "http://127.0.0.1:1/model.gguf"


# --- the command line -------------------------------------------------------------------------
def test_flag_args_make_an_isolated_run():
    args = mockfeed.flag_args(r"C:\runbook\run1", r"C:\runbook\run1-status.json")
    assert args[:2] == ["--data-dir", r"C:\runbook\run1"]
    for flag in ("--fresh", "--assume-setup-done", "--no-capture", "--no-translator", "--no-popups", "--no-window-state"):
        assert flag in args
    assert args[args.index("--status-file") + 1] == r"C:\runbook\run1-status.json"
    # The update check is the thing under test: never switched off here.
    assert "--no-update-check" not in args
    assert "--feed-url" not in args and "--metadata-url" not in args


def test_flag_args_carry_the_mock_urls_and_the_log_file():
    args = mockfeed.flag_args("d", "s", feed_url="http://127.0.0.1:5/latest.json",
                              metadata_url="http://127.0.0.1:5/metadata.json", log_file="app.log")
    assert args[args.index("--feed-url") + 1] == "http://127.0.0.1:5/latest.json"
    assert args[args.index("--metadata-url") + 1] == "http://127.0.0.1:5/metadata.json"
    assert args[args.index("--log-file") + 1] == "app.log"


def test_flag_args_carry_the_dictionary_url():
    args = mockfeed.flag_args("d", "s", dictionary_url="http://127.0.0.1:5/custom_dict.json")
    assert args[args.index("--dictionary-url") + 1] == "http://127.0.0.1:5/custom_dict.json"
    assert "--dictionary-url" not in mockfeed.flag_args("d", "s")


def test_the_mock_serves_a_dictionary_and_counts_who_asked_for_it():
    import urllib.request

    server = mockfeed.MockServer().start()
    try:
        assert server.dictionary_url == server.base_url + "/custom_dict.json"
        server.dictionary_text = '{"term": {"ボス": "보스"}}'
        body = urllib.request.urlopen(server.dictionary_url, timeout=5).read().decode("utf-8")  # noqa: S310 -- local
        assert body == '{"term": {"ボス": "보스"}}'
        assert server.hits.count("GET /custom_dict.json") == 1
    finally:
        server.stop()


def test_flag_args_can_leave_out_fresh():
    assert "--fresh" not in mockfeed.flag_args("d", "s", fresh=False)


def test_every_flag_is_one_the_app_knows():
    known = {"--data-dir", "--fresh", "--assume-setup-done", "--no-capture", "--no-translator",
             "--no-update-check", "--no-popups", "--no-window-state", "--feed-url", "--metadata-url",
             "--dictionary-url", "--status-file", "--log-file", "--print-env"}
    args = mockfeed.flag_args("d", "s", feed_url="http://127.0.0.1:5/f", metadata_url="http://127.0.0.1:5/m",
                              dictionary_url="http://127.0.0.1:5/d", log_file="l")
    assert {a for a in args if a.startswith("--")} <= known


# --- a "new version" exe ----------------------------------------------------------------------
def test_make_new_exe_differs_from_the_source_and_keeps_it(tmp_path):
    src = tmp_path / "app.exe"
    src.write_bytes(b"MZ-original")
    new = mockfeed.make_new_exe(src, tmp_path / "new" / "update.exe")
    assert new.read_bytes().startswith(b"MZ-original") and new.read_bytes() != b"MZ-original"
    assert src.read_bytes() == b"MZ-original"


# --- the mock server --------------------------------------------------------------------------
def get(url, timeout=5):
    with urllib.request.urlopen(url, timeout=timeout) as r:  # noqa: S310 -- 127.0.0.1 test server
        return r.status, r.read()


def test_the_server_serves_feed_metadata_and_exe():
    exe = b"new exe bytes" * 100
    with mockfeed.MockServer(exe_bytes=exe, signature="c2ln", version="9.9.9") as server:
        status, body = get(server.feed_url)
        feed = json.loads(body)
        assert status == 200 and feed["version"] == "9.9.9" and feed["signature"] == "c2ln"
        assert feed["url"] == server.exe_url and server.exe_url.startswith("http://127.0.0.1:")
        assert get(server.exe_url)[1] == exe
        assert "model" in json.loads(get(server.metadata_url)[1])
        assert [h.split()[-1] for h in server.hits] == ["/latest.json", "/update.exe", "/metadata.json"]


# --- the signed metadata -----------------------------------------------------------------------
def test_signed_metadata_names_a_revision_and_the_dictionarys_hash():
    text = json.loads(mockfeed.metadata_json("http://127.0.0.1:1/m.gguf", revision=7, dictionary_sha256="ab" * 32))
    assert text["revision"] == 7 and text["dictionary"]["sha256"] == "ab" * 32
    # the old gist's shape has neither (nothing else about it changed)
    old = json.loads(mockfeed.metadata_json())
    assert "revision" not in old and "sha256" not in old["dictionary"]


def test_flag_args_carry_the_metadata_trust_key():
    args = mockfeed.flag_args("d", "s", metadata_url="http://127.0.0.1:5/m", metadata_trust_key="a2V5")
    assert args[args.index("--metadata-trust-key") + 1] == "a2V5"
    assert "--metadata-trust-key" not in mockfeed.flag_args("d", "s", metadata_url="http://127.0.0.1:5/m")


@pytest.fixture
def dry(monkeypatch):
    monkeypatch.setenv("RUNBOOK_DRYRUN", "1")


def test_a_signed_server_serves_the_metadata_its_signature_and_the_dictionary_it_names(dry, tmp_path):
    signer = mockfeed.MetadataSigner(tmp_path / "k", "good")
    with mockfeed.MockServer(signer=signer) as server:
        body = get(server.metadata_url)[1]
        metadata = json.loads(body)
        assert metadata["revision"] == 1
        assert metadata["dictionary"]["sha256"] == sha(server.dictionary_text.encode("utf-8"))
        assert get(server.metadata_url + ".sig")[0] == 200
        assert server.metadata_flags() == ("--metadata-url", server.metadata_url, "--metadata-trust-key", signer.public_key)
        server.revision = 5
        assert json.loads(get(server.metadata_url)[1])["revision"] == 5


def test_a_server_without_a_signer_publishes_no_signature(dry):
    with mockfeed.MockServer() as server:
        with pytest.raises(urllib.error.HTTPError) as err:
            get(server.metadata_url + ".sig")
        assert err.value.code == 404
        assert server.metadata_flags() == ("--metadata-url", server.metadata_url)


def test_the_metadata_modes_break_it_one_way_each(dry, tmp_path):
    good, other = mockfeed.MetadataSigner(tmp_path / "g", "good"), mockfeed.MetadataSigner(tmp_path / "o", "other")
    with mockfeed.MockServer(signer=good, foreign_signer=other) as server:
        honest_sig = get(server.metadata_url + ".sig")[1]
        server.metadata_mode = "no-signature"
        with pytest.raises(urllib.error.HTTPError):
            get(server.metadata_url + ".sig")
        server.metadata_mode = "wrong-key"
        assert get(server.metadata_url + ".sig")[1] != honest_sig  # signed by the other key
        server.metadata_mode = "tampered"
        assert get(server.metadata_url + ".sig")[1] == honest_sig
        assert json.loads(get(server.metadata_url)[1])["model"]["latest_version"] == "runbook-evil"  # but the text is not what was signed
        server.metadata_mode = "ok"
        server.dictionary_override = "{}"
        assert get(server.dictionary_url)[1] == b"{}"  # no longer the file the signed metadata names
        assert json.loads(get(server.metadata_url)[1])["dictionary"]["sha256"] == sha(server.dictionary_text.encode("utf-8"))


def test_a_wrong_key_server_needs_a_second_key(dry, tmp_path):
    with mockfeed.MockServer(signer=mockfeed.MetadataSigner(tmp_path / "g", "good")) as server:
        server.metadata_mode = "wrong-key"
        with pytest.raises((urllib.error.URLError, http.client.HTTPException, ConnectionError)):  # the handler fails: no foreign signer to sign with
            get(server.metadata_url + ".sig")


@pytest.mark.skipif(__import__("shutil").which("npx") is None, reason="needs npx (and the network the first time)")
def test_the_real_signer_signs_for_the_revision_it_is_given(tmp_path):
    from runbook import updater
    signer = mockfeed.MetadataSigner(tmp_path / "k", "real")
    sig = signer.sign(b'{"revision": 3}', 3)
    assert updater.signature_version(sig) == "3"  # the signed comment the app compares with the file's revision
    assert updater.signature_key_id(sig)
    assert signer.public_key and signer.public_key != sig


@pytest.mark.skipif(__import__("shutil").which("npx") is None, reason="needs npx (and the network the first time)")
def test_a_second_signer_in_the_same_folder_makes_a_fresh_key(tmp_path):
    # A run folder outlives its run: the next run of the same pipeline makes its key there again.
    first = mockfeed.MetadataSigner(tmp_path / "k", "good")
    again = mockfeed.MetadataSigner(tmp_path / "k", "good")
    assert again.public_key and again.public_key != first.public_key


def test_feed_modes():
    with mockfeed.MockServer(exe_bytes=b"x") as server:
        server.feed_mode = "garbage"
        status, body = get(server.feed_url)
        assert status == 200
        with pytest.raises(ValueError):
            json.loads(body)
        server.feed_mode = "404"
        with pytest.raises(urllib.error.HTTPError) as e:
            get(server.feed_url)
        assert e.value.code == 404


def test_the_version_can_be_changed_while_running():
    with mockfeed.MockServer(exe_bytes=b"x", version="9.9.9") as server:
        server.version = "0.0.1"
        assert json.loads(get(server.feed_url)[1])["version"] == "0.0.1"


def test_a_cut_download_ends_early_against_its_content_length():
    exe = b"y" * 100_000
    with mockfeed.MockServer(exe_bytes=exe) as server:
        server.exe_mode = "cut"
        with pytest.raises(http.client.IncompleteRead) as e:
            get(server.exe_url)
        assert 0 < len(e.value.partial) < len(exe)


def test_a_stalled_download_goes_silent_and_the_server_still_stops():
    exe = b"z" * 100_000
    server = mockfeed.MockServer(exe_bytes=exe)
    server.start()
    try:
        server.exe_mode = "stall"
        response = urllib.request.urlopen(server.exe_url, timeout=1)  # noqa: S310 -- local
        with pytest.raises((socket.timeout, TimeoutError)):
            response.read()
    finally:
        server.stop()  # must not hang on the stalled connection


def test_an_unknown_path_is_404():
    with mockfeed.MockServer(exe_bytes=b"x") as server:
        with pytest.raises(urllib.error.HTTPError) as e:
            get(server.base_url + "/nope")
        assert e.value.code == 404


def test_a_closed_server_refuses_connections():
    with mockfeed.MockServer(exe_bytes=b"x") as server:
        url = server.feed_url
    with pytest.raises(urllib.error.URLError):
        get(url, timeout=2)


# --- the status file ------------------------------------------------------------------------
def test_read_status_tolerates_a_missing_or_half_written_file(tmp_path):
    path = tmp_path / "s.json"
    assert mockfeed.read_status(path) is None
    path.write_text('{"ready": tru', encoding="utf-8")
    assert mockfeed.read_status(path) is None
    path.write_text("[1, 2]", encoding="utf-8")
    assert mockfeed.read_status(path) is None
    path.write_text('{"ready": true, "update": "none"}', encoding="utf-8")
    assert mockfeed.read_status(path) == {"ready": True, "update": "none"}


class FakeClock:
    def __init__(self):
        self.now = 0.0

    def clock(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds


def test_wait_status_returns_the_first_status_that_fits(tmp_path):
    path = tmp_path / "s.json"
    fake = FakeClock()
    steps = iter([None, {"ready": False}, {"ready": True, "pid": 7}])

    def reader(_):
        return next(steps)

    got = mockfeed.wait_status(path, lambda s: s.get("ready"), timeout=60, interval=1,
                               clock=fake.clock, sleep=fake.sleep, reader=reader)
    assert got == {"ready": True, "pid": 7} and fake.now == 2.0


def test_wait_status_gives_up_at_the_timeout(tmp_path):
    fake = FakeClock()
    got = mockfeed.wait_status(tmp_path / "s.json", lambda s: True, timeout=10, interval=2,
                               clock=fake.clock, sleep=fake.sleep)
    assert got is None and 10 <= fake.now <= 12


def test_update_kind_splits_the_state():
    assert mockfeed.update_kind("none") == ("none", "")
    assert mockfeed.update_kind("available:9.9.9") == ("available", "9.9.9")
    assert mockfeed.update_kind("downloading") == ("downloading", "")
    assert mockfeed.update_kind("downloaded") == ("downloaded", "")
    assert mockfeed.update_kind("error:Network error: x") == ("error", "Network error: x")
    assert mockfeed.update_kind(None) == ("none", "")


# --- small helpers ----------------------------------------------------------------------------
def test_fingerprint_notices_a_change(tmp_path):
    path = tmp_path / "config.json"
    assert mockfeed.fingerprint(path) is None
    path.write_text("a", encoding="utf-8")
    first = mockfeed.fingerprint(path)
    assert first is not None
    assert mockfeed.fingerprint(path) == first
    path.write_text("b", encoding="utf-8")
    assert mockfeed.fingerprint(path) != first


def test_kill_command_stops_the_process_tree():
    assert mockfeed.kill_command(123) == ["taskkill", "/PID", "123", "/T", "/F"]


def test_log_tail_is_the_last_lines(tmp_path):
    path = tmp_path / "app.log"
    assert mockfeed.log_tail(path) == ""
    path.write_text("\n".join(f"line {i}" for i in range(30)), encoding="utf-8")
    tail = mockfeed.log_tail(path, lines=3)
    assert tail.splitlines() == ["line 27", "line 28", "line 29"]


# --- starting the app (a script stands in for the exe) ------------------------------------------
posix_only = pytest.mark.skipif(sys.platform == "win32", reason="the stand-in app is a shell script")


def _script(tmp_path, body, name="app.exe"):
    path = tmp_path / name
    path.write_text("#!/bin/sh\n" + body + "\n", encoding="utf-8")
    path.chmod(0o755)
    return path


@posix_only
def test_start_app_launches_it_in_its_own_folder_and_returns_the_pid(tmp_path):
    exe = _script(tmp_path, 'pwd > "$1"; sleep 0.2')
    out = tmp_path / "cwd.txt"
    pid = mockfeed.start_app(exe, [str(out)])
    assert isinstance(pid, int) and pid > 0
    for _ in range(50):
        if out.exists() and out.read_text().strip():
            break
        time.sleep(0.1)
    assert out.read_text().strip() == str(tmp_path)


def test_start_app_refuses_a_missing_exe(tmp_path):
    with pytest.raises(ValueError):
        mockfeed.start_app(tmp_path / "nope.exe", [])


@posix_only
def test_print_env_run_returns_the_exit_code_and_does_not_hang_forever(tmp_path):
    quick = _script(tmp_path, "echo hi; exit 0", "quick.exe")
    code, out = mockfeed.run_print_env(quick, tmp_path / "data", tmp_path / "s.json", timeout=10)
    assert code == 0 and "hi" in out
    # an exe that ignores the flags starts its window and never exits: killed at the timeout
    stuck = _script(tmp_path, "sleep 30", "stuck.exe")
    code, out = mockfeed.run_print_env(stuck, tmp_path / "data", tmp_path / "s2.json", timeout=1)
    assert code == 124 and "timed out" in out


def test_feed_signature_is_the_sig_text_without_line_breaks():
    assert mockfeed.feed_signature("dW50\r\ncnVzdGVk\n") == "dW50cnVzdGVk"
    assert mockfeed.feed_signature("  abc \n") == "abc"


@posix_only
def test_stop_app_ends_the_process(tmp_path):
    exe = _script(tmp_path, "sleep 30")
    pid = mockfeed.start_app(exe, [])
    mockfeed.stop_app(pid)
    for _ in range(50):
        try:
            done, _status = os.waitpid(pid, os.WNOHANG)
        except ChildProcessError:
            break
        if done:
            break
        time.sleep(0.1)
    else:
        pytest.fail("the process is still running")


def test_stop_app_ignores_nothing_and_unknown_pids():
    mockfeed.stop_app(None)
    mockfeed.stop_app(0)
    mockfeed.stop_app(2**22 + 12345)  # no such process: no error


# --- closing the stand-in apps (dry run support) -------------------------------------------------
def _stand_in(folder, name="app.exe"):
    """A process that looks to `posix_processes` like a copy of the app in `folder`."""
    import subprocess

    folder.mkdir(parents=True, exist_ok=True)
    exe = _script(folder, "sleep 30", name)
    proc = subprocess.Popen([str(exe)], env={**os.environ, "FAKE_APP_EXE": str(exe)})
    return exe, proc


def _gone(proc, wait=5.0):
    end = time.monotonic() + wait
    while time.monotonic() < end:
        if proc.poll() is not None:
            return True
        time.sleep(0.05)
    return False


@posix_only
def test_posix_processes_finds_a_stand_in_app_by_its_exe(tmp_path):
    exe, proc = _stand_in(tmp_path / "A1-1")
    try:
        assert {"Path": str(exe), "Id": proc.pid} in mockfeed.posix_processes()
    finally:
        proc.kill()
        proc.wait()
    assert all(p["Id"] != proc.pid for p in mockfeed.posix_processes())


@posix_only
def test_stop_copies_closes_the_copies_in_a_folder_and_leaves_the_others(tmp_path):
    _, mine = _stand_in(tmp_path / "A1-9")
    _, other = _stand_in(tmp_path / "A1-90")
    try:
        stopped = mockfeed.stop_copies(tmp_path / "A1-9")
        # the copy and anything it started (the script's `sleep` carries the marker too)
        assert mine.pid in stopped and other.pid not in stopped
        assert _gone(mine)
        assert other.poll() is None, "A1-90 is not A1-9"
    finally:
        other.kill()
        other.wait()
        mine.kill()


@posix_only
def test_stop_copies_with_nothing_running_is_quiet(tmp_path):
    assert mockfeed.stop_copies(tmp_path / "nothing-here") == []


def test_a_server_can_be_stopped_twice():
    server = mockfeed.MockServer(exe_bytes=b"x").start()
    server.stop()
    server.stop()


def test_flag_args_leave_capture_off_unless_asked():
    base = mockfeed.flag_args("D", "S.json")
    assert "--no-capture" in base
    capture = mockfeed.flag_args("D", "S.json", capture=True)
    assert "--no-capture" not in capture
    # everything else is the same
    assert [a for a in base if a != "--no-capture"] == capture


def test_flag_args_keep_window_state_off_unless_asked():
    base = mockfeed.flag_args("D", "S.json")
    assert "--no-window-state" in base
    kept = mockfeed.flag_args("D", "S.json", window_state=True)
    assert "--no-window-state" not in kept
    assert [a for a in base if a != "--no-window-state"] == kept

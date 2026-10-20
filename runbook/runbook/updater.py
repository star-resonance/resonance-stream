"""K2: the signed update path, checked from the files it leaves behind. Pure helpers.

The app (src-tauri/src/services/downloader/app_updater.rs) keeps the new exe as
`update_temp.exe` next to itself, verifies it, and on 재시작 swaps it in, leaving the old exe as
`<exe>.old`. So every outcome can be read off the folder: which exe is there (by SHA-256, against
the published SHA256SUMS.txt), whether `.old` exists, whether `update_temp.exe` is gone.
"""
from __future__ import annotations

import base64
import contextlib
import hashlib
import re
import shutil
import time
import traceback
from pathlib import Path

BACKUP_KEY_PREFIX = "0099CF71"  # CLAUDE.md / handoff: the offline backup key; primary is 9005FB94...


def sha256_file(path: str | Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def parse_sums(text: str) -> dict[str, str]:
    """SHA256SUMS.txt: `<hash>  <name>` (or `<hash> *<name>`), one per line -> {name: lowercase hash}."""
    out = {}
    for line in text.splitlines():
        m = re.fullmatch(r"\s*([0-9a-fA-F]{64})\s+\*?(\S.*?)\s*", line)
        if m:
            out[m.group(2)] = m.group(1).lower()
    if not out:
        raise ValueError(f"no `<sha256>  <name>` lines in: {text.strip()[:80]!r}")
    return out


def flip_one_byte(path: str | Path, offset: int | None = None) -> tuple[int, int, int]:
    """Invert one byte in place (default: the middle one); returns (offset, before, after).
    Flipping the same offset again restores the file."""
    path = Path(path)
    data = bytearray(path.read_bytes())
    if not data:
        raise ValueError(f"{path} is empty")
    if offset is None:
        offset = len(data) // 2
    if not 0 <= offset < len(data):
        raise ValueError(f"offset {offset} is outside {path} ({len(data)} bytes)")
    before = data[offset]
    data[offset] = before ^ 0xFF
    path.write_bytes(bytes(data))
    return offset, before, data[offset]


def _signature_lines(sig_text: str) -> list[str]:
    try:
        return base64.b64decode(sig_text.strip(), validate=True).decode("utf-8").splitlines()
    except Exception as e:  # noqa: BLE001
        raise ValueError("not a .sig file (base64 of a minisign signature)") from e


def signature_key_id(sig_text: str) -> str:
    """The 16-hex-digit id of the key that signed (how minisign prints it)."""
    lines = _signature_lines(sig_text)
    try:
        raw = base64.b64decode(lines[1])
    except Exception as e:  # noqa: BLE001
        raise ValueError("the signature line is not base64") from e
    if len(raw) != 74 or raw[:2] not in (b"Ed", b"ED"):
        raise ValueError("not a minisign signature")
    return raw[2:10][::-1].hex().upper()


def signature_version(sig_text: str) -> str:
    """`version:` from the signed (trusted) comment -- what the app compares with the announced one."""
    m = re.search(r"version:(\S+)", "\n".join(_signature_lines(sig_text)))
    if not m:
        raise ValueError("no version in the signature's trusted comment")
    return m.group(1)


def is_backup_key(key_id: str) -> bool:
    return key_id.upper().startswith(BACKUP_KEY_PREFIX)


def parse_verify(code: int, text: str, version: str) -> tuple[bool, str]:
    """Output of `cargo run -p resonance-core --example verify_update -- <file> <sig> <version>`."""
    lines = text.splitlines()
    line = next((ln for ln in lines if ln.startswith("ok:")), "")
    ok = code == 0 and f"is signed by an app key for version {version}" in line
    # the app's own message (`verify_update: ...`) beats cargo's "process didn't exit successfully"
    refusal = next((ln for ln in lines if ln.startswith("verify_update:")), "")
    last = text.strip().splitlines()[-1] if text.strip() else f"exit {code}, no output"
    return ok, (line or refusal or last)


def leftovers(folder: str | Path, exe_name: str) -> dict[str, bool]:
    folder = Path(folder)
    return {
        "old": (folder / f"{exe_name}.old").exists(),
        "temp": (folder / "update_temp.exe").exists(),
        "part": (folder / "update_temp.exe.part").exists(),
    }


Check = tuple[str, bool, str]


def check_a1(folder: str | Path, exe_name: str, old_sha: str, new_sha: str) -> list[Check]:
    """After a good update and the restart."""
    folder = Path(folder)
    left = leftovers(folder, exe_name)
    exe = folder / exe_name
    now = sha256_file(exe) if exe.exists() else "missing"
    old_file = folder / f"{exe_name}.old"
    old_now = sha256_file(old_file) if old_file.exists() else "missing"
    return [
        ("the exe is now the published new version", now == new_sha, f"sha256 {now[:16]}, published {new_sha[:16]}"),
        (f"{exe_name}.old is left, and is the old version", left["old"] and old_now == old_sha,
         f"sha256 {old_now[:16]}, old was {old_sha[:16]}"),
        ("update_temp.exe is gone", not left["temp"], "still there" if left["temp"] else ""),
    ]


def check_a2(folder: str | Path, exe_name: str, old_sha: str) -> list[Check]:
    """After pressing 재시작 on a tampered update_temp.exe."""
    folder = Path(folder)
    left = leftovers(folder, exe_name)
    exe = folder / exe_name
    now = sha256_file(exe) if exe.exists() else "missing"
    return [
        ("the exe is still the old version", now == old_sha, f"sha256 {now[:16]}, old was {old_sha[:16]}"),
        (f"no {exe_name}.old was created", not left["old"], "it exists" if left["old"] else ""),
    ]


def check_a3(folder: str | Path, exe_name: str) -> list[Check]:
    """After a download that was cut: a half file must not be called update_temp.exe."""
    left = leftovers(folder, exe_name)
    return [("no half file named update_temp.exe", not left["temp"],
             "update_temp.exe exists" if left["temp"] else (".part file left" if left["part"] else ""))]


def fresh_copy(src: str | Path, runs_dir: str | Path, label: str) -> Path:
    """Copy the exe into a folder of its own that has never been used; returns the copy's path.
    An update replaces the exe, so each check needs an untouched one."""
    src, runs_dir = Path(src), Path(runs_dir)
    n = 1
    while (folder := runs_dir / f"{label}-{n}").exists():
        n += 1
    folder.mkdir(parents=True)
    return Path(shutil.copy2(src, folder / src.name))


def download(url: str, dest: str | Path, expect_sha: str, fixture: str | None = None) -> Path:
    """Download `url` to `dest` and keep it only if its SHA-256 is `expect_sha`.
    A dry run copies the named fixture instead."""
    from runbook import common

    dest = Path(dest)
    dest.parent.mkdir(parents=True, exist_ok=True)
    part = dest.with_suffix(dest.suffix + ".part")
    if common.dry_run():
        shutil.copyfile(common.FIXTURES / fixture, part)
    else:
        import urllib.request

        with urllib.request.urlopen(url, timeout=60) as r, open(part, "wb") as f:  # noqa: S310 -- fixed https URL
            shutil.copyfileobj(r, f)
    got = sha256_file(part)
    if got != expect_sha:
        part.unlink()
        raise ValueError(f"hash of {url} is {got}, expected {expect_sha}")
    part.replace(dest)
    return dest


def simulate_app(scenario: str, exe: str | Path, new_tag: str) -> None:
    """DRY RUN ONLY: leave the folder as the app would, so a notebook's checks can be exercised
    without Windows. `updated` = after a good update + restart; `downloaded` = update_temp.exe
    is there, not yet applied; `stalled` = a cut download left a .part file; `nothing` = the app changed nothing."""
    from runbook import common

    if not common.dry_run():
        raise RuntimeError("simulate_app is for dry runs only")
    exe = Path(exe)
    new_bytes = (common.FIXTURES / f"fake_app_{new_tag}.bin").read_bytes()
    if scenario == "updated":
        shutil.copyfile(exe, exe.with_name(exe.name + ".old"))
        exe.write_bytes(new_bytes)
    elif scenario == "downloaded":
        (exe.parent / "update_temp.exe").write_bytes(new_bytes)
    elif scenario == "stalled":
        (exe.parent / "update_temp.exe.part").write_bytes(new_bytes[: len(new_bytes) // 3])
    elif scenario != "nothing":
        raise ValueError(f"unknown scenario {scenario!r}")


def read_key_file(path: str) -> str:
    """The contents of a Tauri signing key file, for TAURI_SIGNING_PRIVATE_KEY -- the variable
    candidate.yml signs with. (The CLI's *_PATH variant gave "Unable to find the private key"
    on a real run, so the notebook does not depend on it.)"""
    if not path.strip():
        raise ValueError("no key path given -- paste the full path of the backup key file")
    key = Path(path.strip().strip('"'))
    if key.is_dir():
        raise ValueError(f"{key} is a folder, not the key file")
    if not key.exists():
        raise ValueError(f"{key} does not exist")
    text = key.read_text(encoding="utf-8").strip()
    if not text:
        raise ValueError(f"{key} is empty")
    return text


POWERSHELL_PROCESSES = (
    "$p = @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.ProcessName -like '*esonance*' } | "
    "ForEach-Object { [pscustomobject]@{Name=$_.ProcessName; Path=$_.Path; Id=$_.Id} }); "
    "if ($p.Count -gt 0) { ConvertTo-Json -InputObject $p }"
)


def launch(exe: str | Path, args: list[str] | None = None, cwd: str | Path | None = None) -> int | None:
    """Start the copy ourselves, in its own folder -- so it cannot be the wrong exe. This process is
    elevated, so the child is too (no UAC prompt). A dry run starts nothing."""
    from runbook import common

    exe = Path(exe)
    if common.dry_run():
        return None
    if not exe.exists():
        raise ValueError(f"{exe} does not exist")
    import subprocess

    return subprocess.Popen([str(exe), *(args or [])], cwd=str(cwd or exe.parent), close_fds=True).pid


def parse_process_paths(text: str) -> list[str]:
    import json

    text = text.strip().lstrip("﻿")
    if not text:
        return []
    try:
        data = json.loads(text)
    except json.JSONDecodeError as e:
        raise ValueError(f"not the JSON the command prints: {text[:120]!r}") from e
    if isinstance(data, dict):
        data = [data]
    return [d["Path"] for d in data if d.get("Path")]


def parse_processes(text: str) -> list[dict]:
    """The JSON `POWERSHELL_PROCESSES` prints, as [{"Path": ..., "Id": ...}]; entries missing either are dropped."""
    import json

    text = text.strip().lstrip("\ufeff")
    if not text:
        return []
    try:
        data = json.loads(text)
    except json.JSONDecodeError as e:
        raise ValueError(f"not the JSON the command prints: {text[:120]!r}") from e
    if isinstance(data, dict):
        data = [data]
    return [{"Path": d["Path"], "Id": d["Id"]} for d in data if d.get("Path") and d.get("Id")]


def _norm(path: str | Path) -> str:
    return str(path).replace("\\", "/").lower().rstrip("/")


def processes_in(folder: str | Path, processes: list[dict]) -> list[dict]:
    """The processes whose exe is in `folder` or below it -- not a folder that merely starts the same
    (`A1-9` is not `A1-90`). Windows paths ignore case and take either slash."""
    prefix = _norm(folder) + "/"
    return [p for p in processes if _norm(p["Path"]).startswith(prefix)]


def _list_processes() -> list[dict]:
    from runbook import common

    code, out = common.capture(["powershell", "-NoProfile", "-Command", POWERSHELL_PROCESSES])
    if code != 0:
        raise ValueError(f"could not list the processes (exit {code}): {out[:120]}")
    return parse_processes(out)


def _kill(pid: int) -> None:
    from runbook import common

    common.capture(["taskkill", "/PID", str(pid), "/T", "/F"])


def stop_copies(folder: str | Path, *, list_processes=None, kill=None, sleep=time.sleep, clock=time.monotonic,
                timeout: float = 15) -> list[int]:
    """Close every app whose exe is in `folder` (or below), and wait until they are gone -- Windows cannot
    replace or delete an exe that still runs. Returns the pids it stopped. Never raises: a copy that cannot
    be closed is reported by the caller, not an exception. A dry run starts nothing, so closes nothing."""
    from runbook import common

    if common.dry_run() and list_processes is None:
        return []
    list_processes = list_processes or _list_processes
    kill = kill or _kill
    try:
        mine = processes_in(folder, list_processes())
    except (ValueError, OSError) as e:
        print(f"  (could not look for running copies: {e})")
        return []
    pids = [p["Id"] for p in mine]
    for pid in pids:
        kill(pid)
    start = clock()
    while pids and clock() - start < timeout:
        try:
            alive = {p["Id"] for p in processes_in(folder, list_processes())}
        except (ValueError, OSError):
            break
        if not alive & set(pids):
            break
        sleep(0.3)
    return pids


@contextlib.contextmanager
def step(rec, check: str, folder: str | Path, strays: str | Path | None = None, stop=None):
    """One test step, as try / catch / finally: run the app and the checks inside; an exception is a failed
    check (the notebook goes on to the next step), and whatever happens the app is closed afterwards.
    `strays`: copies an earlier step left running (anywhere under this folder) are closed first, so a
    check never looks at the wrong window. Ctrl-C / Interrupt still closes the app and still stops the notebook."""
    stop = stop or stop_copies
    if strays is not None:
        stop(strays)
    try:
        yield
    except Exception as e:  # noqa: BLE001 -- the point: one broken step must not end the notebook
        traceback.print_exc()
        rec.auto(f"{check}-error", "the step stopped with an error", False, f"{type(e).__name__}: {e}")
    finally:
        try:
            stopped = stop(folder)
            if stopped:
                print(f"  (closed the app: pid {', '.join(str(p) for p in stopped)})")
        except Exception as e:  # noqa: BLE001
            rec.auto(f"{check}-close", "the app was closed after the step", False, f"{type(e).__name__}: {e}")


def same_file(a: str | Path, b: str | Path) -> bool:
    """Windows paths ignore case and take either slash."""
    norm = lambda p: str(p).replace("\\", "/").lower().rstrip("/")  # noqa: E731
    return norm(a) == norm(b)


def scan_copies(runs_dir: str | Path) -> list[dict]:
    """Every copy folder under runs/updater: its exe's hash (12 hex), and whether `.old` / `update_temp.exe` are
    there -- shows where an update actually landed when the folder the notebook watched did not change."""
    runs_dir = Path(runs_dir)
    rows = []
    if not runs_dir.is_dir():
        return rows
    for folder in sorted(p for p in runs_dir.iterdir() if p.is_dir()):
        exes = [e for e in folder.glob("Resonance-Stream-*.exe")]
        exe = exes[0] if exes else None
        rows.append({
            "folder": folder.name,
            "sha": sha256_file(exe)[:12] if exe else "-",
            "old": any(folder.glob("*.exe.old")),
            "temp": (folder / "update_temp.exe").exists(),
        })
    return rows


# The update dialog is only offered by an app that has finished setup (hydration.rs: the check runs in the
# `init_done` branch; a new user gets the wizard, which never offers an app update). A fresh copy with no
# config therefore cannot be updated -- the notebook marks setup done, with consent and an undo.
APP_IDENTIFIER = "com.enjay.bpsr.resonance-stream"  # src-tauri/tauri.conf.json
BACKUP_SUFFIX = ".runbook-backup"


def app_config_path(env: dict | None = None) -> Path:
    import os

    env = os.environ if env is None else env
    appdata = env.get("APPDATA")
    if not appdata:
        raise ValueError("APPDATA is not set -- this check is for Windows")
    return Path(appdata) / APP_IDENTIFIER / "config.json"


def _read_config(path: Path) -> dict:
    import json

    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as e:
        raise ValueError(f"{path} is not valid JSON ({e}); not touching it") from e
    if not isinstance(data, dict):
        raise ValueError(f"{path} is not a JSON object; not touching it")
    return data


def setup_state(path: str | Path) -> str:
    """'missing' (no config yet: the app writes defaults, init_done false), 'not_done', or 'ready'."""
    path = Path(path)
    if not path.exists():
        return "missing"
    return "ready" if _read_config(path).get("init_done") is True else "not_done"


def mark_setup_done(path: str | Path) -> dict | None:
    """Make the app skip the first-run wizard: init_done true, every other key kept. The original file is
    backed up first (never over an existing backup). Returns what `undo_setup` needs; None if already ready."""
    import json
    import shutil

    path = Path(path)
    state = setup_state(path)  # raises for an unreadable file, before anything is written
    if state == "ready":
        return None
    backup = path.with_name(path.name + BACKUP_SUFFIX)
    if state == "missing":
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"init_done": True}), encoding="utf-8")
        return {"path": path, "backup": None}
    if backup.exists():
        raise ValueError(f"{backup} already exists: restore or remove that backup first")
    shutil.copy2(path, backup)
    data = _read_config(path)
    data["init_done"] = True
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")
    return {"path": path, "backup": backup}


def undo_setup(undo: dict | None) -> None:
    """Put the config back as it was: the backup restored, or the file this notebook created removed."""
    if not undo:
        return
    path, backup = Path(undo["path"]), undo["backup"]
    if backup is not None:
        Path(backup).replace(path)
    else:
        path.unlink(missing_ok=True)

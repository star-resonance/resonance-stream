"""A local stand-in for GitHub, for a test-env build of the app.

The app's test flags (`--feed-url`, `--metadata-url`, `--data-dir`, `--status-file`; see
`.memory/roadmap/test-run-parameters.md` on `main`) let a notebook run the whole update path
against a server of its own, in a data folder of its own, and read the result from the status
file instead of asking a person. This module is the pure part of that: the files the app reads,
the server that serves them (and breaks on purpose), the command line, and the status-file waits.

Needs an exe built with `--features test-env` (a local build or a release candidate); the
released 0.6.0 / 0.6.1 ignore every flag.
"""
from __future__ import annotations

import base64
import hashlib
import http.server
import json
import threading
import time
from pathlib import Path
from typing import Callable

from . import common

NEW_EXE_MARKER = b"\n--runbook-mock-new-version--\n"


# --- what the app reads ---------------------------------------------------------------------------
def feed_json(version: str, url: str, signature: str, notes: str = "") -> str:
    """`latest.json` as src-tauri's `parse_feed` reads it (version, url, signature required)."""
    return json.dumps({
        "version": version,
        "notes": notes,
        "pub_date": "2026-10-05T00:00:00Z",
        "url": url,
        "signature": signature,
    }, ensure_ascii=False)


def metadata_json(model_url: str = "http://127.0.0.1:1/model.gguf", *, revision: int = 0, dictionary_sha256: str = "",
                  dictionary_version: str = "runbook-mock", model_sha256: str = "") -> str:
    """The published `metadata.json` (`GistMetadata`): the `app` entry is ignored by the app and left out. With a `revision`
    (and the dictionary's SHA-256) it is the signed kind `metadata.yml` publishes; without, the old gist's shape."""
    out: dict = {}
    if revision:
        out["revision"] = revision
    out["model"] = {"latest_version": "runbook-mock", "download_url": model_url, "release_notes": "", "sha256": model_sha256}
    out["dictionary"] = {"version": dictionary_version, "updated_at": "2026-10-05"}
    if dictionary_sha256:
        out["dictionary"]["sha256"] = dictionary_sha256
    return json.dumps(out)


class MetadataSigner:
    """A throwaway signing key for the mock's metadata, made and used with `tauri signer` (the tool `metadata.yml` signs with), so
    the app's real verifier is what judges it: it is told to trust this key alone (`--metadata-trust-key`). The key lives in `folder`
    for one run and signs nothing real.

    In a dry run nothing is run: a stand-in scheme is used (`FAKESIG`), which only the stand-in app in `tests/fake_app.py` reads."""

    def __init__(self, folder: str | Path, name: str = "metadata-key") -> None:
        self.folder = Path(folder)
        self.folder.mkdir(parents=True, exist_ok=True)
        self.key = self.folder / f"{name}.key"
        self._count = 0
        if common.dry_run():
            self.key_id = hashlib.sha256(name.encode()).hexdigest()[:16].upper()
            self.public_key = base64.b64encode(f"untrusted comment: dry run key\n{self.key_id}\n".encode()).decode()
            return
        code, out = common.capture(["npx", "--yes", "@tauri-apps/cli@2", "signer", "generate", "--ci", "-w", str(self.key)], timeout=300)
        pub = self.key.with_name(self.key.name + ".pub")
        if code != 0 or not pub.exists():
            raise RuntimeError("could not make a signing key with `tauri signer generate`: " + " ".join(out.split())[-300:])
        self.public_key = pub.read_text(encoding="utf-8").strip()
        self.key_id = ""

    def sign(self, body: bytes, revision: int) -> str:
        """The `.sig` text (base64, as `tauri signer sign` writes it) for `body`, its signed comment saying `version:<revision>`."""
        if common.dry_run():
            digest = hashlib.sha256(body).hexdigest()
            return base64.b64encode(f"FAKESIG\n{self.key_id}\n{digest}\n{revision}\n".encode()).decode()
        self._count += 1
        target = self.folder / f"{self.key.stem}-{self._count}.json"
        target.write_bytes(body)
        code, out = common.capture(["npx", "--yes", "@tauri-apps/cli@2", "signer", "sign", "-f", str(self.key), "-p", "", "--app-version",
                                    str(revision), str(target)], timeout=300)
        sig = target.with_name(target.name + ".sig")
        if code != 0 or not sig.exists():
            raise RuntimeError("`tauri signer sign` failed: " + " ".join(out.split())[-300:])
        return sig.read_text(encoding="utf-8").strip()


def make_new_exe(src: str | Path, dest: str | Path) -> Path:
    """A "newer" exe: the source with a marker appended after the last section. Windows ignores bytes
    past the end of an exe's last section, so it still starts -- and its hash differs from the source."""
    src, dest = Path(src), Path(dest)
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_bytes(src.read_bytes() + NEW_EXE_MARKER)
    return dest


# --- the command line -----------------------------------------------------------------------------
def flag_args(data_dir: str | Path, status_file: str | Path, *, feed_url: str | None = None,
              metadata_url: str | None = None, metadata_trust_key: str | None = None, dictionary_url: str | None = None,
              log_file: str | Path | None = None, fresh: bool = True,
              capture: bool = False, window_state: bool = False, translator: bool = False, popups: bool = False,
              extra: tuple[str, ...] = ()) -> list[str]:
    """An isolated run: its own data folder, no wizard, no sniffer, no translator, no popups, no saved
    window place. The update check is left ON -- it is what is being tested. `capture=True` leaves the sniffer on, `window_state=True` lets the app restore and save the window's size and place, `translator=True` leaves the translator on (give it `--llama-url` in `extra`), `popups=True` lets the app create its popup windows."""
    args = ["--data-dir", str(data_dir)]
    if fresh:
        args.append("--fresh")
    args += ["--assume-setup-done", *([] if capture else ["--no-capture"]), *([] if translator else ["--no-translator"]), *([] if popups else ["--no-popups"]),
             *([] if window_state else ["--no-window-state"]), "--status-file", str(status_file)]
    if log_file is not None:
        args += ["--log-file", str(log_file)]
    if feed_url is not None:
        args += ["--feed-url", feed_url]
    if metadata_url is not None:
        args += ["--metadata-url", metadata_url]
    if metadata_trust_key is not None:
        args += ["--metadata-trust-key", metadata_trust_key]
    if dictionary_url is not None:
        args += ["--dictionary-url", dictionary_url]
    return [*args, *extra]


# --- the mock server --------------------------------------------------------------------------------
class _Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args) -> None:  # keep the notebook's output clean
        pass

    def _send(self, status: int, body: bytes, content_type: str = "application/octet-stream") -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()

    def do_GET(self) -> None:  # noqa: N802 -- http.server's name
        owner: MockServer = self.server.owner  # type: ignore[attr-defined]
        owner.hits.append(f"GET {self.path}")
        if self.path == "/latest.json":
            if owner.feed_mode == "404":
                return self._reply(404, b"not found")
            body = owner.feed_text().encode("utf-8")
            return self._reply(200, body, "application/json")
        if self.path == "/metadata.json":
            return self._reply(200, owner.metadata_served().encode("utf-8"), "application/json")
        if self.path == "/metadata.json.sig":
            sig = owner.metadata_signature()
            return self._reply(404, b"not found") if sig is None else self._reply(200, sig.encode("utf-8"))
        if self.path == "/custom_dict.json":
            return self._reply(200, owner.dictionary_served().encode("utf-8"), "application/json; charset=utf-8")
        if self.path == "/update.exe" and owner.exe_mode != "404":
            return self._exe(owner)
        if self.path == "/model.gguf" and owner.model_mode != "404":
            return self._model(owner)
        self._reply(404, b"not found")

    def _reply(self, status: int, body: bytes, content_type: str = "text/plain") -> None:
        self._send(status, body, content_type)
        self.wfile.write(body)

    def _exe(self, owner: "MockServer") -> None:
        body = owner.exe_bytes
        self._send(200, body)
        if owner.exe_mode == "ok":
            self.wfile.write(body)
            return
        # cut / stall: the headers promised the whole file; a third of it goes out
        self.wfile.write(body[: max(1, len(body) // 3)])
        self.wfile.flush()
        if owner.exe_mode == "stall":
            owner.stopping.wait()  # silence until the server is stopped
        # cut: the handler returns and the connection closes early


def _model_handler(self, owner: "MockServer") -> None:
    body = owner.model_bytes
    self._send(200, body)
    if owner.model_mode == "ok":
        self.wfile.write(body)
        return
    # cut: the headers promised the whole file; half of it goes out, then the connection closes
    self.wfile.write(body[: max(1, len(body) // 2)])
    self.wfile.flush()


_Handler._model = _model_handler  # type: ignore[attr-defined]


class MockServer:
    """GitHub in miniature on 127.0.0.1: `/latest.json`, `/metadata.json`, `/update.exe`, `/model.gguf`, `/custom_dict.json`.

    `feed_mode`: ok | garbage (not JSON) | 404.   `exe_mode`: ok | cut (connection closes early) |
    stall (goes silent after a third) | 404.   `model_mode` (the file at `/model.gguf`, `model_bytes`): ok | cut | 404.
    Set them, or `version`, while it runs.
    `hits` lists every request, in order.

    With a `signer` (a `MetadataSigner`) the metadata is the signed kind: `revision`, the dictionary's SHA-256, and the signature at
    `/metadata.json.sig`. `metadata_mode`: ok | no-signature (the `.sig` is 404) | wrong-key (signed by `foreign_signer`, another key) |
    tampered (the signature is for another text). `revision` and `dictionary_override` (the text served at `/custom_dict.json`
    instead of `dictionary_text`, so it no longer matches the signed hash) can be set while it runs, and so can the model entry the
    app downloads from: `model_sha256` (its published SHA-256) and `model_url` (None: this server's `/model.gguf`). Without a signer the metadata is
    the old gist's shape and there is no `.sig`.
    """

    def __init__(self, exe_bytes: bytes = b"", signature: str = "c2ln", version: str = "9.9.9",
                 notes: str = "runbook mock release", signer: MetadataSigner | None = None,
                 foreign_signer: MetadataSigner | None = None) -> None:
        self.exe_bytes = exe_bytes
        self.signature = signature
        self.version = version
        self.notes = notes
        self.feed_mode = "ok"
        self.exe_mode = "ok"
        self.model_bytes = b""
        self.model_mode = "ok"
        self.model_url: str | None = None  # the model address the signed metadata names (None: this server's /model.gguf)
        self.model_sha256 = ""  # the model SHA-256 the signed metadata names
        self.dictionary_text = '{"term": {"ボス": "보스"}}'  # what `/custom_dict.json` serves (the gist's custom dictionary)
        self.dictionary_override: str | None = None
        self.signer = signer
        self.foreign_signer = foreign_signer
        self.revision = 1
        self.metadata_mode = "ok"
        self._signatures: dict[tuple, str] = {}
        self._sign_lock = threading.Lock()
        self.hits: list[str] = []
        self.stopping = threading.Event()
        self._httpd: http.server.ThreadingHTTPServer | None = None
        self._thread: threading.Thread | None = None

    # -- addresses
    @property
    def base_url(self) -> str:
        assert self._httpd is not None, "server not started"
        return f"http://127.0.0.1:{self._httpd.server_address[1]}"

    @property
    def feed_url(self) -> str:
        return self.base_url + "/latest.json"

    @property
    def metadata_url(self) -> str:
        return self.base_url + "/metadata.json"

    @property
    def dictionary_url(self) -> str:
        return self.base_url + "/custom_dict.json"

    @property
    def exe_url(self) -> str:
        return self.base_url + "/update.exe"

    # -- what is served
    def feed_text(self) -> str:
        if self.feed_mode == "garbage":
            return "<html>this is not a feed</html>"
        return feed_json(self.version, self.exe_url, self.signature, self.notes)

    def metadata_text(self) -> str:
        """The metadata as published (before any fault): signed kind when there is a signer."""
        if self.signer is None:
            return metadata_json(self.base_url + "/model.gguf")
        return metadata_json(self.model_url or self.base_url + "/model.gguf", revision=self.revision,
                             dictionary_sha256=hashlib.sha256(self.dictionary_text.encode("utf-8")).hexdigest(),
                             model_sha256=self.model_sha256)

    def metadata_served(self) -> str:
        """What `/metadata.json` answers: the published text, or -- `tampered` -- that text changed after it was signed."""
        text = self.metadata_text()
        return text.replace("runbook-mock", "runbook-evil") if self.metadata_mode == "tampered" else text

    def metadata_signature(self) -> str | None:
        """What `/metadata.json.sig` answers; None for 404 (no signer, or `no-signature`). Signing runs a tool, so a result is kept."""
        if self.signer is None or self.metadata_mode == "no-signature":
            return None
        signer = self.foreign_signer if self.metadata_mode == "wrong-key" else self.signer
        if signer is None:
            raise RuntimeError("metadata_mode 'wrong-key' needs a foreign_signer")
        text = self.metadata_text()
        with self._sign_lock:
            key = (id(signer), self.revision, text)
            if key not in self._signatures:
                self._signatures[key] = signer.sign(text.encode("utf-8"), self.revision)
            return self._signatures[key]

    def dictionary_served(self) -> str:
        return self.dictionary_text if self.dictionary_override is None else self.dictionary_override

    def metadata_flags(self) -> tuple[str, ...]:
        """The flags that make an app read this server's metadata and trust its key (and nothing else)."""
        flags = ("--metadata-url", self.metadata_url)
        return flags + (("--metadata-trust-key", self.signer.public_key) if self.signer is not None else ())

    # -- life cycle
    def start(self) -> "MockServer":
        self.stopping.clear()
        httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
        httpd.daemon_threads = True
        httpd.owner = self  # type: ignore[attr-defined]
        self._httpd = httpd
        self._thread = threading.Thread(target=httpd.serve_forever, daemon=True)
        self._thread.start()
        self.metadata_signature()  # signs now, not while the app waits for its answer
        return self

    def stop(self) -> None:
        self.stopping.set()
        if self._httpd is not None:
            self._httpd.shutdown()
            self._httpd.server_close()
        if self._thread is not None:
            self._thread.join(timeout=5)

    def __enter__(self) -> "MockServer":
        return self.start()

    def __exit__(self, *exc) -> None:
        self.stop()


# --- the status file --------------------------------------------------------------------------------
def read_status(path: str | Path) -> dict | None:
    """The app's `--status-file` as a dict; None while it is missing or half written."""
    try:
        data = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    return data if isinstance(data, dict) else None


def wait_status(path: str | Path, predicate: Callable[[dict], object], timeout: float, interval: float = 0.5,
                *, clock: Callable[[], float] = time.monotonic, sleep: Callable[[float], None] = time.sleep,
                reader: Callable[[str | Path], dict | None] = read_status) -> dict | None:
    """The first status that satisfies `predicate`, or None after `timeout` seconds."""
    start = clock()
    while True:
        status = reader(path)
        if status is not None and predicate(status):
            return status
        if clock() - start >= timeout:
            return None
        sleep(interval)


def feed_signature(sig_text: str) -> str:
    """The `signature` of `latest.json`: the `.sig` file's text with its line breaks removed (what
    release-lib.sh puts in the feed)."""
    return sig_text.replace("\r", "").replace("\n", "").strip()


def update_kind(state: str | None) -> tuple[str, str]:
    """`"available:9.9.9"` -> ("available", "9.9.9"); the status file's `update` split in two."""
    kind, _, detail = (state or "none").partition(":")
    return kind, detail


# --- small helpers ----------------------------------------------------------------------------------
def fingerprint(path: str | Path) -> tuple[int, int, str] | None:
    """(size, mtime in ns, SHA-256) of a file, None if missing -- to show it was not touched."""
    path = Path(path)
    try:
        stat = path.stat()
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError:
        return None
    return stat.st_size, stat.st_mtime_ns, digest


def start_app(exe: str | Path, args: list[str]) -> int:
    """Start a copy, in its own folder, and return its pid. Unlike `updater.launch` it also starts in a
    dry run: the dry run of the mock notebook drives a stand-in app on purpose. This process is elevated
    on Windows, so the child is too (no UAC prompt)."""
    import subprocess

    exe = Path(exe)
    if not exe.exists():
        raise ValueError(f"{exe} does not exist")
    return subprocess.Popen([str(exe), *args], cwd=str(exe.parent), close_fds=True).pid


def run_print_env(exe: str | Path, data_dir: str | Path, status_file: str | Path,
                  timeout: float = 30) -> tuple[int, str]:
    """`exe --data-dir D --print-env --status-file S`: a test-env build writes the file and exits at once.
    An exe that ignores the flags starts its window instead -- it is killed at the timeout (exit code 124)."""
    import subprocess

    command = [str(exe), "--data-dir", str(data_dir), "--print-env", "--status-file", str(status_file)]
    try:
        done = subprocess.run(command, capture_output=True, text=True, timeout=timeout,
                              encoding="utf-8", errors="replace")
    except subprocess.TimeoutExpired:
        return 124, f"timed out after {timeout}s: the exe did not exit, so it does not understand --print-env"
    except OSError as e:
        return 127, f"could not start {exe}: {e}"
    return done.returncode, (done.stdout or "") + (done.stderr or "")


def kill_command(pid: int) -> list[str]:
    return ["taskkill", "/PID", str(pid), "/T", "/F"]


def stop_app(pid: int | None) -> None:
    """Close a copy the notebook started (by the pid its status file names). Never raises."""
    if not pid:
        return
    import subprocess
    import sys

    try:
        if sys.platform == "win32":
            subprocess.run(kill_command(pid), capture_output=True, timeout=30, check=False)
        else:  # the dry run's stand-in app
            import os
            import signal

            os.kill(pid, signal.SIGKILL)
    except (OSError, subprocess.SubprocessError):
        pass


def posix_processes() -> list[dict]:
    """DRY RUN SUPPORT (not Windows): the stand-in apps running here, as [{"Path", "Id"}] -- found by the
    `FAKE_APP_EXE` in their environment (tests/fake_app.py's shell-script "exe" sets it). A finished
    process that has not been reaped has no environment and is not listed."""
    import os

    found: list[dict] = []
    if not os.path.isdir("/proc"):
        return found
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            environment = Path(f"/proc/{entry}/environ").read_bytes().split(b"\0")
        except OSError:
            continue
        for item in environment:
            if item.startswith(b"FAKE_APP_EXE="):
                found.append({"Path": item.split(b"=", 1)[1].decode("utf-8", "replace"), "Id": int(entry)})
                break
    return found


def stop_copies(folder: str | Path) -> list[int]:
    """Close every app whose exe is in `folder` or below and wait until it is gone: `updater.stop_copies`
    on Windows; in a dry run on another OS the same, for the stand-in apps."""
    import sys

    from runbook import updater

    if sys.platform == "win32":
        return updater.stop_copies(folder)
    import contextlib
    import os
    import signal

    def kill(pid: int) -> None:
        with contextlib.suppress(OSError):
            os.kill(pid, signal.SIGKILL)

    return updater.stop_copies(folder, list_processes=posix_processes, kill=kill)


def log_tail(path: str | Path, lines: int = 15) -> str:
    """The last lines of the app's `--log-file`, for the report when a check fails."""
    try:
        text = Path(path).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""
    return "\n".join(text.splitlines()[-lines:])

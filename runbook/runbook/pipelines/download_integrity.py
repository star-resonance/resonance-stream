"""Pipeline `download-integrity`: the model download refuses what it must refuse, and never breaks the model that is installed.

A mock server on 127.0.0.1 (`mockfeed.MockServer`, `/model.gguf`) stands in for GitHub; the app is started with `--metadata-url` pointing
at it and `--metadata-trust-key` naming this run's throwaway key, which is what lets a test run download over local http. The app takes
the model's url and SHA-256 from that signed metadata (N-5), never from the caller, so each case changes what the mock publishes (its
model entry, re-signed) and plays the setup wizard over the bridge: `download-model` with no arguments, and the end is the
`download-result` event. After every case the installed model file is looked at on disk, and so is the folder for a partial file. Rows:

  DI-good     a good download installs the model (its SHA-256 on disk is the published one), shows progress up to 100 %, leaves no partial file
  DI-skip     the same model again is not downloaded again (the server is not asked), progress says it was skipped
  DI-hash     bytes that do not match the published SHA-256 are refused; the installed model and the folder are as they were
  DI-cut      a download the server cuts off half way is refused, likewise
  DI-404      a server that answers 404 is a failure, likewise
  DI-nohash   a model published with no SHA-256 is refused before anything is downloaded
  DI-https    a published address that is neither https nor this machine's is refused before anything is downloaded

(The stall case -- a server that goes silent -- takes the app's 30 s stall limit and is covered for the app update by `updater-mock`.)
"""
from __future__ import annotations

import hashlib
from pathlib import Path

from runbook import bridge, common, mockfeed, updater
from runbook.common import Recorder


def blob(seed: str, size: int = 3 * 1024 * 1024) -> bytes:
    """Deterministic pseudo-random bytes: big enough for progress to move in more than one step."""
    out, counter = bytearray(), 0
    while len(out) < size:
        out += hashlib.sha256(f"{seed}:{counter}".encode()).digest()
        counter += 1
    return bytes(out[:size])


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class DownloadIntegrity:
    def __init__(self, rec: Recorder, exe: str | Path, runs: Path | None = None) -> None:
        self.rec = rec
        self.exe = Path(exe)
        self.runs = Path(runs or common.RUNS) / "download-integrity"
        self._serves: list[bridge.Serve] = []
        self._servers: list[mockfeed.MockServer] = []
        self._signer: mockfeed.MetadataSigner | None = None

    def run(self, key_path: str | None = None, password: str | None = None) -> None:
        """(`key_path` and `password` are for the pipelines that sign; this one ignores them.)"""
        with updater.step(self.rec, "DI", self.runs, strays=self.runs, stop=self._stop):
            self.round()

    def _stop(self, folder: str | Path) -> list[int]:
        for server in self._servers:
            server.stop()
        self._servers.clear()
        for serve in self._serves:
            serve.stop()
        self._serves.clear()
        return mockfeed.stop_copies(folder)

    @staticmethod
    def model_dir(exe: Path) -> Path:
        return exe.parent / "data" / "data" / "models" / "translation-model"

    def state(self, exe: Path) -> str:
        """The installed model's SHA-256 (or 'none') and the files next to it that are not the model: what must not change."""
        folder = self.model_dir(exe)
        model = folder / "model.gguf"
        digest = sha(model.read_bytes()) if model.exists() else "none"
        extra = sorted(p.name for p in folder.glob("*") if p.name != "model.gguf") if folder.exists() else []
        return f"model {digest[:12]}, other files {extra}"

    def signer(self) -> mockfeed.MetadataSigner:
        """This run's throwaway metadata key (made once)."""
        if self._signer is None:
            self._signer = mockfeed.MetadataSigner(self.runs / "signing", "good")
        return self._signer

    @staticmethod
    def model_hits(server: mockfeed.MockServer) -> int:
        """How often the model file was asked for (the metadata is fetched at every download, so it is not counted)."""
        return sum(1 for hit in server.hits if hit == "GET /model.gguf")

    @staticmethod
    def download(events: bridge.Serve, server: mockfeed.MockServer, url: str | None, digest: str) -> dict:
        """Publish `url` (None: the server's own `/model.gguf`) and `digest` as the model, then `download-model`; what the app
        said when it ended: the `download-result` payload."""
        server.model_url, server.model_sha256 = url, digest
        ack = events.send("download-model", {}, timeout=15)
        return events.expect("download-result", {"payload.id": ack["id"]}, timeout=90)["payload"]

    def round(self) -> None:
        good, other = blob("good"), blob("other")
        server = mockfeed.MockServer(signer=self.signer()).start()
        self._servers.append(server)
        server.model_bytes = good
        exe = updater.fresh_copy(self.exe, self.runs, "run")
        events = bridge.Serve(exe.parent / "events.jsonl")
        self._serves.append(events)
        args = mockfeed.flag_args(exe.parent / "data", exe.parent / "status.json", log_file=exe.parent / "app.log",
                                  metadata_url=server.metadata_url, metadata_trust_key=self.signer().public_key, extra=("--no-update-check", "--bridge-url", events.url))
        mockfeed.start_app(exe, args)
        if not bridge.wait_started(events, exe.parent / "app.log"):
            self.rec.auto("DI-start", "the app started and connected to the bridge", False, "no app-started event in 120 s")
            return
        # DI-good
        result = self.download(events, server, None, sha(good))
        installed = (self.model_dir(exe) / "model.gguf")
        progress = [m["message"]["payload"]["percent"] for m in events.events() if m["topic"] == "rs/app/event/download-progress"]
        ok = (result["ok"] and installed.exists() and sha(installed.read_bytes()) == sha(good) and 100 in progress
              and self.state(exe) == f"model {sha(good)[:12]}, other files []")
        self.rec.auto("DI-good", "a good download installs the model, shows progress to 100 %, leaves no partial file", ok,
                      f"result {result}; {self.state(exe)}; progress steps {len(set(progress))}, last {progress[-1] if progress else None}")
        reference = self.state(exe)

        # DI-skip
        before = self.model_hits(server)
        result = self.download(events, server, None, sha(good))
        skipped = [m for m in events.events() if m["topic"] == "rs/app/event/download-progress"
                   and "Skipped" in str(m["message"]["payload"].get("current_file"))]
        self.rec.auto("DI-skip", "the model it already has is not downloaded again", result["ok"] and self.model_hits(server) == before and bool(skipped),
                      f"result {result}; model asked {self.model_hits(server) - before} more time(s); 'Skipped' progress seen: {bool(skipped)}")

        def refused(check: str, title: str, url: str | None, digest: str, *, asks_server: bool, needle: str = "") -> None:
            hits = self.model_hits(server)
            result = self.download(events, server, url, digest)
            unchanged = self.state(exe) == reference
            asked = self.model_hits(server) - hits
            ok = (not result["ok"] and bool(result.get("error")) and needle.lower() in str(result["error"]).lower()
                  and unchanged and (asked > 0 if asks_server else asked == 0))
            self.rec.auto(check, title, ok, f"ok={result['ok']}, error={result.get('error')!r}; {self.state(exe)} "
                                            f"(was: {reference}); model asked {asked} time(s)")

        server.model_bytes = other
        refused("DI-hash", "bytes that do not match the published SHA-256 are refused and change nothing", None, sha(good)[::-1], asks_server=True)
        server.model_mode = "cut"
        refused("DI-cut", "a download cut off half way is refused and changes nothing", None, sha(other), asks_server=True)
        server.model_mode = "404"
        refused("DI-404", "a 404 is a failure and changes nothing", None, sha(other), asks_server=True)
        server.model_mode = "ok"
        refused("DI-nohash", "a model published with no SHA-256 is refused before anything is downloaded", None, "", asks_server=False,
                needle="sha-256")
        refused("DI-https", "a published address that is neither https nor this machine's is refused before anything is downloaded",
                "http://203.0.113.9/model.gguf", sha(other), asks_server=False, needle="non-HTTPS")
        try:
            events.send("quit", timeout=15)
        except RuntimeError:
            pass

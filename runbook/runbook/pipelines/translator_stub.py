"""Pipeline `translator-stub`: the whole translate path with no model and no GPU.

The app is started with `--llama-url` at a stand-in llama-server (`runbook/llama_stub.py`: the same endpoints and replies, nothing behind them),
the translator is started over the bridge (`start-translator`), Japanese chat is replayed, and what the app does is read from its events and
from what the stand-in received. Rows:

  TS-ready      the translator goes Starting -> Loading Model -> Active, asking /health until it says it is up (the app starts the
                translator itself at start-up when `use_translation` is on; `start-translator` is then a no-op)
  TS-translate  a Japanese line gets its translation (`translation-event`, and the row in the chat log); a line that is not Japanese is not sent
  TS-prompt     what the server was sent is one user turn holding the line, ends by opening the model's turn, and has the sampling settings
                (stream off, stop tokens, an output limit). Also reported, not judged: how many literal `<bos>` the prompt text holds (K8)
  TS-restart    three lines in a row that the server cannot answer (HTTP 500) restart the translator: Restarting, then Active again
  TS-catchup    the lines that failed are translated after the restart, not lost
  TS-log-quiet  the system log has no "Polling .../health" line (the health wait is quiet there)
  TS-reload     after the app is closed and started again on the same folder, each Japanese line is served once, with its translation

The custom dictionary (`sync-dictionary`, `save-local-dictionary`: the settings view's sync and the dictionary editor's save). A term is
shielded: the server is sent a placeholder `[P<n>]` where the term stood, and the Korean comes back in the shown translation. One run
with a stand-in server for the dictionary (`--dictionary-url`; `mockfeed.MockServer`), a line about ボス said at each step:

  TS-dict-before  before any dictionary the server is sent the line as it is
  TS-dict-sync    after `sync-dictionary` the next line goes out with a placeholder and comes back with the dictionary's Korean (보스); the
                  server was asked for the dictionary once and `custom_dict.json` holds what it served
  TS-dict-local   after `save-local-dictionary` with another Korean term the next line uses it, with no restart of the translator
  TS-dict-bad     a text that does not parse is refused, the file is as it was, and the dictionary in use is kept

And two runs with the update check on (the gist announces a newer dictionary), `auto_sync_latest_dict` off and on:

  TS-dict-auto-off  the update check ran and nothing was synced (no request for the dictionary): the default syncs nothing at start
  TS-dict-auto-on   with `auto_sync_latest_dict` on the dictionary is fetched at start-up

The dictionary and the model's metadata are signed (`metadata.yml` publishes them; the app trusts only its built-in keys). The stand-in
server signs with a throwaway key (`mockfeed.MetadataSigner`, made with `tauri signer`) and the app is told to trust that key alone
(`--metadata-trust-key`), so the app's real verifier judges every case. The update check is on, `auto_sync_latest_dict` on, the feed
announces a newer app:

  TS-meta-accepted  a good publication is accepted: the dictionary is fetched and the revision is remembered in `metadata.json`
  TS-meta-unsigned  no `.sig` is published: nothing is synced, no `custom_dict.json`, an Error line in the system log, and the app update is
                    still announced
  TS-meta-wrong-key the signature is from another key: refused the same way
  TS-meta-tampered  the metadata changed after it was signed: refused the same way
  TS-meta-replay    the app has accepted revision 5 and is shown a correctly signed revision 3: refused as a rollback
  TS-meta-hash      the metadata is good but the dictionary served is not the one it names: `sync-dictionary` fails, nothing is saved, the
                    dictionary in use is kept; served right again, the same sync works

Three more runs of their own:

  TS-later-wait / TS-later-catchup  translation off (`use_translation` false), five Japanese lines are said, then `start-translator` with
                `translation_catch_up_limit` = 3: nothing is asked of the server while the translator is off, and then Catching Up
                translates the newest three, oldest first (the two older ones are passed over)
  TS-live-first  the server is slow and still loading when four Japanese lines arrive; during the catch-up a fifth, live line is said:
                it is translated before the last catch-up item, not after all of them
  TS-hang       the server takes a request and never answers: the line is given up on after the app's 30 s limit, no restart, and the next
                line is translated
"""
from __future__ import annotations

import json
import time
from pathlib import Path

from runbook import bridge, common, mockfeed, updater
from runbook.common import Recorder
from runbook.llama_stub import LlamaStub, source_of

GREETING, THANKS, ENGLISH = "こんにちは", "ありがとうございました", "Hello from an English speaker"
CANNED = {GREETING: "안녕하세요", THANKS: "감사합니다"}
LOADING_S = 3.0
LATER = ["おはよう", "こんばんは", "お疲れさま", "よろしくお願いします", "がんばって"]
LATER_LIMIT = 3
SLOW_LOADING_S = 12.0  # the lines of the live-first run arrive while the server loads
SLOW_DELAY_S = 2.0  # then each reply takes this long, so the catch-up is still running when the live line arrives
REQUEST_TIMEOUT_S = 30.0  # the app's own limit for a reply (resonance_llama::REQUEST_TIMEOUT)
HANG_MARGIN_S = 30.0
BOSS = ["ボスはどこですか", "ボスを倒しました", "ボスが強いです", "ボスは楽勝でした"]  # one line per step of the dictionary run
LOCAL_DICTIONARY = '{"term": {"ボス": "BOSS"}}'
AUTO_GRACE_S = 10.0  # after the update check has run, how long a start-up sync that should not happen is waited for
WAIT_S = 60.0  # how long a run waits for a state or a translation it expects before it says the row failed


def line(text: str, seq: int, ts: int, delay_ms: int = 100) -> str:
    return json.dumps({"delay_ms": delay_ms, "channel": "GUILD", "nickname": "Tester", "uid": 4242, "level": 60,
                       "timestamp": ts, "sequence_id": seq, "text": text}, ensure_ascii=False)


class TranslatorStub:
    def __init__(self, rec: Recorder, exe: str | Path, runs: Path | None = None) -> None:
        self.rec = rec
        self.exe = Path(exe)
        self.runs = Path(runs or common.RUNS) / "translator-stub"
        self._serves: list[bridge.Serve] = []
        self._stubs: list[LlamaStub] = []
        self._servers: list[mockfeed.MockServer] = []
        self._signers: dict[str, mockfeed.MetadataSigner] = {}

    def run(self, key_path: str | None = None, password: str | None = None) -> None:
        """(`key_path` and `password` are for the pipelines that sign; this one ignores them.)"""
        with updater.step(self.rec, "TS", self.runs, strays=self.runs, stop=self._stop):
            self.round()

    def _stop(self, folder: str | Path) -> list[int]:
        for stub in self._stubs:
            stub.stop()
        self._stubs.clear()
        for server in self._servers:
            server.stop()
        self._servers.clear()
        for serve in self._serves:
            serve.stop()
        self._serves.clear()
        return mockfeed.stop_copies(folder)

    def signer(self, name: str = "good") -> mockfeed.MetadataSigner:
        """A throwaway metadata key of this run (made once per name)."""
        if name not in self._signers:
            self._signers[name] = mockfeed.MetadataSigner(self.runs / "signing" / name, name)
        return self._signers[name]

    def signed_server(self, foreign: bool = False) -> mockfeed.MockServer:
        """A stand-in GitHub whose metadata is signed by this run's key (`foreign`: it also has a second key, for a wrong-key case)."""
        server = mockfeed.MockServer(signer=self.signer(), foreign_signer=self.signer("foreign") if foreign else None).start()
        self._servers.append(server)
        return server

    def replay(self, events: bridge.Serve, path: Path, lines: list[str]) -> None:
        path.write_text("\n".join(lines) + "\n", encoding="utf-8")
        events.send("replay-chat", {"path": str(path)})

    def translated(self, events: bridge.Serve) -> dict[int, str]:
        return {m["message"]["payload"]["pid"]: m["message"]["payload"]["translated"]
                for m in events.events() if m["topic"] == "rs/app/event/translation-event"}

    @staticmethod
    def done(events: bridge.Serve) -> dict[str, str]:
        """The lines of the chat log that have a translation now, {text: translation}."""
        return {m["message"]: m["translated"] for m in events.send("get-chat-history")["data"] if m.get("translated")}

    def states(self, events: bridge.Serve) -> list[str]:
        return [m["message"]["payload"]["state"] for m in events.events() if m["topic"] == "rs/app/event/translator-state"]

    def round(self) -> None:
        stub = LlamaStub(loading_seconds=LOADING_S).start()
        stub.translations = dict(CANNED)
        self._stubs.append(stub)
        exe = updater.fresh_copy(self.exe, self.runs, "run")
        events = self.launch("first", exe, stub, {"init_done": True, "use_translation": True})
        if events is None:
            self.rec.auto("TS-start", "the app started and connected to the bridge", False, "no app-started event in 120 s")
            return

        began = time.monotonic()
        events.send("start-translator")
        try:
            events.expect_sequence([("translator-state", {"payload.state": "Starting"}),
                                    ("translator-state", {"payload.state": "Loading Model"}),
                                    ("translator-state", {"payload.state": "Active"})], timeout=90)
            took = time.monotonic() - began
            ok = stub.health_503 >= 1 and "GET /health" in stub.hits
            detail = (f"Active; the app asked /health {stub.hits.count('GET /health')} times and was told 503 {stub.health_503} times "
                      f"before the stand-in was up ({LOADING_S:.0f} s after the first ask); {took:.1f} s since the command; states {self.states(events)}")
        except RuntimeError as e:
            ok, detail = False, f"{e}; states {self.states(events)}"
        self.rec.auto("TS-ready", "the translator goes Starting -> Loading Model -> Active, and waits for the server's /health", ok, detail)
        if not ok:
            return
        polling = [m["message"]["payload"]["message"] for m in events.events()
                   if m["topic"] == "rs/app/event/system-event" and "polling" in str(m["message"]["payload"].get("message", "")).lower()]
        self.rec.auto("TS-log-quiet", "the health wait leaves no 'Polling .../health' lines in the system log", not polling,
                      f"{len(polling)} such line(s)" + (f", e.g. {polling[0]!r}" if polling else ""))

        ts = int(time.time())
        self.replay(events, exe.parent / "first.jsonl", [line(GREETING, 1, ts, 0), line(THANKS, 2, ts), line(ENGLISH, 3, ts)])
        try:
            events.expect("system-event", {"payload.message": "Replay finished"}, timeout=30)
            time.sleep(3)
        except RuntimeError:
            self.rec.auto("TS-translate", "a Japanese line gets its translation", False, "the replay never finished")
            return
        packets = {p["message"]["payload"]["message"]: p["message"]["payload"]["pid"] for p in events.events()
                   if p["topic"] == "rs/app/event/packet-event"}
        got = self.translated(events)
        history = {m["message"]: m.get("translated") for m in events.send("get-chat-history")["data"]}
        want = {packets.get(GREETING): CANNED[GREETING], packets.get(THANKS): CANNED[THANKS]}
        asked = [source_of(r["prompt"]) for r in stub.requests]
        ok = (got == want and history.get(GREETING) == CANNED[GREETING] and history.get(THANKS) == CANNED[THANKS]
              and not history.get(ENGLISH) and sorted(asked) == sorted([GREETING, THANKS]))
        self.rec.auto("TS-translate", "a Japanese line gets its translation, a line that is not Japanese is not sent", ok,
                      f"translation events {got}; chat log {history}; the server was asked {asked}")

        prompt = str(stub.requests[0].get("prompt", "")) if stub.requests else ""
        request = stub.requests[0] if stub.requests else {}
        ok = (prompt.startswith("<bos><start_of_turn>user\n") and prompt.endswith("<end_of_turn>\n<start_of_turn>model\n")
              and prompt.count("<start_of_turn>user") == 1 and source_of(prompt) in (GREETING, THANKS)
              and request.get("stream") is False and "<end_of_turn>" in (request.get("stop") or [])
              and 1 <= int(request.get("n_predict") or 0) <= 512)
        self.rec.auto("TS-prompt", "the prompt is one user turn with the line and ends by opening the model's turn; sampling settings are set", ok,
                      f"literal <bos> in the prompt text: {prompt.count('<bos>')} (K8: llama.cpp may add its own); "
                      f"stream={request.get('stream')} stop={request.get('stop')} n_predict={request.get('n_predict')}; prompt starts {prompt[:40]!r}")

        stub.completion_mode = "error"
        before = len(stub.requests)
        lost = ["さようなら", "おやすみなさい", "またあした"]
        self.replay(events, exe.parent / "second.jsonl", [line(t, 10 + i, ts + 5, 100) for i, t in enumerate(lost)])
        try:
            events.expect("translator-state", {"payload.state": "Restarting"}, timeout=60)
            stub.completion_mode = "ok"
            events.expect_sequence([("translator-state", {"payload.state": "Restarting"}),
                                    ("translator-state", {"payload.state": "Active"})], timeout=90)
            ok, detail = True, f"{len(stub.requests) - before} requests while the server failed; states {self.states(events)[-6:]}"
        except RuntimeError as e:
            stub.completion_mode = "ok"
            ok, detail = False, f"{e}; states {self.states(events)}"
        self.rec.auto("TS-restart", "three failed lines in a row restart the translator (Restarting, then Active)", ok, detail)

        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            korean = {m["message"]: m.get("translated") for m in events.send("get-chat-history")["data"]}
            if all(korean.get(t) for t in lost):
                break
            time.sleep(1)
        self.rec.auto("TS-catchup", "the lines that failed are translated after the restart", all(korean.get(t) for t in lost),
                      f"{ {t: korean.get(t) for t in lost} }")
        time.sleep(2)  # the archive writer is a thread of its own
        if not self.leave(events):
            self.rec.auto("TS-reload", "a translated line is served once, translated, after a restart", False, "the first run did not end when told to")
        else:
            self.reload(exe, stub, [GREETING, THANKS, *lost])
        self.later_round()
        self.slow_round()
        self.dictionary_round()
        self.auto_sync_round(False)
        self.auto_sync_round(True)
        self.refused_round("TS-meta-unsigned", "no signature published", mode="no-signature", want="no signature is published")
        self.refused_round("TS-meta-wrong-key", "a signature from another key", mode="wrong-key", want="signature was refused")
        self.refused_round("TS-meta-tampered", "metadata changed after it was signed", mode="tampered", want="signature was refused")
        self.refused_round("TS-meta-replay", "an older revision than one already accepted", revision=3, accepted=5, want="older than the revision 5")
        self.hash_round()

    def launch(self, label: str, exe: Path, stub: LlamaStub, config: dict | None = None, extra: tuple[str, ...] = ()) -> bridge.Serve | None:
        """Starts the app on its own folder, at `stub`; writes `config` first when given. None when it never says app-started."""
        data = exe.parent / "data"
        if config is not None:
            (data / "config").mkdir(parents=True, exist_ok=True)
            (data / "config" / "config.json").write_text(json.dumps(config), encoding="utf-8")
        events = bridge.Serve(exe.parent / f"{label}-events.jsonl")
        self._serves.append(events)
        args = mockfeed.flag_args(data, exe.parent / f"{label}-status.json", log_file=exe.parent / f"{label}.log", fresh=False, translator=True,
                                  extra=("--no-update-check", "--llama-url", stub.url, "--bridge-url", events.url, *extra))
        mockfeed.start_app(exe, args)
        return events if bridge.wait_started(events, exe.parent / f"{label}.log", label=f"the app ({label})") else None

    @staticmethod
    def leave(events: bridge.Serve) -> bool:
        try:
            events.send("quit", timeout=15)
            events.expect("rs/app/status", "offline", timeout=60)
        except RuntimeError:
            return False
        time.sleep(1)
        return True

    def reload(self, exe: Path, stub: LlamaStub, japanese: list[str]) -> None:
        """The same folder again: every Japanese line comes back once, with its translation (a line saved untranslated and again
        translated is one line, the newer)."""
        events = self.launch("reload", exe, stub)
        if events is None:
            self.rec.auto("TS-reload", "a translated line is served once, translated, after a restart", False, "no app-started event in 120 s")
            return
        history = events.send("get-chat-history")["data"]
        rows = {t: [m for m in history if m["message"] == t] for t in japanese}
        ok = all(len(r) == 1 and r[0].get("translated") for r in rows.values())
        self.rec.auto("TS-reload", "after a restart each Japanese line is served once, with its translation", ok,
                      "; ".join(f"{t}: {len(r)} row(s), translated {r[0].get('translated')!r}" if r else f"{t}: missing" for t, r in rows.items()))
        self.leave(events)

    def later_round(self) -> None:
        """Translation off while Japanese chat is said; turned on later it catches up on the newest `LATER_LIMIT` only."""
        stub = LlamaStub().start()
        self._stubs.append(stub)
        exe = updater.fresh_copy(self.exe, self.runs, "later")
        events = self.launch("later", exe, stub, {"init_done": True, "use_translation": False, "translation_catch_up_limit": LATER_LIMIT})
        if events is None:
            self.rec.auto("TS-later-wait", "nothing is translated while translation is off", False, "no app-started event in 120 s")
            return
        ts = int(time.time())
        self.replay(events, exe.parent / "later.jsonl", [line(t, 20 + i, ts, 0 if i == 0 else 100) for i, t in enumerate(LATER)])
        try:
            events.expect("system-event", {"payload.message": "Replay finished"}, timeout=30)
        except RuntimeError:
            self.rec.auto("TS-later-wait", "nothing is translated while translation is off", False, "the replay never finished")
            return
        time.sleep(2)
        states = self.states(events)
        waited = not stub.requests and not self.done(events) and "Active" not in states
        self.rec.auto("TS-later-wait", "with translation off, nothing is asked of the server and nothing is translated", waited,
                      f"{len(stub.requests)} request(s), {len(self.done(events))} translation(s), translator states {states}")
        events.send("start-translator")
        want = LATER[-LATER_LIMIT:]
        deadline = time.monotonic() + WAIT_S
        while time.monotonic() < deadline and len(self.done(events)) < LATER_LIMIT:
            time.sleep(0.5)
        time.sleep(2)  # a fourth would have come right after the third
        asked = [source_of(r["prompt"]) for r in stub.requests]
        history = self.done(events)
        caught = "Catching Up" in self.states(events)
        ok = caught and asked == want and not any(history.get(t) for t in LATER[:-LATER_LIMIT])
        self.rec.auto("TS-later-catchup", f"turned on later, Catching Up translates the newest {LATER_LIMIT} lines, oldest first, and passes over the older",
                      ok, f"Catching Up seen: {caught}; the server was asked {asked} (wanted {want}); older lines translated: "
                          f"{[t for t in LATER[:-LATER_LIMIT] if history.get(t)]}")
        self.leave(events)

    def say(self, events: bridge.Serve, exe: Path, stub: LlamaStub, text: str, seq: int) -> tuple[str, str]:
        """One Japanese line in; what the server was sent for it and the translation shown for it (both empty when it never came)."""
        asked = len(stub.requests)
        self.replay(events, exe.parent / f"say-{seq}.jsonl", [line(text, seq, int(time.time()), 0)])
        deadline = time.monotonic() + WAIT_S
        while time.monotonic() < deadline and len(stub.requests) <= asked:
            time.sleep(0.3)
        sent = source_of(stub.requests[asked]["prompt"]) if len(stub.requests) > asked else ""
        while time.monotonic() < deadline and text not in self.done(events):
            time.sleep(0.3)
        return sent, self.done(events).get(text, "")

    def dictionary_round(self) -> None:
        """`sync-dictionary` and `save-local-dictionary`, seen from what the translator sends and shows."""
        stub = LlamaStub().start()
        self._stubs.append(stub)
        server = self.signed_server()
        exe = updater.fresh_copy(self.exe, self.runs, "dict")
        events = self.launch("dict", exe, stub, {"init_done": True, "use_translation": True},
                             extra=(*server.metadata_flags(), "--dictionary-url", server.dictionary_url))
        rows = ("TS-dict-before", "TS-dict-sync", "TS-dict-local", "TS-dict-bad")
        if events is None:
            self.rec.auto(rows[0], "the dictionary run started", False, "no app-started event in 120 s")
            return
        try:
            events.expect("translator-state", {"payload.state": "Active"}, timeout=90)
        except RuntimeError as e:
            self.rec.auto(rows[0], "the translator is up before the dictionary is tried", False, str(e))
            self.leave(events)
            return
        on_disk = exe.parent / "data" / "data" / "custom_dict.json"
        sent, shown = self.say(events, exe, stub, BOSS[0], 101)
        self.rec.auto("TS-dict-before", "before any dictionary the server is sent the line as it is", sent == BOSS[0] and "ボス" in shown,
                      f"sent {sent!r}, shown {shown!r}")
        try:
            events.send("sync-dictionary", {}, timeout=30)
            sent, shown = self.say(events, exe, stub, BOSS[1], 102)
            saved = on_disk.read_text(encoding="utf-8") if on_disk.exists() else None
            asked = server.hits.count("GET /custom_dict.json")
            ok = "[P" in sent and "ボス" not in sent and "보스" in shown and asked == 1 and saved == server.dictionary_text
            detail = (f"sent {sent!r}, shown {shown!r}; the server was asked for the dictionary {asked} time(s); "
                      f"custom_dict.json {'holds what was served' if saved == server.dictionary_text else f'holds {saved!r}'}")
        except RuntimeError as e:
            ok, detail = False, str(e)
        self.rec.auto("TS-dict-sync", "after sync-dictionary the next line goes out with a placeholder and comes back with the Korean term", ok, detail)
        try:
            events.send("save-local-dictionary", {"content": LOCAL_DICTIONARY}, timeout=15)
            sent, shown = self.say(events, exe, stub, BOSS[2], 103)
            restarted = [st for st in self.states(events) if st in ("Restarting", "Starting")]
            ok = "[P" in sent and "ボス" not in sent and "BOSS" in shown and "보스" not in shown and len(restarted) <= 1
            detail = f"sent {sent!r}, shown {shown!r}; translator states {self.states(events)}"
        except RuntimeError as e:
            ok, detail = False, str(e)
        self.rec.auto("TS-dict-local", "after save-local-dictionary the next line uses the edited term, with no restart", ok, detail)
        try:
            events.send("save-local-dictionary", {"content": '{"term": {'}, timeout=15)
            ok, detail = False, "the app took a text that does not parse"
        except RuntimeError as e:
            ok, detail = "refused" in str(e), str(e)
        kept = on_disk.read_text(encoding="utf-8") if on_disk.exists() else None
        sent, shown = self.say(events, exe, stub, BOSS[3], 104)
        self.rec.auto("TS-dict-bad", "a text that does not parse is refused, the file is as it was and the dictionary in use is kept",
                      ok and kept == LOCAL_DICTIONARY and "BOSS" in shown,
                      f"{detail}; the file holds {kept!r}; the next line is shown {shown!r}")
        self.leave(events)

    def auto_sync_round(self, auto: bool) -> None:
        """The update check on, the gist announcing a dictionary this folder has not got: `auto_sync_latest_dict` decides whether it is fetched."""
        check = "TS-dict-auto-on" if auto else "TS-dict-auto-off"
        title = ("with auto_sync_latest_dict on the dictionary is fetched at start-up" if auto
                 else "with auto_sync_latest_dict off (the default) nothing is synced at start-up")
        server = self.signed_server()
        server.revision = 4
        label = "auto-on" if auto else "auto-off"
        exe = updater.fresh_copy(self.exe, self.runs, label)
        data = exe.parent / "data"
        (data / "config").mkdir(parents=True, exist_ok=True)
        (data / "config" / "config.json").write_text(json.dumps({"init_done": True, "auto_sync_latest_dict": auto}), encoding="utf-8")
        events = bridge.Serve(exe.parent / f"{label}-events.jsonl")
        self._serves.append(events)
        args = mockfeed.flag_args(data, exe.parent / f"{label}-status.json", log_file=exe.parent / f"{label}.log", fresh=False,
                                  feed_url=server.feed_url, metadata_url=server.metadata_url, metadata_trust_key=server.signer.public_key,
                                  dictionary_url=server.dictionary_url, extra=("--bridge-url", events.url))
        mockfeed.start_app(exe, args)
        if not bridge.wait_started(events, exe.parent / f"{label}.log", label=f"the app ({label})"):
            self.rec.auto(check, title, False, "no app-started event in 120 s")
            return
        deadline = time.monotonic() + WAIT_S
        while time.monotonic() < deadline and "GET /metadata.json" not in server.hits:
            time.sleep(0.5)
        checked = "GET /metadata.json" in server.hits
        if auto:
            while time.monotonic() < deadline and "GET /custom_dict.json" not in server.hits:
                time.sleep(0.5)
        else:
            time.sleep(AUTO_GRACE_S)
        asked = server.hits.count("GET /custom_dict.json")
        invoked = [m["topic"] for m in events.events() if m["topic"] == "rs/app/command/sync_dictionary"]
        ok = checked and (asked >= 1 if auto else asked == 0 and not invoked)
        self.rec.auto(check, title, ok, f"the update check ran: {checked}; the dictionary was asked for {asked} time(s); "
                                        f"the UI invoked sync_dictionary {len(invoked)} time(s)")
        if auto:
            kept = self.accepted_revision(data)
            self.rec.auto("TS-meta-accepted", "a good publication is accepted: the revision is remembered", ok and kept == server.revision,
                          f"the dictionary was asked for {asked} time(s); metadata.json holds accepted_revision {kept} (the server's: {server.revision})")
        self.leave(events)

    @staticmethod
    def accepted_revision(data: Path) -> int | None:
        """`accepted_revision` in the app's `metadata.json` (None while it is missing or unreadable)."""
        try:
            return int(json.loads((data / "config" / "metadata.json").read_text(encoding="utf-8")).get("accepted_revision", 0))
        except (OSError, ValueError, AttributeError):
            return None

    @staticmethod
    def logged(events: bridge.Serve, source: str, level: str, containing: str) -> str | None:
        """The first system-log line from `source` at `level` that holds `containing`, else None."""
        for m in events.events():
            payload = m["message"].get("payload") if isinstance(m["message"], dict) else None
            if (m["topic"] == "rs/app/event/system-event" and isinstance(payload, dict) and payload.get("source") == source
                    and str(payload.get("level", "")).lower() == level and containing in str(payload.get("message", ""))):
                return str(payload["message"])
        return None

    def refused_round(self, row: str, title: str, *, mode: str = "ok", revision: int = 4, accepted: int | None = None, want: str) -> None:
        """The update check on and `auto_sync_latest_dict` on, the publication broken in one way: it must be refused -- no dictionary is
        fetched or saved, the system log has an Error line from `Metadata` that says why (`want`), the remembered revision is as it was --
        while the feed's app update is still announced."""
        server = self.signed_server(foreign=mode == "wrong-key")
        server.metadata_mode, server.revision = mode, revision
        exe = updater.fresh_copy(self.exe, self.runs, row)
        data = exe.parent / "data"
        (data / "config").mkdir(parents=True, exist_ok=True)
        (data / "config" / "config.json").write_text(json.dumps({"init_done": True, "auto_sync_latest_dict": True}), encoding="utf-8")
        if accepted is not None:
            (data / "config" / "metadata.json").write_text(json.dumps({
                "current_model_version": "0.0.0", "current_dict_version": "0.0.0", "ignored_app_version": None,
                "ignored_model_version": None, "last_update_check": 0, "accepted_revision": accepted}), encoding="utf-8")
        events = bridge.Serve(exe.parent / f"{row}-events.jsonl")
        self._serves.append(events)
        status = exe.parent / f"{row}-status.json"
        args = mockfeed.flag_args(data, status, log_file=exe.parent / f"{row}.log", fresh=False, feed_url=server.feed_url,
                                  metadata_url=server.metadata_url, metadata_trust_key=server.signer.public_key,
                                  dictionary_url=server.dictionary_url, extra=("--bridge-url", events.url))
        mockfeed.start_app(exe, args)
        if not bridge.wait_started(events, exe.parent / f"{row}.log", label=f"the app ({row})"):
            self.rec.auto(row, title, False, "no app-started event in 120 s")
            return
        deadline = time.monotonic() + WAIT_S
        error = None
        while time.monotonic() < deadline and error is None:
            error = self.logged(events, "Metadata", "error", want)
            time.sleep(0.5)
        announced = mockfeed.wait_status(status, lambda st: mockfeed.update_kind(st.get("update"))[0] == "available", timeout=WAIT_S)
        time.sleep(2)  # a dictionary fetch that should not happen would have started by now
        asked = server.hits.count("GET /custom_dict.json")
        on_disk = (data / "data" / "custom_dict.json").exists()
        kept = self.accepted_revision(data)
        ok = error is not None and asked == 0 and not on_disk and announced is not None and (kept or 0) == (accepted or 0)
        self.rec.auto(row, f"{title}: refused, nothing synced, an Error line, the app update still announced", ok,
                      f"Error line {error!r} (wanted one holding {want!r}); the dictionary was asked for {asked} time(s); custom_dict.json on disk: "
                      f"{on_disk}; accepted_revision {kept}; app update announced: {announced is not None}")
        self.leave(events)

    def hash_round(self) -> None:
        """Good metadata, but the dictionary served is not the file it names (a stale copy on a cache, say): `sync-dictionary` refuses it."""
        stub = LlamaStub().start()
        self._stubs.append(stub)
        server = self.signed_server()
        exe = updater.fresh_copy(self.exe, self.runs, "meta-hash")
        events = self.launch("meta-hash", exe, stub, {"init_done": True, "use_translation": True},
                             extra=(*server.metadata_flags(), "--dictionary-url", server.dictionary_url))
        if events is None:
            self.rec.auto("TS-meta-hash", "the hash run started", False, "no app-started event in 120 s")
            return
        on_disk = exe.parent / "data" / "data" / "custom_dict.json"
        server.dictionary_override = '{"term": {"ボス": "evil"}}'
        try:
            events.send("sync-dictionary", {}, timeout=30)
            refused, detail = False, "the app took a dictionary that is not the signed one"
        except RuntimeError as e:
            refused, detail = "does not match" in str(e), str(e)
        error = self.logged(events, "Metadata", "error", "does not match")
        saved = on_disk.exists()
        server.dictionary_override = None
        try:
            events.send("sync-dictionary", {}, timeout=30)
            again = on_disk.exists() and on_disk.read_text(encoding="utf-8") == server.dictionary_text
            again_detail = "synced"
        except RuntimeError as e:
            again, again_detail = False, str(e)
        self.rec.auto("TS-meta-hash", "a dictionary that is not the signed one is refused (nothing saved); served right, the same sync works",
                      refused and error is not None and not saved and again,
                      f"first sync: {detail}; Error line {error!r}; saved after it: {saved}; second sync: {again_detail}, "
                      f"file holds the served dictionary: {again}")
        self.leave(events)

    def slow_round(self) -> None:
        """A slow server that is still loading when the lines arrive: the catch-up serves a live line before its last item; and a request
        the server never answers costs one line, not the translator."""
        stub = LlamaStub(loading_seconds=SLOW_LOADING_S).start()
        stub.completion_delay = SLOW_DELAY_S
        self._stubs.append(stub)
        exe = updater.fresh_copy(self.exe, self.runs, "slow")
        events = self.launch("slow", exe, stub, {"init_done": True, "use_translation": True})
        if events is None:
            self.rec.auto("TS-live-first", "a live line is served during the catch-up", False, "no app-started event in 120 s")
            return
        ts = int(time.time())
        owed, live = LATER[:4], LATER[4]
        self.replay(events, exe.parent / "slow-owed.jsonl", [line(t, 30 + i, ts, 0 if i == 0 else 100) for i, t in enumerate(owed)])
        try:
            events.expect("translator-state", {"payload.state": "Catching Up"}, timeout=SLOW_LOADING_S + WAIT_S)
        except RuntimeError as e:
            self.rec.auto("TS-live-first", "a live line is served during the catch-up", False, f"{e}; states {self.states(events)}")
            self.leave(events)
            return
        self.replay(events, exe.parent / "slow-live.jsonl", [line(live, 40, ts + 5, 0)])
        deadline = time.monotonic() + SLOW_DELAY_S * 8 + WAIT_S / 2
        while time.monotonic() < deadline and len(self.done(events)) < len(LATER):
            time.sleep(0.5)
        asked = [source_of(r["prompt"]) for r in stub.requests]
        ok = live in asked and asked.index(live) < len(asked) - 1 and len(self.done(events)) == len(LATER)
        self.rec.auto("TS-live-first", "a live line said during the catch-up is translated before its last item, not after all of them", ok,
                      f"the server was asked, in order: {asked}; {len(self.done(events))} of {len(LATER)} translated")
        self.hang(events, stub, exe, ts)
        self.leave(events)

    def hang(self, events: bridge.Serve, stub: LlamaStub, exe: Path, ts: int) -> None:
        stub.completion_delay = 0
        stub.completion_mode = "hang"
        before = len(stub.requests)
        states = len(self.states(events))
        self.replay(events, exe.parent / "hang-1.jsonl", [line("ちょっと待って", 50, ts + 10, 0)])
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline and len(stub.requests) == before:
            time.sleep(0.2)
        stub.completion_mode = "ok"  # the hung request stays hung; the next one is answered
        began = time.monotonic()
        self.replay(events, exe.parent / "hang-2.jsonl", [line("もう一度", 51, ts + 11, 0)])
        limit = REQUEST_TIMEOUT_S + HANG_MARGIN_S
        while time.monotonic() < began + limit and "もう一度" not in self.done(events):
            time.sleep(0.5)
        took = time.monotonic() - began
        history = self.done(events)
        later = self.states(events)[states:]
        stub.release_hang()
        ok = bool(history.get("もう一度")) and "Restarting" not in later
        self.rec.auto("TS-hang", f"a request the server never answers costs one line, not the translator (the app's limit is {REQUEST_TIMEOUT_S:.0f} s)", ok,
                      f"the next line was translated after {took:.0f} s: {bool(history.get('もう一度'))}; the hung line translated: "
                      f"{bool(history.get('ちょっと待って'))}; translator states since: {later}")

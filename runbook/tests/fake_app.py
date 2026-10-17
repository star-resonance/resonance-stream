"""A stand-in for a test-env build of Resonance Stream -- only for the dry run of updater-mock.ipynb.

It reads the same flags, writes the same status file and follows the same update steps (check the
feed, download, verify, swap + restart), so the pipeline's steps, waits and checks can be exercised
on any OS. With --bridge-url it also talks to the bridge's MQTT broker the way src-tauri/src/bridge.rs
does (a minimal MQTT 3.1.1 client): it publishes `app-started` and `update-state` events and obeys the
`ping`, `start-update`, `restart-update` and `quit` commands -- nobody "clicks" any more. It is NOT the
app: what it does is this author's reading of src-tauri, and proves nothing about Windows or the real exe.

The notebook's "exe" is a shell script that runs this file (so bytes appended to it, as the
notebook does for the new version, are never parsed). Environment:
  FAKE_APP_EXE            path of the "exe" (set by the script)
  FAKE_APP_SIGNED_FILE    the file the "signature" is valid for: a download must equal it
  FAKE_APP_VERSION        the version it reports (default 0.5.0)
  FAKE_APP_LIFETIME       seconds until it exits by itself (default 40)
  FAKE_APP_STALL_AFTER    seconds of silence it takes for a stalled download (default 3)
  FAKE_APP_SKIP_VERIFY=1  a bug to catch: accepts any download
  FAKE_APP_TOUCH_REAL=1   a bug to catch: also writes to the real %APPDATA% config
  FAKE_APP_IFACE_LINE     what the sniffer says when it is on (no --no-capture): the text of its system-event,
                          e.g. "Auto-Targeting Network Interface: 192.168.0.23 (default route)"
  FAKE_APP_RECT           the main window's first rect, "WxH" (default 800x600; also x,y = 100,100)
  FAKE_APP_SCREEN         the work area, "WxH" (default 1920x1040): a grow never goes beyond it
  FAKE_APP_K18_BUG        a bug to catch: "quit", "close" or "both" -- that way out saves the GROWN window
                          (what window-state would do if settings were still open at exit)
  FAKE_APP_BAD_STATUS=1   a bug to catch: a status file without pid and data_dir
  FAKE_APP_CHAT_BUG       a bug to catch in the chat rules: "no-dedupe" publishes a repeated line again, "no-retro" blocks
                          only later lines (earlier rows keep their flag), "block-later" never flags later lines of a blocked
                          sender, "clear-keeps" leaves the history when told to clear it, "ruby-merged" answers `annotate-furigana`
                          with one list for all the lines, "ruby-lossy" drops a "!" from a line, "ruby-marks-plain" gives a line without
                          kanji a reading, "ruby-katakana" reads in katakana
  FAKE_APP_PERSIST_BUG    a bug to catch in what survives a restart: "small-packets" never sends an answer over 10 KiB (the MQTT client's
                          default limit), "no-archive" writes no chat log, "world-archived" also archives WORLD
                          (ignored by default), "no-config" does not save the block list, "no-reload" starts with an empty log,
                          "pid-restart" numbers new lines from 1 again, "unflagged-reload" restores a blocked sender's rows unflagged,
                          "no-retention" never prunes old day logs, "retention-takes-all" prunes files that are no day log too,
                          "global-limit" reloads the newest N of ALL channels (N = the WORLD limit) instead of N per channel,
                          "fav-not-saved" does not write the favorites to config.json, "fav-renumbered" hands the favorites tabs new ids 1..n,
                          "fav-clobbered" lets a block-list write bring the old favorites back, "fav-silent" never says `favorites-changed`
  FAKE_APP_DL_BUG         a bug to catch in the model download: "no-verify" accepts any bytes, "keeps-part" leaves its partial file after a
                          failure, "overwrites-on-fail" replaces the installed model before checking the new one, "accepts-http" takes
                          a plain-http address that is not this machine's
  FAKE_APP_REQUEST_TIMEOUT  seconds the translator waits for the server's reply (default 10; the real client waits 30)
  FAKE_APP_TR_BUG         a bug to catch in the translator: "no-catchup" never retries the lines that failed, "no-restart" gives up after three failures,
                          "translates-english" also sends lines that are not Japanese, "no-limit" catches up every owed line instead of the
                          newest translation_catch_up_limit, "live-starves" serves live lines only after the whole catch-up,
                          "poll-spam" puts a "Polling .../health" line in the system log for every health poll,
                          "hang-stalls" never gives up on a request the server does not answer, "reload-twice" serves a row that was
                          saved twice (untranslated, then translated) twice after a restart
  FAKE_APP_POPUP_BUG      a bug to catch in the popup windows: "second-window" makes another window when a popup is opened twice, "no-restore"
                          opens a popup at its default place instead of where it was left, "pin-ignored" leaves the popups unpinned when the main
                          window is pinned
  FAKE_APP_SNIFF_BUG      a bug to catch in the capture: "drop-after-2" publishes only the first two chats it reads,
                          "never" does not start the sniffer at all, "no-watchdog" never notices a silent game, "no-throttle" writes the
                          watchdog's line on every trip, "vpn-mismatch" names the VPN in the log but not on the badge, "no-remember" does
                          not teach the capture the chat reloaded from the logs (a re-sent line is shown again), "restart-dead" takes
                          `restart-sniffer` and never starts the sniffer again
  FAKE_APP_DICT_BUG       a bug to catch in the custom dictionary: "sync-not-installed" saves a synced dictionary but only uses it after a
                          restart, "local-needs-restart" does the same for the editor's save, "bad-saved" writes a text that does not parse
                          before it notices, "auto-always" syncs at start-up although `auto_sync_latest_dict` is off
  FAKE_APP_READY_DELAY    seconds after start before the app takes a command (the real one holds a command until its start-up is done,
                          which takes ~20 s with the sniffer on); `quit` is taken at once
  FAKE_APP_WATCHDOG       "check,trip,window" seconds of the stand-in watchdog: how often it looks, how long a silence trips it, how long
                          an identical log line is held back (default 5,15,60 like the app)
  FAKE_APP_VPN            the name of a VPN adapter the default route runs through (the watchdog says so)
                          (the stand-in "sniffer" is a TCP client of the port-5003 server the capture-spike pipeline starts)
"""
from __future__ import annotations

import base64
import datetime
import hashlib
import http.client
import json
import os
import shutil
import queue
import re
import socket
import struct
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

SWITCHES = {"fresh", "assume-setup-done", "no-capture", "no-translator", "no-update-check", "no-popups",
            "no-window-state", "print-env"}
VALUES = {"data-dir", "feed-url", "metadata-url", "metadata-trust-key", "dictionary-url", "status-file", "log-file", "bridge-url", "llama-url"}


def parse(argv: list[str]) -> dict:
    flags: dict = {}
    it = iter(argv)
    for arg in it:
        name = arg[2:] if arg.startswith("--") else None
        if name in SWITCHES:
            flags[name] = True
        elif name in VALUES:
            flags[name] = next(it, "")
        else:
            print(f"resonance-stream: unknown test flag {arg}", file=sys.stderr)
            sys.exit(2)
    return flags


def version_tuple(text: str) -> tuple[int, ...]:
    return tuple(int(part) for part in text.split("."))


class Mqtt:
    """The least MQTT 3.1.1 a stand-in needs: connect (with a will), publish at QoS 0, subscribe, and hand every
    PUBLISH it receives to `inbox`. Not a library -- it only talks to the bridge's local broker."""

    def __init__(self, url: str, client_id: str, will_topic: str, will_payload: bytes) -> None:
        parts = urllib.parse.urlparse(url)
        self.sock = socket.create_connection((parts.hostname, parts.port), timeout=10)
        self.inbox: queue.Queue = queue.Queue()
        body = self._string("MQTT") + bytes([4, 0x2E]) + struct.pack(">H", 0) + self._string(client_id)
        body += self._string(will_topic) + struct.pack(">H", len(will_payload)) + will_payload
        self._send(0x10, body)
        if self._read_packet()[0] >> 4 != 2:
            raise OSError("no CONNACK from the broker")
        self.sock.settimeout(None)
        threading.Thread(target=self._reader, daemon=True).start()

    @staticmethod
    def _string(text: str) -> bytes:
        raw = text.encode("utf-8")
        return struct.pack(">H", len(raw)) + raw

    def _send(self, first: int, body: bytes) -> None:
        length, out = len(body), bytearray()
        while True:
            digit, length = length % 128, length // 128
            out.append(digit | (0x80 if length else 0))
            if not length:
                break
        self.sock.sendall(bytes([first]) + bytes(out) + body)

    def _recv(self, n: int) -> bytes:
        data = b""
        while len(data) < n:
            chunk = self.sock.recv(n - len(data))
            if not chunk:
                raise OSError("the broker closed the connection")
            data += chunk
        return data

    def _read_packet(self) -> tuple[int, bytes]:
        first = self._recv(1)[0]
        length, shift = 0, 0
        while True:
            digit = self._recv(1)[0]
            length |= (digit & 0x7F) << shift
            shift += 7
            if not digit & 0x80:
                break
        return first, self._recv(length)

    def _reader(self) -> None:
        try:
            while True:
                first, body = self._read_packet()
                if first >> 4 == 3:  # PUBLISH
                    (n,) = struct.unpack(">H", body[:2])
                    topic = body[2:2 + n].decode("utf-8")
                    rest = body[2 + n:]
                    if (first >> 1) & 3:
                        rest = rest[2:]  # a packet id: we subscribe at QoS 0, so the broker sends none
                    self.inbox.put((topic, rest))
        except OSError:
            pass

    def publish(self, topic: str, payload: bytes | str, retain: bool = False) -> None:
        data = payload.encode("utf-8") if isinstance(payload, str) else payload
        self._send(0x30 | (1 if retain else 0), self._string(topic) + data)

    def subscribe(self, topic_filter: str) -> None:
        self._send(0x82, struct.pack(">H", 1) + self._string(topic_filter) + bytes([0]))


def _varint(data: bytes, at: int) -> tuple[int, int]:
    value = shift = 0
    while at < len(data):
        byte = data[at]
        at += 1
        value |= (byte & 0x7F) << shift
        if not byte & 0x80:
            break
        shift += 7
    return value, at


def _fields(data: bytes):
    """(field number, value) of a protobuf message: an int for a varint, bytes for a length-delimited field."""
    at = 0
    while at < len(data):
        tag, at = _varint(data, at)
        kind, number = tag & 7, tag >> 3
        if kind == 0:
            value, at = _varint(data, at)
        elif kind == 2:
            size, at = _varint(data, at)
            value, at = data[at:at + size], at + size
        else:
            return
        yield number, value


def decode_chat_frame(frame: bytes) -> dict | None:
    """A live chat frame (`[len][0x0002][16-byte header][root]`) as the ChatMessage JSON the app publishes; None for the rest.
    The stand-in's own reading of the layout resonance_core::capture::synth writes."""
    if len(frame) < 23 or frame[4:6] != b"\x00\x02" or frame[22] != 0x0A:
        return None
    root_len, at = _varint(frame, 23)
    chat = {"channel": "WORLD", "nickname": "", "uid": 0, "level": 0, "message": ""}
    names = {1: "WORLD", 2: "LOCAL", 3: "PARTY", 4: "GUILD", 9: "BEGINNER"}
    for number, value in _fields(frame[at:at + root_len]):
        if number == 1 and isinstance(value, int):
            chat["channel"] = names.get(value, "WORLD")
        elif number == 2 and isinstance(value, bytes):
            for n2, v2 in _fields(value):
                if n2 == 1:
                    chat["sequenceId"] = v2
                elif n2 == 2:
                    for n3, v3 in _fields(v2):
                        if n3 == 1:
                            chat["uid"] = v3
                        elif n3 == 2:
                            chat["nickname"] = v3.decode("utf-8", "replace")
                        elif n3 == 5:
                            chat["level"] = v3
                elif n2 == 4:
                    for n3, v3 in _fields(v2):
                        if n3 == 3:
                            chat["message"] += v3.decode("utf-8", "replace")
    return chat if chat["message"] else None


class App:
    def __init__(self, flags: dict) -> None:
        self.flags = flags
        self.exe = Path(os.environ.get("FAKE_APP_EXE", sys.argv[0])).resolve()
        self.version = os.environ.get("FAKE_APP_VERSION", "0.5.0")
        data = flags.get("data-dir")
        self.status = {
            "version": self.version,
            "exe": str(self.exe),
            "pid": os.getpid(),
            "ready": False,
            "config_dir": str(Path(data) / "config") if data else None,
            "data_dir": str(Path(data) / "data") if data else None,
            "flags": [n for n in [*sorted(SWITCHES), *sorted(VALUES)] if n in flags],
            "update": "none",
        }
        w, _, h = os.environ.get("FAKE_APP_RECT", "800x600").partition("x")
        self.rect = {"x": 100, "y": 100, "width": int(w), "height": int(h)}
        self.grown_from: dict | None = None
        self.bridge: Mqtt | None = None
        self.seq = 0
        self.announced_url: str | None = None
        self.stop = False
        # the chat rules (sniffer/mod.rs, capture/message_processor.rs, events.rs, as understood)
        self.history: list[dict] = []
        self.signatures: dict[tuple, int] = {}
        self.contents: dict[tuple, int] = {}
        self.blocked: dict[int, str] = {}
        self.ignored = ["WORLD"]
        self.next_pid = 1
        # the translator (translator/mod.rs, as understood)
        self.jobs: queue.Queue = queue.Queue()
        self.owed: list[int] = []
        self.translator_on = False
        self.use_translation = False
        self.catch_up_limit = 100
        self.last_traffic = time.monotonic()
        self.sniff_gen, self.sniff_sock, self.watchdog_started = 0, None, False  # restart-sniffer ends the old reader (a newer generation)
        self.started_at = time.monotonic()
        self.tr_seq = 0
        # the popup windows (window.rs, as understood)
        self.popups: dict[str, dict] = {}
        self.main_on_top = False

    def write_status(self) -> None:
        path = self.flags.get("status-file")
        if path:
            tmp = Path(path + ".tmp")
            tmp.parent.mkdir(parents=True, exist_ok=True)
            status = dict(self.status)
            if os.environ.get("FAKE_APP_BAD_STATUS") == "1" and "print-env" not in self.flags:  # a bug to catch: a status file missing keys
                status.pop("pid", None)
                status.pop("data_dir", None)
            tmp.write_text(json.dumps(status), encoding="utf-8")
            tmp.replace(path)

    def log(self, text: str) -> None:
        path = self.flags.get("log-file")
        if path:
            Path(path).parent.mkdir(parents=True, exist_ok=True)
            with open(path, "a", encoding="utf-8") as f:
                f.write(text + "\n")

    def set_update(self, state: str) -> None:
        self.status["update"] = state
        self.write_status()
        self.log(f"update -> {state}")
        self.event("update-state", {"state": state})

    # -- the bridge (src-tauri/src/bridge.rs, as understood)
    def event(self, name: str, payload: dict) -> None:
        if self.bridge is None:
            return
        body = {"seq": self.seq, "t_ms": int(time.time() * 1000), "name": name, "payload": payload}
        self.seq += 1
        self.bridge.publish(f"rs/app/event/{name}", json.dumps(body))

    def connect_bridge(self) -> None:
        url = self.flags.get("bridge-url")
        if not url:
            return
        self.bridge = Mqtt(url, "resonance-stream-app", "rs/app/status", b"offline")
        self.bridge.subscribe("rs/test/command/+")
        self.event("app-started", {"pid": os.getpid(), "version": self.version, "exe": str(self.exe)})
        self.bridge.publish("rs/app/status", "online", retain=True)

    # -- the sniffer (services/sniffer, as understood): a TCP client of the capture-spike pipeline's port-5003 server stands in for
    #    the raw socket on the adapter named by `network_interface` in config.json
    def start_sniffer(self) -> None:
        data = self.flags.get("data-dir")
        config = Path(data) / "config" / "config.json" if data else None
        ip = json.loads(config.read_text(encoding="utf-8")).get("network_interface") if config and config.exists() else None
        if not ip or os.environ.get("FAKE_APP_SNIFF_BUG") == "never":  # "never": a bug to catch, the sniffer does not start
            return
        self.event("sniffer-state", {"state": "Pending", "message": "Listening for game traffic...", "seq": 1})
        self.last_traffic = time.monotonic()
        self.sniff_gen += 1
        threading.Thread(target=self._sniff, args=(ip, self.sniff_gen), daemon=True).start()
        if os.environ.get("FAKE_APP_SNIFF_BUG") != "no-watchdog" and not self.watchdog_started:
            self.watchdog_started = True
            threading.Thread(target=self._watchdog, daemon=True).start()

    def restart_sniffer(self) -> None:
        """`restart_sniffer_command`, as understood: the old worker ends, 0.5 s for the OS to release the socket, then a fresh one."""
        self.sniff_gen += 1  # the old reader ends at its next read
        old, self.sniff_sock = self.sniff_sock, None
        for call in ("shutdown", "close"):
            try:
                getattr(old, call)(*((socket.SHUT_RDWR,) if call == "shutdown" else ()))
            except (OSError, AttributeError, TypeError):
                pass
        if os.environ.get("FAKE_APP_SNIFF_BUG") == "restart-dead":  # a bug to catch: acked, never started again
            return

        def again() -> None:
            time.sleep(0.5)
            if not self.stop:
                self.start_sniffer()

        threading.Thread(target=again, daemon=True).start()

    def _watchdog(self) -> None:
        """sniffer/mod.rs spawn_watchdog + the throttled system log, as understood: a silence longer than `trip` says so (log + red badge),
        the same log line is held back for `window` seconds and then comes with a count of what it held back."""
        check, trip, window = (float(x) for x in os.environ.get("FAKE_APP_WATCHDOG", "5,15,60").split(","))
        vpn = os.environ.get("FAKE_APP_VPN")
        written, held = None, 0
        while not self.stop:
            time.sleep(check)
            if time.monotonic() - self.last_traffic <= trip:
                continue
            text = "Watchdog: No game traffic for 15s."
            if vpn:
                text = f"{text[:-1]} The default route runs through a VPN adapter ({vpn}); turn the VPN off or exclude the game from it."
            now = time.monotonic()
            if os.environ.get("FAKE_APP_SNIFF_BUG") == "no-throttle" or written is None or now - written >= window:
                self.event("system-event", {"pid": 0, "level": "warning", "source": "Sniffer",
                                            "message": text + (f" (repeated {held} more times)" if held else "")})
                written, held = now, 0
            else:
                held += 1
            badge = "게임 트래픽 감지 안됨 (클릭하여 어댑터 복구)"
            if vpn and os.environ.get("FAKE_APP_SNIFF_BUG") != "vpn-mismatch":
                badge = f"게임 트래픽 감지 안됨 (VPN 사용 중: {vpn} - VPN을 끄거나 게임을 터널에서 제외하세요)"
            self.event("sniffer-state", {"state": "Error", "message": badge, "seq": 2})
            self.last_traffic = now

    def _sniff(self, ip: str, gen: int) -> None:
        deadline = time.monotonic() + 60
        sock = None
        while sock is None and time.monotonic() < deadline and gen == self.sniff_gen:
            try:
                sock = socket.create_connection((ip, 5003), timeout=2)
            except OSError:
                time.sleep(0.3)
        if sock is None:
            return
        self.sniff_sock = sock
        sock.settimeout(None)
        buf, count = b"", 0
        while True:
            try:
                data = sock.recv(65536)
            except OSError:
                return
            if not data or gen != self.sniff_gen:
                return
            buf += data
            while len(buf) >= 6 and len(buf) >= int.from_bytes(buf[:4], "big"):
                size = int.from_bytes(buf[:4], "big")
                chat = decode_chat_frame(buf[:size])
                buf = buf[size:]
                self.last_traffic = time.monotonic()
                if chat is None:
                    continue
                count += 1
                if os.environ.get("FAKE_APP_SNIFF_BUG") == "drop-after-2" and count > 2:
                    continue
                self.feed({**chat, "timestamp": 0, "sequenceId": chat.get("sequenceId", 0), "isBlocked": False})

    # -- the chat rules, fed by `replay-chat`
    def replay(self, path: str) -> None:
        lines = []
        for raw in Path(path).read_text(encoding="utf-8").splitlines():
            if raw.strip() and not raw.lstrip().startswith("#"):
                lines.append(json.loads(raw))
        threading.Thread(target=self._replay, args=(lines,), daemon=True).start()

    def _replay(self, lines: list[dict]) -> None:
        time.sleep(float(os.environ.get("FAKE_APP_LEAD_IN_MS", "300")) / 1000)
        for index, line in enumerate(lines):
            time.sleep(line.get("delay_ms", 0) / 1000)
            chat = {"channel": line.get("channel", "WORLD"), "nickname": line.get("nickname", "Tester"),
                    "uid": line.get("uid", 1), "level": line.get("level", 0), "message": line["text"],
                    "timestamp": line.get("timestamp", int(time.time())), "sequenceId": line.get("sequence_id", index + 1),
                    "isBlocked": False}
            self.feed(chat)
        self.event("system-event", {"pid": 0, "level": "info", "source": "Replay", "message": "Replay finished"})

    def feed(self, chat: dict) -> None:
        bug = os.environ.get("FAKE_APP_CHAT_BUG", "")
        if chat["uid"] in self.blocked and bug != "block-later":
            chat["isBlocked"] = True
        signature = (chat["uid"], chat["timestamp"], chat["sequenceId"])
        # the pipeline's second key (message_processor.rs): the same line from a second client has
        # another sequence id but the same sender, words and send time. No clock; none without a send time.
        content = (chat["uid"], chat["message"], chat["timestamp"]) if chat["timestamp"] else None
        seen = self.signatures.get(signature)
        if seen is None and content is not None:
            seen = self.contents.get(content)
        if seen is not None and bug != "no-dedupe":
            if chat["isBlocked"]:  # the pipeline's UpdateBlockedMessage
                row = next((m for m in self.history if m["pid"] == seen), None)
                if row and not row["isBlocked"]:
                    row["isBlocked"] = True
                    self.event("chat-message-update", row)
            return
        chat["pid"] = self.next_pid
        self.next_pid += 1
        self.signatures[signature] = chat["pid"]
        if content is not None:
            self.contents[content] = chat["pid"]
        self.history.append(chat)
        self.archive(chat)
        self.event("packet-event", chat)
        if self.japanese(chat["message"]) or os.environ.get("FAKE_APP_TR_BUG") == "translates-english":
            self.owed.append(chat["pid"])
            if self.translator_on and self.use_translation:
                self.jobs.put(chat["pid"])

    # -- what survives a restart (data_factory.rs, config/app_config.rs, lib.rs start-up reload, as understood)
    def data_path(self, *parts: str) -> Path | None:
        data = self.flags.get("data-dir")
        return Path(data).joinpath(*parts) if data else None

    def load_persisted(self) -> None:
        bug = os.environ.get("FAKE_APP_PERSIST_BUG", "")
        config = self.data_path("config", "config.json")
        saved: dict = {}
        if config and config.exists():
            saved = json.loads(config.read_text(encoding="utf-8"))
            self.blocked = {int(uid): name for uid, name in saved.get("blocked_users", {}).items()}
        self.favorites = {"messages": saved.get("favorite_messages", []), "tabs": saved.get("favorite_tabs", [])}
        self.auto_sync_dict = bool(saved.get("auto_sync_latest_dict", False))
        self.dict_version = ""
        self.dictionary = {}
        path = self.data_path("data", "custom_dict.json")
        if path is not None and path.exists():
            try:
                self.dictionary = self.parse_dictionary(path.read_text(encoding="utf-8"))
            except ValueError:
                pass
        self.ignored = saved.get("archive_ignored_channels", ["WORLD"])
        self.use_translation = bool(saved.get("use_translation", False))
        self.catch_up_limit = int(saved.get("translation_catch_up_limit", 100))
        limits = {"WORLD": 200, **saved.get("tab_limits", {})}
        logs = self.data_path("data", "chat_logs")
        if logs and logs.is_dir() and bug != "no-reload":
            self.prune(logs, int(saved.get("chat_log_retention_days", 0)), bug)
            rows = []
            for file in sorted(logs.glob("*.jsonl")):
                for raw in file.read_text(encoding="utf-8").splitlines():
                    try:
                        row = json.loads(raw)
                    except ValueError:  # load_recent skips a line that is no chat message
                        continue
                    row["isBlocked"] = row["uid"] in self.blocked and bug != "unflagged-reload"
                    rows.append(row)
            if os.environ.get("FAKE_APP_TR_BUG") != "reload-twice":  # load_recent: a message saved twice (untranslated, then translated) comes back once, newest line
                newest = {(r["uid"], r["timestamp"], r["sequenceId"]): i for i, r in enumerate(rows)}
                rows = [r for i, r in enumerate(rows) if newest[(r["uid"], r["timestamp"], r["sequenceId"])] == i]
            if bug == "global-limit":
                rows = rows[-limits["WORLD"]:]
            else:  # load_recent: the newest `limit` of each channel, oldest first
                kept: dict[str, int] = {}
                newest_first = []
                for row in reversed(rows):
                    kept[row["channel"]] = kept.get(row["channel"], 0) + 1
                    if kept[row["channel"]] <= limits.get(row["channel"], 1000):
                        newest_first.append(row)
                rows = newest_first[::-1]
            self.history.extend(rows)
            for pid, row in enumerate(self.history, start=1):  # load_recent: saved pids come from earlier runs, so 1..=n again
                row["pid"] = pid
            if bug != "pid-restart":
                self.next_pid = len(self.history) + 1
            for row in self.history:
                if os.environ.get("FAKE_APP_SNIFF_BUG") != "no-remember":  # the capture is taught the chat reloaded from disk
                    self.signatures[(row["uid"], row["timestamp"], row["sequenceId"])] = row["pid"]
                    if row["timestamp"]:  # and its second key (the same words from a second client)
                        self.contents[(row["uid"], row["message"], row["timestamp"])] = row["pid"]

    @staticmethod
    def prune(logs: Path, keep_days: int, bug: str) -> None:
        """data_factory::prune_chat_logs: a day log older than `keep_days` days (today counts as one) goes; 0 keeps all."""
        if keep_days <= 0 or bug == "no-retention":
            return
        oldest = datetime.date.today() - datetime.timedelta(days=keep_days - 1)
        for file in logs.glob("*.jsonl"):
            try:
                day = datetime.date.fromisoformat(file.stem)
            except ValueError:  # not a day log: never touched
                if bug == "retention-takes-all":
                    file.unlink()
                continue
            if day < oldest:
                file.unlink()

    def archive(self, chat: dict) -> None:
        bug = os.environ.get("FAKE_APP_PERSIST_BUG", "")
        logs = self.data_path("data", "chat_logs")
        ignored = [] if bug == "world-archived" else self.ignored
        if logs is None or bug == "no-archive" or chat["channel"] in ignored:
            return
        logs.mkdir(parents=True, exist_ok=True)
        with open(logs / f"{time.strftime('%Y-%m-%d')}.jsonl", "a", encoding="utf-8") as f:
            f.write(json.dumps(chat, ensure_ascii=False) + "\n")

    def save_blocked(self) -> None:
        config = self.data_path("config", "config.json")
        if config is None or os.environ.get("FAKE_APP_PERSIST_BUG") == "no-config":
            return
        saved = json.loads(config.read_text(encoding="utf-8")) if config.exists() else {}
        saved["blocked_users"] = {str(uid): name for uid, name in self.blocked.items()}
        if os.environ.get("FAKE_APP_PERSIST_BUG") == "fav-clobbered":  # a bug to catch: another config write brings the old favorites back
            self.favorites = {"messages": [], "tabs": []}
            saved["favorite_messages"], saved["favorite_tabs"] = [], []
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text(json.dumps(saved), encoding="utf-8")

    # -- furigana (resonance_core::furigana, as understood): the spans of a line, joined, are the line; a run of kanji gets a hiragana
    #    reading, everything else is plain. A toy dictionary: the contract is what the pipeline judges, not the readings.
    READINGS = {"日韓辞書": "にっかんじしょ", "今日": "きょう", "募集": "ぼしゅう", "一人": "ひとり", "二人": "ふたり",
                "一人前": "いちにんまえ", "行": "い"}

    def annotate(self, text: str) -> list[dict]:
        bug = os.environ.get("FAKE_APP_CHAT_BUG", "")
        spans = []
        for run in re.findall(r"[\u4e00-\u9fff]+|[^\u4e00-\u9fff]+", text):
            if bug == "ruby-lossy":
                run = run.replace("!", "")
            if not run:
                continue
            if re.match(r"[\u4e00-\u9fff]", run):
                reading = self.READINGS.get(run, "よみ")
                if bug == "ruby-katakana":
                    reading = "".join(chr(ord(c) + 0x60) if "ぁ" <= c <= "ゖ" else c for c in reading)
                spans.append({"text": run, "reading": reading})
            elif bug == "ruby-marks-plain":
                spans.append({"text": run, "reading": run})
            else:
                spans.append({"text": run})
        return spans

    # -- the custom dictionary (downloader/gist.rs, text.rs, as understood): {"category": {"ja": "ko"}}; a term is shielded as [P<n>] on
    #    its way to the server and put back in the answer, longest term first
    @staticmethod
    def parse_dictionary(text: str) -> dict[str, str]:
        if not text.strip():
            return {}
        root = json.loads(text)
        if not isinstance(root, dict):
            raise ValueError("Root JSON is not an object.")
        terms = {}
        for inner in root.values():
            if isinstance(inner, dict):
                terms.update({ja: ko for ja, ko in inner.items() if ja and isinstance(ko, str)})
        return terms

    def shield(self, text: str) -> tuple[str, dict[str, str]]:
        masked, restore = text, {}
        for ja, ko in sorted(self.dictionary.items(), key=lambda t: (-len(t[0]), t[0])):
            if ja in masked:
                key = f"[P{len(restore)}]"
                masked, restore[key] = masked.replace(ja, key), ko
        return masked, restore

    def save_dictionary_file(self, text: str) -> None:
        path = self.data_path("data", "custom_dict.json")
        if path is not None:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")

    # -- the signed metadata (downloader/gist.rs, core signed_metadata.rs, as understood). The stand-in cannot check minisign: it reads the
    #    dry-run scheme of `mockfeed.MetadataSigner` (FAKESIG: key id, SHA-256 of the body, revision), with the real app's refusal texts
    def read_text_url(self, url: str) -> str | None:
        """The body at `url`; None for a 404."""
        try:
            with urllib.request.urlopen(url, timeout=10) as r:  # noqa: S310 -- local mock
                return r.read().decode("utf-8")
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return None
            raise

    def accepted_revision(self) -> int:
        path = self.data_path("config", "metadata.json")
        try:
            return int(json.loads(path.read_text(encoding="utf-8")).get("accepted_revision", 0)) if path else 0
        except (OSError, ValueError, AttributeError):
            return 0

    def remember_revision(self, revision: int) -> None:
        path = self.data_path("config", "metadata.json")
        if path is None or revision <= self.accepted_revision():
            return
        try:
            saved = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            saved = {"current_model_version": "0.0.0", "current_dict_version": "0.0.0", "ignored_app_version": None,
                     "ignored_model_version": None, "last_update_check": 0}
        saved["accepted_revision"] = revision
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(saved), encoding="utf-8")

    def verify_metadata(self) -> tuple[dict | None, str]:
        """(the metadata, "") when it is signed by the trusted key for its revision and not older than the accepted one, else (None, why)."""
        url = self.flags.get("metadata-url")
        if not url:
            return None, "no --metadata-url"
        body = self.read_text_url(url)
        if body is None:
            return None, "Metadata: nothing is published at the metadata address"
        sig = self.read_text_url(url + ".sig")
        if sig is None:
            return None, "no signature is published for the metadata"
        try:
            metadata = json.loads(body)
            revision = int(metadata["revision"])
            key_id = base64.b64decode(self.flags.get("metadata-trust-key", "")).decode().splitlines()[1]
            tag, signer, digest, signed_rev = base64.b64decode(sig.strip()).decode().splitlines()[:4]
        except (ValueError, KeyError, IndexError, TypeError):
            return None, "the metadata signature was refused: the update signature is malformed"
        if tag != "FAKESIG" or signer != key_id or digest != hashlib.sha256(body.encode("utf-8")).hexdigest():
            return None, "the metadata signature was refused: the update is not signed by a trusted key"
        if int(signed_rev) != revision:
            return None, f"the metadata signature was refused: the update is signed for version {signed_rev}, not the announced {revision}"
        if revision < self.accepted_revision():
            return None, f"the metadata is revision {revision}, older than the revision {self.accepted_revision()} already accepted"
        return metadata, ""

    def refuse(self, why: str) -> None:
        self.event("system-event", {"pid": 0, "level": "error", "source": "Metadata",
                                    "message": f"Model and dictionary updates were refused: {why}. The installed model and dictionary are kept."})

    def sync_dictionary(self) -> str | None:
        """`sync_dictionary`: fetch, validate, save, install, remember the version. The error text is the ack's."""
        url = self.flags.get("dictionary-url")
        if not url:
            return "no --dictionary-url: the stand-in only knows the stand-in server"
        try:
            metadata, why = self.verify_metadata()
            if metadata is None:
                self.refuse(why)
                return f"Model and dictionary updates were refused: {why}. The installed model and dictionary are kept."
            self.remember_revision(int(metadata["revision"]))
            with urllib.request.urlopen(url, timeout=10) as r:  # noqa: S310 -- local mock
                text = r.read().decode("utf-8")
        except (OSError, ValueError, http.client.HTTPException) as e:
            return str(e)
        named = str(metadata.get("dictionary", {}).get("sha256", "")).lower()
        found = hashlib.sha256(text.encode("utf-8")).hexdigest()
        if named != found:
            line = (f"The dictionary was refused: the dictionary does not match the signed metadata (expected {named}, got {found}). "
                    "The installed dictionary is kept. A file published a moment ago can take a few minutes to reach every server; try again later.")
            self.event("system-event", {"pid": 0, "level": "error", "source": "Metadata", "message": line})
            return line
        try:
            terms = self.parse_dictionary(text)
        except ValueError as e:
            return f"Invalid dictionary received from Gist: {e}"
        self.save_dictionary_file(text)
        if os.environ.get("FAKE_APP_DICT_BUG") != "sync-not-installed":  # a bug to catch: used only after a restart
            self.dictionary = terms
        self.dict_version = str(metadata.get("dictionary", {}).get("version", ""))
        return None

    def save_local_dictionary(self, content: str) -> str | None:
        """`save_local_dictionary`: validate, write, install from the next job on."""
        bug = os.environ.get("FAKE_APP_DICT_BUG")
        if bug == "bad-saved":  # a bug to catch: written before it is looked at
            self.save_dictionary_file(content)
        try:
            terms = self.parse_dictionary(content)
        except ValueError as e:
            return f"JSON Syntax Error: {e}"
        if bug != "bad-saved":
            self.save_dictionary_file(content)
        if bug != "local-needs-restart":
            self.dictionary = terms
        return None

    def save_favorites(self, favorites: dict) -> None:
        """`save_favorites`, as understood: the favorites only (the block list and the rest of config.json stay), written, then
        `favorites-changed` to every window."""
        bug = os.environ.get("FAKE_APP_PERSIST_BUG")
        tabs = [{"id": int(t["id"]), "name": t["name"]} for t in favorites.get("tabs", [])]
        messages = [{"text": m["text"], "note": m.get("note", ""), "shortcut": m.get("shortcut", ""), "tab": int(m.get("tab", 0))}
                    for m in favorites["messages"]]
        if bug == "fav-renumbered":  # a bug to catch: the tab ids are handed out again, 1..n, in order
            renumber = {t["id"]: n for n, t in enumerate(tabs, start=1)}
            tabs = [{**t, "id": renumber[t["id"]]} for t in tabs]
            messages = [{**m, "tab": renumber.get(m["tab"], m["tab"])} for m in messages]
        self.favorites = {"messages": messages, "tabs": tabs}
        config = self.data_path("config", "config.json")
        if config is not None and bug != "fav-not-saved":
            saved = json.loads(config.read_text(encoding="utf-8")) if config.exists() else {}
            saved["favorite_messages"], saved["favorite_tabs"] = messages, tabs
            config.parent.mkdir(parents=True, exist_ok=True)
            config.write_text(json.dumps(saved, ensure_ascii=False), encoding="utf-8")
        if bug != "fav-silent":
            self.event("favorites-changed", self.favorites)

    def set_blocked(self, uid: int, blocked: bool, nickname: str = "") -> None:
        if blocked:
            self.blocked[uid] = nickname
        else:
            self.blocked.pop(uid, None)
        self.save_blocked()
        if os.environ.get("FAKE_APP_CHAT_BUG") == "no-retro":
            return
        for row in self.history:
            if row["uid"] == uid and row["isBlocked"] != blocked:
                row["isBlocked"] = blocked
                self.event("chat-message-update", row)

    # -- the popup windows: two kinds, made hidden ~2 s after start unless --no-popups, closing hides, the place is remembered
    POPUP_SIZES = {"popup-cheatsheet": (420, 560), "popup-favorites": (480, 620)}
    POPUP_NAMES = {"cheatsheet": "popup-cheatsheet", "favorites": "popup-favorites"}

    def popup_state_file(self) -> Path | None:
        data = self.flags.get("data-dir")
        return Path(data) / "config" / "popup-state.json" if data and "no-window-state" not in self.flags else None

    def make_popup(self, label: str) -> dict:
        width, height = self.POPUP_SIZES.get(label, (420, 560))
        popup = {"exists": True, "visible": False, "rect": {"x": 300, "y": 200, "width": width, "height": height},
                 "always_on_top": self.main_on_top}
        self.popups[label] = popup
        return popup

    def prewarm_popups(self) -> None:
        time.sleep(2)
        for label in self.POPUP_SIZES:
            if label not in self.popups:
                self.make_popup(label)

    def open_popup(self, label: str) -> None:
        bug = os.environ.get("FAKE_APP_POPUP_BUG", "")
        if label in self.popups and bug == "second-window":
            self.make_popup(label + "-2")
        popup = self.popups.get(label) or self.make_popup(label)
        path = self.popup_state_file()
        if path and path.exists() and not popup["visible"] and bug != "no-restore":
            saved = json.loads(path.read_text(encoding="utf-8")).get(label)
            if saved:
                popup["rect"] = saved
        popup["visible"] = True
        self.event("popup-shown", {})

    def popups_snapshot(self) -> dict:
        out = {"main": {"exists": True, "visible": True, "always_on_top": self.main_on_top, "rect": dict(self.rect)}}
        for label in self.POPUP_SIZES:
            out[label] = dict(self.popups[label]) if label in self.popups else {"exists": False}
        out["labels"] = sorted(["main", *self.popups])
        return out

    # -- the translator: lines with Japanese in them are owed a translation; a worker asks the server named by --llama-url
    @staticmethod
    def japanese(text: str) -> bool:
        return any(0x3040 <= ord(c) <= 0x30FF or 0x3400 <= ord(c) <= 0x4DBF or 0x4E00 <= ord(c) <= 0x9FFF
                   or 0xFF66 <= ord(c) <= 0xFF9F or c == "\u3005" for c in text)

    def tr_state(self, state: str, message: str = "") -> None:
        self.tr_seq += 1
        self.event("translator-state", {"state": state, "message": message, "seq": self.tr_seq})

    def start_translator(self) -> str | None:
        if self.translator_on:
            return None
        if "llama-url" not in self.flags:
            return "no --llama-url: the stand-in only knows the stand-in server"
        self.translator_on = True
        threading.Thread(target=self._translator, daemon=True).start()
        return None

    def ask(self, text: str) -> str | None:
        masked, restore = self.shield(text)
        prompt = ("<bos><start_of_turn>user\nYou are a professional Japanese (ja) to Korean (ko) translator.\n"
                  "Please translate the following Japanese text into Korean:\n" + masked + "<end_of_turn>\n<start_of_turn>model\n")
        body = json.dumps({"prompt": prompt, "stream": False, "temperature": 0.1, "n_predict": max(64, len(text) * 3 + 32),
                           "stop": ["<end_of_turn>", "<eos>"]}).encode()
        request = urllib.request.Request(self.flags["llama-url"] + "/completion", data=body, headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(request, timeout=self.request_timeout()) as r:  # noqa: S310 -- local stand-in
                content = json.loads(r.read()).get("content", "").strip()
        except (OSError, ValueError, http.client.HTTPException):
            return None
        for key, ko in restore.items():
            content = content.replace(key, ko)
        return content or None

    @staticmethod
    def request_timeout() -> float:
        if os.environ.get("FAKE_APP_TR_BUG") == "hang-stalls":
            return 3600.0
        return float(os.environ.get("FAKE_APP_REQUEST_TIMEOUT", "10"))

    def system(self, message: str, level: str = "info") -> None:
        self.event("system-event", {"pid": 0, "level": level, "source": "Translator", "message": message})

    def healthy(self) -> bool:
        try:
            with urllib.request.urlopen(self.flags["llama-url"] + "/health", timeout=3) as r:  # noqa: S310
                return r.status == 200
        except (OSError, ValueError):
            return False

    def _translator(self) -> None:
        bug = os.environ.get("FAKE_APP_TR_BUG", "")
        while not self.stop:
            self.tr_state("Starting", "Initializing AI Backend...")
            self.tr_state("Loading Model", "Loading AI weights into VRAM...")
            while not self.healthy():
                if bug == "poll-spam":
                    self.system(f"Polling {self.flags['llama-url']}/health...", "debug")
                time.sleep(0.3)
                if self.stop:
                    return
            while not self.jobs.empty():  # queued while the server loaded: they are owed instead
                try:
                    self.jobs.get_nowait()
                except queue.Empty:
                    break
            pids = list(self.owed)
            if bug != "no-limit":  # the newest `translation_catch_up_limit`; the older ones are passed over for good
                pids = pids[-self.catch_up_limit:] if self.catch_up_limit > 0 else []
                self.owed = [pid for pid in self.owed if pid in pids]
            if bug == "no-catchup":
                pids = []
            if pids:
                self.system(f"Catching up {len(pids)} missed message(s)...")
                for done, pid in enumerate(pids):
                    self.tr_state("Catching Up", f"{done + 1}/{len(pids)}")
                    while bug != "live-starves":  # live lines go before the next catch-up item
                        try:
                            live = self.jobs.get_nowait()
                        except queue.Empty:
                            break
                        self.translate(live)
                    self.translate(pid)
                self.system(f"Catch-up done ({len(self.owed)} still untranslated).")
            self.tr_state("Active", "AI Engine Ready")
            failures = 0
            while not self.stop:
                try:
                    pid = self.jobs.get(timeout=0.5)
                except queue.Empty:
                    continue
                if self.translate(pid):
                    failures = 0
                    continue
                failures += 1
                if failures >= 3:
                    break
            if bug == "no-restart" or self.stop:
                self.tr_state("Error", "AI Engine keeps stopping.")
                return
            self.tr_state("Restarting", "AI Engine stopped. Restarting in 2s...")
            time.sleep(2)

    def translate(self, pid: int) -> bool:
        row = next((m for m in self.history if m["pid"] == pid), None)
        if row is None or row.get("translated"):
            return True
        korean = self.ask(row["message"])
        if korean is None:
            return False
        row["translated"] = korean
        self.archive(row)
        if pid in self.owed:
            self.owed.remove(pid)
        self.event("translation-event", {"pid": pid, "translated": korean})
        return True

    # -- the model download (downloader/model.rs and fetch.rs, as understood)
    def progress(self, label: str, percent: int) -> None:
        self.event("download-progress", {"current_file": label, "percent": percent, "total_percent": percent})

    def download_model(self, request: dict) -> None:
        import hashlib
        import http.client

        bug = os.environ.get("FAKE_APP_DL_BUG", "")

        def done(error: str | None = None) -> None:
            self.event("download-result", {"id": request["id"], "what": "model", "ok": error is None, "error": error})

        # The model comes from the signed metadata, checked now (N-5); what the request carries is ignored.
        try:
            metadata, why = self.verify_metadata()
        except (OSError, http.client.HTTPException, ValueError) as e:
            return done(f"Network error: {e}")
        if metadata is None:
            self.refuse(why)
            return done(f"Model and dictionary updates were refused: {why}. The installed model and dictionary are kept.")
        self.remember_revision(int(metadata["revision"]))
        model = metadata.get("model", {})
        url, expected = str(model.get("download_url", "")).strip(), str(model.get("sha256", "")).strip().lower()
        if not expected:
            return done("No SHA-256 published for this model; refusing to download it")
        if not url:
            return done("No download address published for this model; refusing to download it")
        parts = urllib.parse.urlparse(url)
        local_http = parts.scheme == "http" and parts.hostname in ("127.0.0.1", "localhost")
        if parts.scheme != "https" and not local_http and bug != "accepts-http":
            return done(f"Refusing to download from a non-HTTPS URL: {url!r}")
        folder = self.data_path("data", "models", "translation-model")
        folder.mkdir(parents=True, exist_ok=True)
        dest, part = folder / "model.gguf", folder / "model.gguf.new.part"
        if dest.exists() and hashlib.sha256(dest.read_bytes()).hexdigest() == expected:
            self.progress("로컬 AI 모델 확인 완료 (Skipped download)", 100)
            return done()
        error = None
        try:
            with urllib.request.urlopen(url, timeout=10) as response:  # noqa: S310 -- local mock
                total = int(response.headers.get("Content-Length", "0"))
                data = b""
                while True:
                    chunk = response.read(65536)
                    if not chunk:
                        break
                    data += chunk
                    part.write_bytes(data)
                    self.progress("AI 모델 다운로드 중...", min(100, len(data) * 100 // max(total, 1)))
            if len(data) != total:
                error = f"Download incomplete: {len(data)} of {total} bytes"
            elif hashlib.sha256(data).hexdigest() != expected and bug != "no-verify":
                error = "SHA-256 mismatch: the downloaded file is not the published one"
        except (OSError, http.client.HTTPException, ValueError) as e:
            error = f"Download failed: {e}"
            data = b""
        if error is not None:
            if bug == "overwrites-on-fail" and data:
                dest.write_bytes(data)
            if bug != "keeps-part":
                part.unlink(missing_ok=True)
            return done(error)
        part.replace(dest)
        done()

    # -- the main window (window.rs and the window-state plugin, as understood)
    def state_file(self) -> Path | None:
        data = self.flags.get("data-dir")
        return Path(data) / "config" / "window-state.json" if data and "no-window-state" not in self.flags else None

    def load_window_state(self) -> None:
        path = self.state_file()
        if path and path.exists():
            self.rect = json.loads(path.read_text(encoding="utf-8"))

    def leave(self, way: str) -> None:
        """The app is closing by `way` ("quit": the tray's Quit, "close": the X): put a grown window back, save its state."""
        buggy = os.environ.get("FAKE_APP_K18_BUG") in (way, "both")
        if self.grown_from is not None and not buggy:
            self.rect, self.grown_from = self.grown_from, None
        path = self.state_file()
        if path:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(self.rect), encoding="utf-8")
        popup_path = self.popup_state_file()
        if popup_path:
            popup_path.parent.mkdir(parents=True, exist_ok=True)
            popup_path.write_text(json.dumps({label: p["rect"] for label, p in self.popups.items()}), encoding="utf-8")
        self.stop = True

    def handle(self, topic: str, payload: bytes) -> None:
        request = json.loads(payload)
        command, id_ = topic.rsplit("/", 1)[-1], request.get("id", "")
        left = self.started_at + float(os.environ.get("FAKE_APP_READY_DELAY", "0")) - time.monotonic()
        if left > 0:  # bridge/live.rs: a command waits until the app has finished starting
            time.sleep(left)

        def ack(error: str | None = None, data=None) -> None:
            body = {"id": id_, "ok": True} if error is None else {"id": id_, "ok": False, "error": error}
            if error is None and data is not None:
                body["data"] = data
            text = json.dumps(body)
            if os.environ.get("FAKE_APP_PERSIST_BUG") == "small-packets" and len(text.encode("utf-8")) > 10 * 1024:
                return  # a bug to catch: rumqttc's default limit of 10 KiB -- the ack is never sent (bridge/live.rs, #200 smoke run)
            self.bridge.publish(f"rs/app/ack/{id_}", text)

        if command == "ping":
            ack()
        elif command == "start-update":
            if self.announced_url is None:
                ack("No update has been announced; check for updates first")
            else:
                ack()
                threading.Thread(target=self.download, args=(self.announced_url,), daemon=True).start()
        elif command == "restart-update":
            error = self.restart()  # a success ends this process
            ack(error or "restart returned without error")
        elif command == "snapshot":
            body = {"id": id_, "ok": True, "data": dict(self.rect)}
            self.bridge.publish(f"rs/app/ack/{id_}", json.dumps(body))
        elif command == "grow-window":
            screen_w, _, screen_h = os.environ.get("FAKE_APP_SCREEN", "1920x1040").partition("x")
            # like grow_to_fit: at least what was asked, but never more than the work area has
            want_w = min(int(request["min_width"]), int(screen_w))
            want_h = min(int(request["min_height"]), int(screen_h))
            old = dict(self.rect)
            if want_w > old["width"] or want_h > old["height"]:
                self.grown_from = self.grown_from or old
                self.rect["width"], self.rect["height"] = max(want_w, old["width"]), max(want_h, old["height"])
                self.bridge.publish(f"rs/app/ack/{id_}", json.dumps({"id": id_, "ok": True, "data": old}))
            else:  # already big enough: nothing changed (data null)
                self.bridge.publish(f"rs/app/ack/{id_}", json.dumps({"id": id_, "ok": True, "data": None}))
        elif command == "replay-chat":
            try:
                self.replay(request["path"])
                ack()
            except (OSError, ValueError, KeyError) as e:
                ack(f"replay failed: {e}")
        elif command == "block-user":
            self.set_blocked(int(request["uid"]), True, request.get("nickname", ""))
            ack()
        elif command == "unblock-user":
            self.set_blocked(int(request["uid"]), False)
            ack()
        elif command == "get-chat-history":
            ack(data=[dict(m) for m in self.history])
        elif command == "sync-dictionary":
            ack(self.sync_dictionary())
        elif command == "save-local-dictionary":
            ack(self.save_local_dictionary(str(request.get("content", ""))))
        elif command == "annotate-furigana":
            texts = request.get("texts")
            if not isinstance(texts, list) or not all(isinstance(t, str) for t in texts):
                ack("annotate-furigana needs a list of strings")
            elif os.environ.get("FAKE_APP_CHAT_BUG") == "ruby-merged":  # a bug to catch: one list for all the lines
                ack(data=[[span for t in texts for span in self.annotate(t)]])
            else:
                ack(data=[self.annotate(t) for t in texts])
        elif command == "get-favorites":
            ack(data=self.favorites)
        elif command == "save-favorites":
            try:
                self.save_favorites(request["favorites"])
                ack()
            except (KeyError, TypeError, ValueError) as e:
                ack(f"save-favorites refused: {e}")
        elif command == "restart-sniffer":
            self.restart_sniffer()
            ack()
        elif command == "clear-history":
            if os.environ.get("FAKE_APP_CHAT_BUG") != "clear-keeps":
                self.history.clear()
            ack()
        elif command in ("open-popup", "hide-popup", "place-popup"):
            label = self.POPUP_NAMES.get(request.get("kind", ""))
            if label is None:
                ack("unknown popup kind")
            elif command == "open-popup":
                self.open_popup(label)
                ack()
            elif label not in self.popups:
                ack(f"{label} does not exist")
            elif command == "hide-popup":
                self.popups[label]["visible"] = False
                ack()
            else:
                self.popups[label]["rect"] = {k: request[k] for k in ("x", "y", "width", "height")}
                ack()
        elif command == "snapshot-popups":
            ack(data=self.popups_snapshot())
        elif command == "pin-main":
            self.main_on_top = bool(request["on"])
            if os.environ.get("FAKE_APP_POPUP_BUG") != "pin-ignored":
                for popup in self.popups.values():
                    popup["always_on_top"] = self.main_on_top
            ack()
        elif command == "start-translator":
            error = self.start_translator()
            ack(error)
        elif command == "download-model":
            ack()
            threading.Thread(target=self.download_model, args=(request,), daemon=True).start()
        elif command == "close-window":
            ack()
            time.sleep(0.2)
            self.leave("close")
        elif command == "quit":
            ack()
            time.sleep(0.2)
            self.leave("quit")
        else:
            ack(f"unknown command {command}")

    # -- the update path (app_updater.rs / gist.rs, as understood)
    def check(self) -> str | None:
        """Returns the announced download url when a newer version is on offer."""
        try:
            verified, why = self.verify_metadata()
        except (OSError, KeyError, ValueError):
            self.log("check failed: metadata")
            return None
        if verified is None:  # refused: no model or dictionary update is offered; the app's own update below is unaffected
            self.refuse(why)
        else:
            self.remember_revision(int(verified["revision"]))
        # the UI's silent dictionary update (hydration.rs): only when auto-sync is on, whatever the metadata announces
        try:
            announced = verified["dictionary"]["version"] if verified is not None else None
        except (ValueError, KeyError, TypeError):
            announced = None
        if announced and announced != self.dict_version and (self.auto_sync_dict or os.environ.get("FAKE_APP_DICT_BUG") == "auto-always"):
            self.sync_dictionary()
        try:
            body = urllib.request.urlopen(self.flags["feed-url"], timeout=5).read()  # noqa: S310
        except urllib.error.HTTPError:
            self.set_update("none")  # the real app reports the finished check, whatever it found
            return None
        except (OSError, KeyError, ValueError) as e:
            self.set_update(f"error:Network error: {e}")
            return None
        try:
            feed = json.loads(body)
            version, url, signature = feed["version"], feed["url"], feed["signature"]
        except (ValueError, KeyError, TypeError):
            self.set_update("error:the update feed is not JSON")
            return None
        if version_tuple(version) > version_tuple(self.version):
            self.set_update(f"available:{version}")
            return url
        self.set_update("none")
        return None

    def download(self, url: str) -> bool:
        temp = self.exe.with_name("update_temp.exe")
        part = self.exe.with_name("update_temp.exe.part")
        self.set_update("downloading")
        try:
            with urllib.request.urlopen(url, timeout=float(os.environ.get("FAKE_APP_STALL_AFTER", "3"))) as r:  # noqa: S310
                data = r.read()
        except (http.client.IncompleteRead, socket.timeout, TimeoutError, OSError) as e:
            part.unlink(missing_ok=True)
            reason = "no data for the stall limit" if isinstance(e, (socket.timeout, TimeoutError)) else str(e)
            self.set_update(f"error:{reason}")
            return False
        if not self.verified(data):
            self.set_update("error:the update is not signed by a trusted key")
            return False
        temp.write_bytes(data)
        self.set_update("downloaded")
        return True

    @staticmethod
    def verified(data: bytes) -> bool:
        if os.environ.get("FAKE_APP_SKIP_VERIFY") == "1":
            return True
        signed = os.environ.get("FAKE_APP_SIGNED_FILE")
        return bool(signed) and Path(signed).read_bytes() == data

    def restart(self) -> str:
        """Installs the downloaded update and restarts (this process ends); returns why it refused."""
        temp = self.exe.with_name("update_temp.exe")
        if not temp.exists():
            return "No verified update has been downloaded"
        if not self.verified(temp.read_bytes()):
            self.log("restart refused: update_temp.exe fails the check")
            return "the update is not signed by a trusted key"
        old = self.exe.with_name(self.exe.name + ".old")
        old.unlink(missing_ok=True)
        self.exe.rename(old)
        temp.rename(self.exe)
        self.exe.chmod(0o755)  # a downloaded file is not executable on POSIX; the real exe is
        args = [a for a in sys.argv[1:] if a not in ("--fresh", "--print-env")]
        subprocess.Popen([str(self.exe), *args], cwd=str(self.exe.parent), close_fds=True, env=os.environ.copy())
        os._exit(0)


def main() -> int:
    flags = parse(sys.argv[1:])
    app = App(flags)
    deadline = time.monotonic() + float(os.environ.get("FAKE_APP_LIFETIME", "40"))
    app.write_status()
    if "print-env" in flags:
        print(json.dumps(app.status))
        return 0
    if os.environ.get("FAKE_APP_TOUCH_REAL") == "1" and os.environ.get("APPDATA"):
        real = Path(os.environ["APPDATA"]) / "com.enjay.bpsr.resonance-stream" / "config.json"
        real.parent.mkdir(parents=True, exist_ok=True)
        real.write_text(json.dumps({"touched": time.time()}), encoding="utf-8")
    data = flags.get("data-dir")
    if data:
        if "fresh" in flags:
            for sub in ("config", "data", "webview"):
                shutil.rmtree(Path(data) / sub, ignore_errors=True)
        for sub in ("config", "data", "webview"):
            (Path(data) / sub).mkdir(parents=True, exist_ok=True)
    app.status["ready"] = True
    app.write_status()
    app.load_window_state()
    app.load_persisted()
    app.connect_bridge()
    if app.use_translation and "llama-url" in flags and "no-translator" not in flags:  # lib.rs: the worker starts with the app
        app.start_translator()
    line = os.environ.get("FAKE_APP_IFACE_LINE")
    if line and "no-capture" not in flags:
        level = "error" if line.startswith("NETWORK_ERROR") else "info"
        app.event("system-event", {"pid": 1, "level": level, "source": "Sniffer", "message": line})
    if "no-popups" not in flags:
        threading.Thread(target=app.prewarm_popups, daemon=True).start()
    if "no-capture" not in flags:
        app.start_sniffer()
    app.announced_url = None if "no-update-check" in flags else app.check()
    while time.monotonic() < deadline and not app.stop:
        if app.bridge is None:
            time.sleep(0.5)
            continue
        try:
            topic, payload = app.bridge.inbox.get(timeout=0.5)
        except queue.Empty:
            continue
        app.handle(topic, payload)
    return 0


if __name__ == "__main__":
    sys.exit(main())

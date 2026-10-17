"""Pipeline `chat-rules`: what the backend does with chat lines, tried with no game.

Lines go in through `replay-chat` (they enter at the same place a captured line does, after the parser), commands come in
over the bridge (`block-user`, `unblock-user`, `get-chat-history`, `clear-history`: the ones the chat row's menu and the clear button
call), and what comes out is read from the events the app publishes. Rows:

  CR-dedupe       a line sent twice (same sender, time and sequence) and the same line from a second client (same sender, text
                  and time, another sequence) are each shown once
  CR-history      the backend's chat log is what was published, in order (what the window reads when it starts)
  CR-block        blocking a sender flags the rows already there (`chat-message-update`) and no other sender's
  CR-block-later  a line from a blocked sender that arrives afterwards is flagged
  CR-unblock      unblocking takes the flag off again
  CR-clear        clearing the history empties the log

And the Study view's furigana (`annotate-furigana`, the app's `annotate_furigana`; six lines, one of them empty):

  CR-ruby-answer   one list of spans per line, in order, and nothing for the empty line
  CR-ruby-join     the spans of a line, joined, are the line (nothing lost, nothing added)
  CR-ruby-plain    a line without kanji is one plain span, no reading
  CR-ruby-reading  a line with kanji has readings, and they are hiragana

Which reading a word gets is not asked here: the dictionary is embedded in the exe, so the core tests pin the readings on any OS
(`crates/core/tests/golden_text.rs`; roadmap K16).

`min_sender_level` is the window's filter, not the backend's (the backend publishes every line), so it is not tested here.
"""
from __future__ import annotations

import json
import re
import time
from pathlib import Path

from runbook import bridge, common, mockfeed, updater
from runbook.common import Recorder

ALICE, BOB = 1001, 2002
PLAIN = ["ありがとう!", "[스티커]よろしく", "hello"]
KANJI = ["日韓辞書", "今日はパーティー募集します"]
RUBY_LINES = KANJI + PLAIN + [""]
HIRAGANA = re.compile(r"^[ぁ-ゖゝゞー・]+$")


def line(nickname: str, uid: int, seq: int, text: str, ts: int, delay_ms: int = 50) -> str:
    return json.dumps({"delay_ms": delay_ms, "channel": "WORLD", "nickname": nickname, "uid": uid, "level": 60,
                       "timestamp": ts, "sequence_id": seq, "text": text}, ensure_ascii=False)


def first_replay(ts: int) -> str:
    return "\n".join([
        line("Alice", ALICE, 1, "A first", ts, 0),
        line("Alice", ALICE, 1, "A first", ts),   # the same line again: the pipeline's duplicate check
        line("Alice", ALICE, 9, "A first", ts),   # a second client's copy: another sequence, same sender / text / time
        line("Bob", BOB, 2, "B first", ts),
    ]) + "\n"


def second_replay(ts: int) -> str:
    return "\n".join([line("Alice", ALICE, 3, "A after block", ts + 1, 0), line("Bob", BOB, 4, "B after", ts + 1)]) + "\n"


class ChatRules:
    def __init__(self, rec: Recorder, exe: str | Path, runs: Path | None = None) -> None:
        self.rec = rec
        self.exe = Path(exe)
        self.runs = Path(runs or common.RUNS) / "chat-rules"
        self._serves: list[bridge.Serve] = []

    def run(self, key_path: str | None = None, password: str | None = None) -> None:
        """(`key_path` and `password` are for the pipelines that sign; this one ignores them.)"""
        with updater.step(self.rec, "CR", self.runs, strays=self.runs, stop=self._stop):
            self.round()

    def _stop(self, folder: str | Path) -> list[int]:
        for serve in self._serves:
            serve.stop()
        self._serves.clear()
        return mockfeed.stop_copies(folder)

    @staticmethod
    def packets(events: bridge.Serve) -> list[dict]:
        return [m["message"]["payload"] for m in events.events() if m["topic"] == "rs/app/event/packet-event"]

    @staticmethod
    def flags_of(rows: list[dict]) -> dict[str, bool]:
        return {row["message"]: row["isBlocked"] for row in rows}

    def round(self) -> None:
        exe = updater.fresh_copy(self.exe, self.runs, "run")
        events = bridge.Serve(exe.parent / "events.jsonl")
        self._serves.append(events)
        args = mockfeed.flag_args(exe.parent / "data", exe.parent / "status.json", log_file=exe.parent / "app.log",
                                  extra=("--no-update-check", "--bridge-url", events.url))
        mockfeed.start_app(exe, args)
        if not bridge.wait_started(events, exe.parent / "app.log"):
            self.rec.auto("CR-start", "the app started and connected to the bridge", False, "no app-started event in 120 s")
            return
        ts = int(time.time())
        first = exe.parent / "first.jsonl"
        first.write_text(first_replay(ts), encoding="utf-8")
        events.send("replay-chat", {"path": str(first)})
        try:
            events.expect("system-event", {"payload.message": "Replay finished"}, timeout=30)
        except RuntimeError:
            self.rec.auto("CR-dedupe", "a repeated line is shown once", False, "the replay never finished")
            return

        shown = [p["message"] for p in self.packets(events)]
        self.rec.auto("CR-dedupe", "a line sent twice, and the same line from a second client, are each shown once",
                      shown == ["A first", "B first"], f"published: {shown} (wanted ['A first', 'B first'])")
        history = events.send("get-chat-history")["data"]
        published = [(p["pid"], p["message"]) for p in self.packets(events)]
        self.rec.auto("CR-history", "the backend's chat log is what was published, in order",
                      [(m["pid"], m["message"]) for m in history] == published,
                      f"log {[(m['pid'], m['message']) for m in history]} / published {published}")

        events.send("block-user", {"uid": ALICE, "nickname": "Alice"})
        flagged = self.wait_update(events, ALICE, True)
        history = events.send("get-chat-history")["data"]
        self.rec.auto("CR-block", "blocking a sender flags the rows already there, and only theirs",
                      flagged and self.flags_of(history) == {"A first": True, "B first": False},
                      f"update seen: {flagged}; flags {self.flags_of(history)}")

        second = exe.parent / "second.jsonl"
        second.write_text(second_replay(ts), encoding="utf-8")
        events.send("replay-chat", {"path": str(second)})
        try:
            events.expect("packet-event", {"payload.message": "B after"}, timeout=30)
        except RuntimeError:
            self.rec.auto("CR-block-later", "a later line from a blocked sender is flagged", False, "the second replay never arrived")
            return
        later = {p["message"]: p["isBlocked"] for p in self.packets(events)}
        self.rec.auto("CR-block-later", "a later line from a blocked sender is flagged, another sender's is not",
                      later.get("A after block") is True and later.get("B after") is False, f"flags {later}")

        events.send("unblock-user", {"uid": ALICE})
        cleared = self.wait_update(events, ALICE, False)
        history = events.send("get-chat-history")["data"]
        self.rec.auto("CR-unblock", "unblocking takes the flag off the sender's rows",
                      cleared and not any(m["isBlocked"] for m in history),
                      f"update seen: {cleared}; flags {self.flags_of(history)}")

        events.send("clear-history")
        left = events.send("get-chat-history")["data"]
        self.rec.auto("CR-clear", "clearing the history empties the backend's log", left == [], f"{len(left)} rows left")
        self.ruby(events)
        try:
            events.send("quit", timeout=15)
        except RuntimeError:
            pass

    def ruby(self, events: bridge.Serve) -> None:
        """`annotate-furigana` on lines with and without kanji."""
        try:
            answer = events.send("annotate-furigana", {"texts": RUBY_LINES})["data"]
        except RuntimeError as e:
            for check in ("CR-ruby-answer", "CR-ruby-join", "CR-ruby-plain", "CR-ruby-reading"):
                self.rec.auto(check, "annotate-furigana answers", False, str(e))
            return
        shaped = isinstance(answer, list) and len(answer) == len(RUBY_LINES) and all(isinstance(spans, list) for spans in answer)
        self.rec.auto("CR-ruby-answer", "one list of spans per line, in order, and nothing for the empty line",
                      shaped and answer[RUBY_LINES.index("")] == [], f"{len(RUBY_LINES)} lines asked, answer: {answer}"[:400])
        if not shaped:
            for check in ("CR-ruby-join", "CR-ruby-plain", "CR-ruby-reading"):
                self.rec.auto(check, "the furigana can be judged", False, "the answer was not one list per line")
            return
        by_line = dict(zip(RUBY_LINES, answer))
        joined = {line: "".join(span.get("text", "") for span in spans) for line, spans in by_line.items()}
        lost = {line: got for line, got in joined.items() if got != line}
        self.rec.auto("CR-ruby-join", "the spans of a line, joined, are the line", not lost, f"changed: {lost}" if lost else f"{len(by_line)} lines intact")
        plain = {line: by_line[line] for line in PLAIN if by_line[line] != [{"text": line}]}
        self.rec.auto("CR-ruby-plain", "a line without kanji is one plain span, no reading", not plain, f"not plain: {plain}" if plain else f"{PLAIN}")
        bad = {line: [s for s in by_line[line] if "reading" in s and not HIRAGANA.match(str(s["reading"]))] for line in KANJI}
        bad = {line: spans for line, spans in bad.items() if spans}
        bare = [line for line in KANJI if not any(s.get("reading") for s in by_line[line])]
        self.rec.auto("CR-ruby-reading", "a line with kanji has readings, and they are hiragana", not bad and not bare,
                      f"not hiragana: {bad}; no reading at all: {bare}" if bad or bare else f"{ {line: by_line[line] for line in KANJI} }"[:300])

    @staticmethod
    def wait_update(events: bridge.Serve, uid: int, blocked: bool) -> bool:
        try:
            events.expect("chat-message-update", {"payload.uid": uid, "payload.isBlocked": blocked}, timeout=10)
            return True
        except RuntimeError:
            return False

# The server's verdict on each send: design options

**Status:** options for Kade, nothing decided. Task R-M2 (lane chat-send, milestone M2,
star-resonance/tasks#46). Written 2026-11-17. No code was written, and nothing was built or run in
the game for this document.

## How to read this

The form is that of [ingame-design.md](ingame-design.md), the R0 doc. Each of sections 1-5 lists
**facts** first, then **options** (A/B/C), then one line, `Recommended: <letter>, <reason>`. Then
come the tests the work would need, the tasks it would split into, **Questions for Kade** (at most
4, each answered by picking a letter), a summary table, and **Not verified**: the claims that only a
run in the game can settle.

Evidence labels follow `docs/security_model.md` (line 6), as in R0: **confirmed** means the code was
opened and the `file:line` says it; **reported** means a source says it but nobody here checked it
by hand (for example a comment in Stellar describing the game); **estimated** means reasoning, not
reading; **derived** means two confirmed or reported facts put together, not observed. `NOT VERIFIED`
marks a claim about the game or the server at run time, which no agent can check.

Citations into this repository are `path:line` on `main` at `4cf0d20`
(`4cf0d206986ae261ad27e383634c214f44667576`). Citations into the Stellar framework are
`Stellar@60a7772:<path>:<line>`, where `<path>` is relative to the
StellarProtocol/StellarResonanceModSystem repository at commit
`60a7772c8de86ec62016703ede89079955f63166`, and `…/` stands for `src/Stellar.Infrastructure/`.
Stellar is AGPL: this doc takes **facts** from it (names, numbers, wire layouts) and copies no code
or comment text (`.claude/rules/ingame.md`, "No Stellar code copied").

## What M2 is

The roadmap (star-resonance/tasks#2) gives M2 as: hook `ZRpcImpl.SendBytes`, correlate `call_id`
with the server's Return, and show "sent / too frequent / cooldown" in the composer. DS-0009 item 1C
(tasks#1) asked for this doc before any of it is planned. The roadmap's wording names one way to do
it (a game hook); section 1 shows a second way that needs no new hook, and asks Kade to choose.

## Fixed decisions (not options)

These were decided earlier (DS-0001, D-35 in [decisions.md](decisions.md), the lane's shared
values). This doc builds on them and reopens none.

- **The send.** The app loads its own Rust DLL, `resonance_ingame.dll`, into the game only when the
  user has opted in, and the DLL calls the game's `ZTcpClient.Send` with a frame it builds itself
  (D-35, `docs/decisions.md:48`). No Stellar plugin, no C# (DS-0001).
- **The frame.** `[size u32 BE][flags u16 BE][service_uuid u64 BE][stub_id u32 BE][call_id u32 BE][method_id u32 BE][payload]`,
  ChitChat service `1321197368`, `SendChitChatMsg` method `1` (`crates/frame/src/lib.rs:114-152`).
- **Channels.** `say`/`world`/`party`/`guild`, wire `channel_type` 2/1/3/4 (R0, Fixed decisions;
  `crates/frame`).
- **Text** at most 140 UTF-16 units; the app's own spacing **1500 ms** between successful sends
  (`SEND_SPACING`, `crates/core/src/send_text.rs:8`).
- **The pipe.** The app's whole send exchange waits `APP_WAIT_MS` = 1500 ms
  (`crates/types/src/ingame_pipe.rs:22`). Errors reach the ui as one Korean sentence.
- **Scope (D-35).** The DLL only sends chat; "No other hook" (`.claude/rules/ingame.md`, Scope).
  Any new game hook is a scope change for Kade.

## What exists today (the facts every section rests on)

**The DLL hooks one game method, a per-frame pump, and nothing else.** (confirmed)

- `HOOK_CANDIDATES` is `ZRpcCtrl.Update(1)`, then `Panda.Core.Game.Update(1)`
  (`crates/ingame/src/names.rs:81-95`). `ZRpcImpl` is in `names.rs` only as a class name the walk
  uses (`ZRPC_IMPL`, `names.rs:36`); no method of it is hooked, `SendBytes` included.
- The hook is an `ilhook` `JmpBack` detour to `pump_detour` (`crates/ingame/src/win/hook.rs:113-119`,
  `crates/ingame/src/win/pump.rs:52-68`), installed while the game's other threads are suspended
  (`hook.rs:101-122`; the no-allocation rule in `.claude/rules/ingame.md`).

**Threads.** (confirmed)

- The detour runs on whichever game thread calls the hooked method, and acts only when
  `GetCurrentThreadId() == MAIN_TID` (`win/pump.rs:23`, `:59-62`; `MAIN_TID` is stored at
  `win/hook.rs:111-112`).
- `pump_once` takes the job, walks to the live `ZTcpClient` again, calls `Send` with one span, and
  completes the slot (`crates/ingame/src/pump.rs:32-63`, `crates/ingame/src/send_ops.rs:56-83`).
- `Send`'s return value is ignored: "what it means is not known" (`send_ops.rs:77`, in our words).
- The pipe worker is its own thread (`crates/ingame/src/win/entry.rs:55`, `worker` at `:225`,
  `serve` at `:278`). It blocks in `SlotQueue::submit` (`win/pump.rs:73-79`) until the pump answers:
  at most `DLL_HOLD_MS` = 1000 ms to be taken (`ingame_pipe.rs:20`), then a 5000 ms cap once taken
  (`TAKEN_WAIT_CAP_MS`, `crates/ingame/src/slot.rs:16`, `:128-161`).

**`call_id` today: the DLL mints it, and the app throws it away.** (confirmed)

- `CallIds` starts at a random seed in the upper half of `u32` and counts up, wrapping inside the
  upper half (`crates/ingame/src/send.rs:64-102`; test `call_ids_stay_in_the_upper_half`,
  `send.rs:280`). The frame carries it as given (`crates/frame/src/lib.rs:152`).
- The DLL answers a send with `Reply::Send { id, call_id }` (`send.rs:131`,
  `crates/types/src/ingame_pipe.rs:154`).
- The app drops it: `Reply::Send { .. } | Reply::Ack { .. } => (Ok(()), false)`
  (`crates/core/src/ingame_send.rs:468`). So the app can already learn each send's `call_id`, and
  does not use it.
- `Reply::Send` means "handed to `Send`", not "the server accepted": the DLL logs `sent` for it
  (`crates/ingame/src/session.rs:112-123`, `:121`).

**The app side.** (confirmed)

- `send_chat_flow` returns `Result<(), String>` after one exchange of at most `APP_WAIT_MS`, and
  records the spacing only on success (`crates/core/src/ingame_send.rs:494-522`; `record_success` at
  `:510`, defined at `crates/core/src/send_text.rs:39`).
- `ErrorCode` has 10 codes (`ingame_pipe.rs:32-43`), each with one Korean sentence
  (`ingame_send.rs:159-180`); `NotSent` is "게임이 메시지를 보내지 못했습니다." (`:170`).
- The composer empties `draft` on `Ok(())` and sets one `error` string on `Err`
  (`src/components/chat_composer.rs:76-82`). There is no state between "sent" and a verdict that
  comes later, and no event carries one.
- Backend-to-ui events are constants in `resonance_types::ipc::event` with a place in
  `event::ALL` (`crates/types/src/ipc.rs:129-163`, 14 today); `.claude/rules/app.md` ("Events are
  emitted with ...") is the rule for adding one. `SendChat` returns `()` (`ipc.rs:74`).

**Versions.** (confirmed)

- `PIPE_PROTOCOL_VERSION = 1` (`ingame_pipe.rs:12`). The app sends `hello { version }`
  (`src-tauri/src/ingame/win/host.rs:95-106`); the DLL answers `error version` when the numbers
  differ, with no negotiation (`crates/ingame/src/session.rs:87-92`).
- The app then says `bye`, waits for the old module to leave and loads its own; a module that will
  not leave is noted as resident, and every later send gets `ResidentModule`, "게임에 전송 모듈이
  남아 있어 다시 불러올 수 없습니다. 게임을 다시 시작해 주세요." (`ingame_send.rs:418-458`,
  `:235-237`).
- The DLL is a per-version download pinned by SHA-256 (DS-0006; `.claude/rules/app.md`, "Only the
  pinned DLL"), so an old DLL meets a new app only in a game that kept the old module loaded.

**What the sniffer reads.** (confirmed)

- Only server-to-client TCP on port 5003 with a payload: `is_capturable`
  (`crates/core/src/capture/recorder.rs:21-29`) and the live pipeline (`tcp.source_port != 5003`,
  `crates/core/src/capture/pipeline.rs:93`). The frames the DLL sends are never seen; the server's
  answers are.
- The frame assembler knows `0x0002` (16-byte inner header), `0x0003` (12-byte inner header),
  `0x0004` keepalive, and the `0x8000` compressed forms (`crates/core/src/protocol/framing.rs:6-12`,
  `:29-33`, `inner_header` at `:188-193`). A plain `0x0003` is kept only when its root has field 5
  as bytes (one-line history); anything else is dropped as "the player's own line, echoed"
  (`framing.rs:235-238`, `has_history_lines` at `:244-249`), pinned by
  `a_plain_0x0003_echo_stays_skipped` (`:372-377`, whose root is `{3: …, 4: …}`).
- Nothing in `crates/core/src/protocol/` names ChitChat, `service_uuid`, `call_id` or 5211 (`grep`,
  no match).
- Per-connection identity: `StreamKey` is server address and port, then client address and port
  (`crates/core/src/capture/stream_tracker.rs:5-8`); the roster maps a connection's local end to a
  character (F-MC-2, `crates/core/src/game_clients.rs:40-107`); `chat_connections()` maps a pid to
  its local endpoints on the chat port (`src-tauri/src/ingame/win/tcp.rs:83-86`).
- Inbound chat lines carry the server's time in Unix seconds (`ChatPayload.timestamp`, field 3,
  `crates/core/src/protocol/parser.rs:22`; `ChatMessage.timestamp`, `crates/types/src/lib.rs:134`).

## What the server answers (Stellar, reported)

- **The Return frame has no service and no method.** Its header is `[size u32][flags u16][stub_id
  u32][call_id u32][error_id u32]`, 18 bytes, then the payload; the message type is the low 15 bits
  of `flags`: 1 Call, 2 Notify, 3 Return (`Stellar@60a7772:…/Game/PandaWireTap.Parsing.cs:15-22`,
  `:103-113`). A Notify's header is `service_uuid u64` + `stub_id` + `method_id` after the prefix
  (`:115-127`).
- **Derived:** the sniffer's `0x0002` (16 inner bytes = 8 + 4 + 4) is a Notify, and its `0x0003`
  (12 inner bytes = 4 + 4 + 4) is a **Return**: `stub_id`, `call_id`, `error_id`. The plain
  `0x0003` that `framing.rs` drops as an "echo" is a Return the app does not read; its root
  `{1: {3: …, 4: …}}` is exactly the shape of an accepted `SendChitChatMsgReply` (below; an
  `err_code` of 0 is not written on the wire, protobuf's default). Not observed here: NOT VERIFIED.
- **The reply.** An envelope `{1: reply}` around `SendChitChatMsgReply { 3: show_msg (a chat
  message), 4: cd_end_time varint, 5: err_code varint }`; `err_code` 0 is accepted, **5211** is
  `ErrChatSendCdNoEnd` ("too frequent"); `cd_end_time` and the message's timestamp are Unix
  seconds, 0 is absent; the game itself counts its cooldown as `cd_end_time` minus the message's
  timestamp (`Stellar@60a7772:src/Stellar.Wire/WireProtocol.ChitChat.Send.cs:10-22`,
  `src/Stellar.Wire/WireProtocol.cs:41-52`, `src/Stellar.Abstractions/Domain/ChatSendResult.cs:19-20`).
- **Cooldowns.** World and Local (`say`) reply with a cooldown end (observed 5 s); Party and Guild
  reply with none (`ChatSendResult.cs:5-10`, reported by Stellar, not seen here). A refused reply
  typically carries no `show_msg`, so no server timestamp (`ChatSendResult.cs:32-34`).
- **Codes.** No code other than 0 and 5211 is named; whether 5211 is the only refusal is NOT
  VERIFIED. `-1` is Stellar's own "never left the client" code, not a server one
  (`ChatSendResult.cs:22-26`).
- **Matching.** Stellar keeps `call_id -> channel` for each send (its own, recorded before the call;
  the game's own, read from a `ZRpcImpl.SendBytes` prefix), at most 64 pending, oldest evicted, and
  an unmatched Return falls through to its history parser
  (`…/Game/PandaChatProbe.SendResult.cs:8-14`, `:18-38`, `:55-81`; `MaxPendingChatCalls` 64 at
  `…/Game/PandaChatProbe.Send.cs:37`, recorded before dispatch at `:236`;
  `…/Game/PandaChatProbe.Receive.cs:150-172`).
- **Our sends bypass the game's own cooldown state.** A send the framework builds itself never
  reaches the game's `chatCD_`, so the Return is the only cooldown source for it
  (`…/Game/PandaChatProbe.SendResult.cs:14`). The same holds for our DLL, which builds its own frame
  (derived).
- No wire fact says what the game does with the text after 5211, or what a timeout looks like: both
  NOT VERIFIED (Kade's check, "The tests the work would need").

## Sections

1. Where the verdict is read
2. Matching a send to its Return
3. The verdict codes and the Korean composer states
4. The pipe protocol change and an old DLL
5. Several clients

Then: the tests the work would need, the tasks it would split into, Questions for Kade, summary,
Not verified.

## 1. Where the verdict is read

### Facts

- **`SendBytes` sees only the outbound call.** `ZRpcImpl.SendBytes(uuid, stubId, callId, methodId,
  pbData, pbSize, framePack)` is the game's own path to the transport (R0 section 1;
  `docs/ingame-design.md:125` cites Stellar for it). A hook there gives the `call_id` and channel of
  sends **the DLL did not make**: the game's own chat-box sends. The DLL's own sends never pass
  through it (they go straight to `ZTcpClient.Send`), and their `call_id` is already known
  (`Reply::Send`). (confirmed for our side, reported for the game's)
- **The Return arrives on the receive side.** Stellar reads Returns with one hook on
  `ZCode.ZNet.ZTcpConnection.OnData(ReadOnlySpan<byte>, ZTcpClient)`, as a prefix, because the
  method clears its buffer before it returns
  (`Stellar@60a7772:…/Game/PandaWireTap.Patching.cs:58-95`, `:90-93`), and a second one on
  `ZCode.ZNet.ZUdpConnection.ProcessKcpData(ReadOnlySpan<byte>)` for UDP/KCP (`:151-167`). Its
  handlers run on the game's network I/O thread, which carries hundreds of packets a second
  (`…/Game/PandaWireTap.cs:9-17`, `:33-35`; `…/Game/PandaChatProbe.History.cs:23`). Reported.
- **Login, chat and social traffic go over TCP; scene and combat over UDP/KCP**
  (`…/Game/PandaWireTap.Patching.cs:34-37`, reported). R0 noted that World can run over UDP/KCP
  (`docs/ingame-design.md:116`, `:708-713`). Our DLL sends only over TCP: a UDP connection is
  refused with `no_client` before anything is sent (R0 section 7). So **every send the DLL makes is
  answered on a TCP connection to port 5003**, which the sniffer already reads (derived).
- **The sniffer already sees every Return on that connection**, frames it (`0x0003`, 12-byte inner
  header) and drops the non-history ones (`framing.rs:231-238`). Reading the `call_id` and the reply
  needs a decoder in `crates/core`, not a new capture (derived from the facts above).
- **Cost of a hook in the DLL** (R0 sections 2, 3, 6): each is one more `ilhook` detour with the
  game's other threads suspended around the 14-byte write, one more method to resolve by name after
  every patch, one more unhook that must be provably safe before unload (`KeepResident`
  otherwise, `crates/ingame/src/win/entry.rs:232-235`), and, on `OnData`, a hot path where the rule
  "one atomic load when idle, never allocate, lock or log" (`.claude/rules/ingame.md`) would have to
  hold for every inbound packet, not once a frame.

### Options

- **A. In the DLL: a receive hook (and a `SendBytes` hook for the game's own sends).** Detour
  `ZTcpConnection.OnData`, parse each frame header on the network thread, pick the Returns whose
  `call_id` is one of ours, and report the verdict over the pipe. A `SendBytes` detour adds the
  game's own chat-box sends (their channel and `call_id`), which our composer does not need. Two new
  hooks on a hot path, a D-35 scope change ("no other hook"), a pipe protocol change (section 4),
  and new code in the game's process that reads every packet. Patch resilience: two more names to
  resolve (R0 section 7's table gains two rows), and a buffer layout to trust at run time.
- **B. In the app: decode the Return from the sniffed TCP stream.** `FrameAssembler` hands a plain
  `0x0003` that is not history to a new pure decoder in `crates/core` (header: `call_id`; root:
  the `SendChitChatMsgReply` fields, through `decoder.rs`'s `Fields`), and the app matches it with
  the `call_id` the DLL already returns (`Reply::Send`). No new game hook, no pipe change, nothing
  new in the game's process, D-35 unchanged. It needs the sniffer running (it is, whenever chat is
  shown) and sees only TCP, which is all the DLL sends over. A segment the raw socket misses loses
  that Return: section 2's "no verdict" state covers it.
- **C. Both: the app as the verdict, the DLL's hooks as a cross-check or for the game's own sends.**
  All of B's code and all of A's hooks; it adds something only if the composer must also show
  verdicts for lines typed in the game's own chat box, which the game already shows itself
  (`ChatSendResult.cs:28-30`, reported).

| | new game hooks | pipe change | D-35 scope | runs in the game | sees a missed segment |
|---|---|---|---|---|---|
| A | 1-2 (`OnData`, `SendBytes`) | yes | widened | packet parser on the I/O thread | yes (in-process) |
| B | 0 | none | unchanged | nothing new | no: "no verdict" |
| C | 1-2 | yes | widened | as A | yes |

What `crates/core` needs for B (and for A's parser, which would live in `crates/frame` or core
too): a `Return` frame kind in `framing.rs` (the plain `0x0003` that is not history, today `None` at
`:238`), a header reader (`stub_id`, `call_id`, `error_id`), and a `SendChitChatMsgReply` reader
using `decoder.rs`'s `Fields` and `read_varint` (`crates/core/src/protocol/decoder.rs`). All pure,
all tested on any OS under `just core-check`.

Effect on the patch-resilience table (R0 section 7): B adds no row; it depends on the wire layout,
which the sniffer already depends on, and not on any game method name. A adds two rows
(`OnData` or `SendBytes` renamed or re-signed: `unsupported` for the verdict only, the send still
works).

`Recommended: B`, because the app already captures every Return the DLL's sends can produce, so the
verdict needs no new hook, no D-35 change and no code on the game's network thread; reading the
reply off the wire is the long-term option as well, since a game patch that renames methods does
not touch it. The roadmap's "hook `ZRpcImpl.SendBytes`" (tasks#2) was written before this was known:
`SendBytes` would not see the DLL's own sends at all.

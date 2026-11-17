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
  (`docs/ingame-design.md:121-123`, `:710-713`). Our DLL sends only over TCP: a UDP connection is
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

## 2. Matching a send to its Return

### Facts

- **The key is `call_id`.** A Return carries no service and no method, so `call_id` is the only key
  (Stellar's layout, section "What the server answers"). The DLL mints ours in the upper half of
  `u32` from a random seed (`crates/ingame/src/send.rs:64-102`), so the game's own ids, which the
  DLL's comment says live in the lower half (`send.rs:64-65`), do not collide with them (reported
  by that comment; the game's ids were never observed: NOT VERIFIED).
- **Which bytes hold it.** In Stellar's layout the `call_id` is bytes 4-7 of the 12-byte inner header
  of a `0x0003` frame (frame bytes 10-13), after `stub_id` and before `error_id` (reported). The
  repository's only fixture cannot confirm it: `crates/core/tests/fixtures/oct03.sanitised.capture.log`
  holds 33 plain `0x0003` frames (counted by reassembling its streams by TCP sequence), but the
  sanitiser blanks every inner header and the body of every frame the app does not read
  (`crates/core/src/capture/sanitise.rs:17-18`), so their `call_id`s and roots are all zeros. Whether
  the player sent chat in that capture cannot be told. NOT VERIFIED until a raw capture is read.
- **Timing.** The DLL answers `Reply::Send` after `Send` returns on the game's main thread
  (`pump.rs:44-56`); the server's Return comes one round trip after the bytes leave. The two race:
  the sniffer can frame the Return **before** the app has read `Reply::Send` from the pipe (derived;
  both arrive within milliseconds of each other, estimated). A matcher that drops a Return it does
  not yet expect loses the verdict.
- **Stellar's map** is `call_id -> channel`, at most 64 pending, oldest evicted, recorded before the
  call so that an instant reply finds it (`…/Game/PandaChatProbe.SendResult.cs:18-38`;
  `…/Game/PandaChatProbe.Send.cs:236`). It has no timeout: an unanswered entry leaves only by
  eviction (reported).
- **The spacing today ends at "handed over".** `send_chat_flow` calls `record_success` right after
  the pipe answers (`ingame_send.rs:509-510`), with `SEND_SPACING` = 1500 ms
  (`send_text.rs:8`). After a 5211 that window is wrong in both directions: it was started by a
  send the server refused, and it is shorter than the server's cooldown (5 s observed on World and
  Local, reported).
- **The draft is cleared on `Ok`** (`chat_composer.rs:77-79`): if the command returns before the
  verdict, a refused line is already gone from the input.

### What happens when no Return comes

A Return can be missing for reasons the app cannot tell apart: the raw socket missed the segment, the
connection dropped and reconnected (the Return died with it), or the server answered nothing (no
case known: NOT VERIFIED). UDP is not one of them: the DLL never sends over UDP (section 1). In every
case the line was handed to the game's socket, so the honest state is "sent, not confirmed", not an
error.

### Options

- **A. Key `(connection, call_id)`; a pending send expires after a timeout and the expiry is an
  error.** The timeout is `VERDICT_WAIT` = 1000 ms (estimated: a round trip to the game server is
  tens to a few hundred milliseconds; the wait starts when `Reply::Send` is read, so a send takes at
  most `APP_WAIT_MS` + 1000 ms = 2.5 s in the worst case and one round trip more than today in the
  usual one, and it stays well under the observed 5 s cooldown). Simple,
  but it calls a line the server probably received "failed", and the user sends it again.
- **B. Match by order (FIFO) per connection, no `call_id`.** Take the next Return on the connection
  after each send. Wrong whenever the game has its own call in flight (history fetches, social
  calls: every RPC on that connection returns as `0x0003`), which is often. Not recommended.
- **C. Key `(connection, call_id)`, an early-Return buffer, and a neutral "no verdict" state after
  the timeout.** The matcher holds:
  - `pending`: the sends the app is waiting for, each with its channel and the time it was handed
    over; at most 64 (Stellar's bound), oldest evicted; each leaves on its Return or after
    `VERDICT_WAIT`.
  - `early`: Returns whose `call_id` is in our upper half but not (yet) pending, kept for
    `VERDICT_WAIT`, at most 64, oldest evicted; a send registered within that time takes its
    Return from here. Returns in the lower half are the game's own and are not kept.
  - Each send ends in exactly one state: accepted, refused (with its code and cooldown), or "no
    verdict" after `VERDICT_WAIT`. "No verdict" is not an error: the composer clears the draft and
    shows a neutral line (section 3). A Return that comes after its send expired updates only the
    cooldown (section 3) and is logged at debug level.

  Memory: two maps of at most 64 entries of a few dozen bytes each; nothing grows with traffic.

**The spacing after a verdict.** In every option the 1500 ms window stays as the floor it is today
(a refused send also started it, which costs nothing). A refusal with a cooldown adds a second,
per-channel gate that ends at the server's `cd_end_time` (section 3). The app's spacing is never
taken as the server's cooldown.

`Recommended: C`, because only `call_id` names a Return, the early buffer closes the pipe-versus-wire
race, and a missing Return is far more likely a missed segment than a lost line, so "no verdict"
must not read as a failure; it is A with the race and the wording fixed, and the standard pattern for
correlating asynchronous RPC replies (a bounded pending table with a deadline).

## 3. The verdict codes and the Korean composer states

### Facts

- **The wire gives three numbers:** the Return header's `error_id` (an RPC-level error, reported:
  `Stellar@60a7772:…/Game/PandaWireTap.Parsing.cs:103-111`; what values it takes is NOT VERIFIED),
  the reply's `err_code` (0 accepted, 5211 too frequent, anything else unknown), and `cd_end_time`
  (Unix seconds, 0 absent), plus `show_msg.timestamp` on an accepted reply (section "What the server
  answers").
- **The clock.** The game counts a cooldown as `cd_end_time` minus the reply's own message
  timestamp, both server seconds (`WireProtocol.ChitChat.Send.cs:10-14`, reported). A refused reply
  usually has no `show_msg` and so no timestamp (`ChatSendResult.cs:32-34`). The PC's clock can be
  minutes off the server's, so `cd_end_time` minus the PC's time alone can show a wrong number or a
  negative one. The sniffer sees the server's time on every inbound chat line
  (`parser.rs:22`, Unix seconds) and on every accepted reply.
- **Party and Guild carry no cooldown** (reported, `ChatSendResult.cs:5-10`): only the app's 1500 ms
  spacing applies to them.
- **Today the composer has two states**: sending (`sending`), then either the draft cleared or one
  error sentence (`chat_composer.rs:73-83`).

### The states (placeholder Korean words, marked: the words are Kade's)

The state is computed by one pure function in `crates/core`, `verdict(reply, server_now) -> Verdict`,
with no clock inside: `server_now` is passed in (below). The ui picks the sentence by the state,
never by a number, and shows no digits except the remaining seconds.

| wire fact | state | sentence shown (ko, placeholder) | draft |
|---|---|---|---|
| `err_code` 0, `error_id` 0 (any channel) | accepted | none (as today) | cleared |
| `err_code` 5211, `cd_end_time` present and `server_now` known | cooldown, `n` s left | 아직 보낼 수 없습니다. `n`초 뒤에 다시 보내 주세요. | kept |
| `err_code` 5211, no `cd_end_time` or no `server_now` | too frequent | 너무 자주 보냈습니다. 잠시 후 다시 보내 주세요. | kept |
| any other `err_code`, or `error_id` not 0 | refused, unknown | 서버가 메시지를 받지 않았습니다. | kept |
| no Return within `VERDICT_WAIT` (section 2) | no verdict | 보냈지만 서버의 확인을 받지 못했습니다. | cleared |
| an existing `ErrorCode` or app error | not sent | the existing sentence (`ingame_send.rs:159-180`, `:209-243`) | kept |

- `n` is whole seconds rounded up: `cd_end_time - server_now`, at least 1; a value above 600 s is
  taken as a wrong clock and shown as "too frequent" (600 is a placeholder).
- An unknown code is logged as `log::debug!("send refused: code {err_code} error_id {error_id}")`,
  with no chat text, and never shown.
- `server_now` is, in order: the reply's own `show_msg.timestamp` (an accepted reply); else the
  PC's time when the Return was framed plus the offset `server seconds - PC seconds` taken from the
  last inbound chat line or accepted reply on that connection; else unknown (so "too frequent",
  never a guess from the PC's clock alone). After the verdict, the count runs on the PC's monotonic
  clock (`Instant`) from the moment the Return was framed, so a PC clock change cannot move it.

### Options: how the composer learns the verdict

- **A. The command waits for it.** `send_chat` waits up to `VERDICT_WAIT` after `Reply::Send` and
  returns the state; `SendChat`'s result changes from `()` to a small `SendVerdict` type in
  `crates/types` (an ipc change, both ends in one PR). The draft is cleared only on "accepted" or
  "no verdict", so a refused line is never lost. No event. The send takes one round trip longer
  before the input clears (tens to hundreds of ms, estimated).
- **B. The command returns at "handed over"; the verdict comes as a new event.** A `send-verdict`
  event in `ipc::event` (`ALL` grows to 15), and the composer keeps a copy of the last line to put
  back on a late refusal, which may overwrite what the user typed since.
- **C. A, plus the event for a Return that comes after `VERDICT_WAIT`.** Covers a slow server, at
  the cost of both paths.

`Recommended: A`, because the draft stays in the input until the server has answered, which is how
a chat box normally behaves, and it needs no new event or second code path; a late Return still
updates the cooldown (section 2 C).

### Options: how a cooldown is shown

- **A. A static sentence** with the seconds left at the moment of the verdict. Nothing else changes.
- **B. A countdown** in the composer's sentence, once a second down to zero, then the sentence goes
  away; computed from the stored end on the monotonic clock (above).
- **C. B, plus the send button disabled for that client and channel until the cooldown ends, and the
  backend refusing a send there before it leaves** (a new `SendError::Cooldown` with the seconds
  left, the same sentence as the table). An **accepted** World or Local reply's `cd_end_time` starts
  the same gate, so the app never sends a line the server will refuse. Party and Guild, with no
  cooldown, keep only the 1500 ms spacing.

`Recommended: C`, because the server tells the app the exact end of each channel's cooldown, and
not sending a line the server is certain to refuse is better than showing a refusal (it is what the
game's own chat box does with its `chatCD_`, reported); it is the most work of the three (a
per-client, per-channel map in `crates/core` beside the spacing, and a timer in the composer).

## 4. The pipe protocol change and an old DLL

### Facts

- `PIPE_PROTOCOL_VERSION` is 1 (`ingame_pipe.rs:12`); lines are JSON, at most `MAX_LINE_BYTES` =
  4096 bytes (`:14`); the app sends requests and the DLL answers each with exactly one reply that
  echoes its `id` (`ingame_pipe.rs:1-5`). An unsolicited line from the DLL would break that rule: the
  app reads one reply per request and treats anything else as a broken session
  (`map_send_reply`, `ingame_send.rs:466-476`: a `Hello` to a send drops the session).
- A version mismatch is refused, not negotiated: the DLL answers `error version`
  (`session.rs:87-92`), the app says `bye` and loads its own copy, and a module that will not leave
  makes every send fail with `ResidentModule` until the game restarts (`ingame_send.rs:418-458`).
- The DLL is downloaded per app version and pinned by SHA-256 (DS-0006, `.claude/rules/app.md`), so a
  new app always brings the matching DLL; only a game that kept an old module resident runs a
  mismatched one.
- The `call_id` the verdict needs is **already** on the wire: `{"id":2,"ok":true,"call_id":17}`
  (`Reply::Send`, `ingame_pipe.rs:154-157`, `:248`).

### Options

- **A. Bump to 2: the DLL gains a message.** Needed only with section 1 A or C: the DLL pushes a
  `{"verdict":…,"call_id":…,"err_code":…,"cd_end_time":…}` line, or the app polls with a new
  `{"op":"verdict","id":N,"call_id":…}` request (a push breaks one-request-one-reply, so a poll keeps
  the rule; the push needs a reader thread that sorts replies from pushes). New `ErrorCode`s: none
  for the verdict itself (it is not an error of the pipe). An old resident DLL then meets
  `version`, and the user must restart the game once after the update (the existing
  `ResidentModule` sentence). Tests in `crates/types`: the new line round-trips, stays under
  4096 bytes, and an old `hello` gets `version`.
- **B. No pipe change.** The verdict is read in the app (section 1 B); `PIPE_PROTOCOL_VERSION` stays
  1, every DLL already released works unchanged (M2 needs only the `call_id` it already returns),
  and no game restart is ever needed for M2. Tests: none in `crates/types` beyond the existing
  `Reply::Send` round trip (`ingame_pipe.rs:460`).
- **C. Additive: `hello` reports a `features` list.** `{"id":1,"ok":true,"version":1,"dll":"…",
  "features":["verdict"]}`; an old DLL sends none and the app falls back to "sent, no verdict". It
  avoids a bump for later additions, at the cost of a second version axis to test.

`Recommended: B`, because with the verdict read in the app nothing in the pipe needs to change, and
an unchanged protocol is the cheapest one to keep compatible; if Kade picks section 1 A, take C over
A (it keeps an old resident DLL usable for sending, which A does not).

## 5. Several clients

### Facts

- **Each game process has its own pipe, session and DLL** (F-MC-1): the pipe is named by the game's
  pid (`pipe_name`, `ingame_pipe.rs:25-27`), and `send_chat`'s `target` is that pid, `None` only
  with one client (`SendBackend::send`, `ingame_send.rs:479-487`; `pick_target`, `:97`).
- **Each DLL seeds its own `call_id` counter** (`CallIds::new`, `send.rs:79-86`), so two DLLs can mint
  the same id. With random 31-bit seeds the chance that two clients' pending ids meet is tiny, but
  not zero (estimated).
- **A Return belongs to the connection it arrived on.** The sniffer keys streams by server and
  client address and port (`StreamKey`, `stream_tracker.rs:5-8`); the roster names a connection's
  local end (`Endpoint`, `game_clients.rs:21-24`, `Roster` at `:40`); `chat_connections()` gives each
  pid's local endpoints on the chat port (`src-tauri/src/ingame/win/tcp.rs:83-86`).
- **Spacing per client is coming** (F-MC-3, tasks#39, DS-0009 item 2): one 1500 ms window per
  `target`, and a `client` field on `SendChat`; this doc takes both as given.
- **`own_player` is one slot today** and flaps between clients; F-GR (tasks#40) gives each
  connection its own. The verdict does not use it.
- **The cooldown is the server's, per channel** (`ChatSendResult.cs:8-10`, reported), and the server
  knows the character by its connection, so it is per character and per channel (derived).

### Options

- **A. Key by `(local endpoint, call_id)`; the endpoint comes from the pid at send time.** When
  `send_chat` sends to pid `p`, the app reads `chat_connections()` once (a Win32 table read it
  already makes to list clients) and registers the pending send under `p`'s local endpoint on the
  chat port. A Return is looked up by the `StreamKey`'s client half and its `call_id`. The matcher
  in `crates/core` stays pure (no Win32): it sees endpoints and ids only. Two DLLs' equal ids on two
  connections never cross. The cooldown map is keyed `(pid, channel)`.
- **B. The DLL reports the verdict on its own pipe.** The pid is known by construction, no mapping.
  Needs section 1 A or C (a receive hook in every game process).
- **C. One global verdict, last one wins.** Not recommended: with two clients sending close together
  a Return for one can be shown as the other's verdict, a wrong "sent" or a wrong cooldown on the
  other character's channel.

**Behaviour around it (all options):**

- **The client picker changes while a send waits for its verdict.** With section 3's delivery A the
  verdict is the command's own result and belongs to the client it was sent to; the composer shows
  it, and the cooldown it starts applies to that client only. Choosing the other client shows that
  client's own cooldown state (or none).
- **A client's game closes with a send pending.** Its connection's Return never comes; the pending
  entry ends as "no verdict" after `VERDICT_WAIT`, and its cooldowns are dropped when the pid leaves
  `game_clients` (the stream itself is forgotten after `STREAM_IDLE_TIMEOUT`, 60 s,
  `stream_tracker.rs:13`).
- **The client reconnects** (new local port): the pending entry under the old endpoint expires as
  "no verdict"; the next send reads `chat_connections()` again.

`Recommended: A`, because the connection is what a Return truly belongs to, the endpoint is known
once per send from a table the app already reads, and the matcher stays a pure function tested on
any OS; it is the only option that works with section 1 B.

## The tests the work would need

Pure tests, in `crates/core` unless named otherwise, each one assertion set (for the recommended
options: section 1 B, 2 C, 3 A and C, 4 B, 5 A):

1. `a_send_return_with_code_0_is_accepted`: a `0x0003` frame built by hand (header `stub_id` 0,
   `call_id` `0x8000_0011`, `error_id` 0; root `{1: {3: {…}, 4: T}}`) gives a Return with that
   `call_id` and `verdict(…)` = accepted, with the cooldown end `T` for the channel.
2. `a_5211_with_a_cooldown_end_gives_the_seconds_left`: the same with `{5: 5211, 4: T}` and
   `server_now` = `T - 3` gives cooldown with 3 s; `T - 2.2` rounds up to 3 (seconds are whole, so
   this is `server_now` passed in milliseconds or the offset rule's fraction); `server_now` unknown
   gives "too frequent"; `T - 700` gives "too frequent" (above the 600 s cap).
3. `an_unknown_code_or_error_id_is_refused_unknown`: `err_code` 9999, or `error_id` 1 with an empty
   root, gives refused-unknown; the number is not in any sentence.
4. `a_plain_0x0003_echo_becomes_a_return`: the existing `a_plain_0x0003_echo_stays_skipped`
   (`framing.rs:372-377`) changes meaning: the frame is no chat and no history (still asserted), and
   it now comes out as a Return. History frames (`{3: channel, 5: chat}`) still parse as history.
5. `an_unmatched_call_id_is_ignored`: a Return whose `call_id` is in the lower half changes nothing;
   one in the upper half that no send registers within `VERDICT_WAIT` is dropped from `early`.
6. `a_return_before_its_send_is_matched`: the Return is fed at `t0`, the send registered at
   `t0 + 5 ms`: the send's state is the Return's.
7. `the_timeout_gives_no_verdict_and_frees_the_entry`: a send registered at `t0`, no Return; at
   `t0 + VERDICT_WAIT` it is "no verdict" and `pending` is empty; a Return for it at
   `t0 + 2 s` updates only the cooldown.
8. `the_65th_pending_send_evicts_the_oldest`: 65 sends registered, the first one's Return is
   ignored, the 65th's is matched; the same for `early`.
9. `two_clients_with_the_same_call_id_do_not_cross`: one id registered on endpoints `E1` and `E2`,
   a refusal on `E2` and an acceptance on `E1`: each send gets its own.
10. `a_cooldown_blocks_only_its_client_and_channel`: after a World cooldown to `T` on pid 7, a World
    send on pid 7 before `T` is `SendError::Cooldown` with the seconds left and nothing reaches the
    backend; World on pid 42 and Party on pid 7 pass; at `T` World on pid 7 passes.
11. `a_malformed_return_is_ignored_and_never_panics`: truncated headers (0 to 11 bytes), a root
    whose lengths run past the end, a varint of 11 bytes, a root that is not `0x0A`: no Return, no
    panic (a `proptest`-style loop over random bytes if the crate has one; plain cases otherwise).
12. `crates/types`: `SendVerdict` round-trips through serde with the camelCase names the ui reads,
    and the existing `Reply::Send` test (`ingame_pipe.rs:460`) stays.
13. `src/`: the composer's pure parts (`composer_view.rs`): which state clears the draft, the
    countdown's text for 3, 2, 1, 0 s, the send button disabled while a cooldown runs for the chosen
    client and channel.

**What cannot be unit-tested (Kade's check in the game, on Windows):** that the `0x0003` frame is
the Return, where its `call_id` sits, the codes, and the cooldowns. It can be done **before any M2
code**, with the app as it is: turn on the raw capture (`raw_capture`,
`crates/types/src/app_config.rs:82-84`; `src-tauri/src/services/sniffer/raw_capture.rs`), log in,
then

1. send one line to World from the composer, and a second one about 2 s later;
2. send two lines to Party 2 s apart, then one to Guild and one to Local;
3. type one line in the game's own chat box;
4. stop the capture and hand the raw file to a session (it holds chat text and names: it stays on
   the PC, or goes only through Kade's private channel; it is never committed).

A session then reads the `0x0003` frames that follow each send: a `call_id` in the upper half right
after each composer send, a root `{1: {…, 4: …}}` for the first World line, `{1: {4: …, 5: 5211}}`
for the second (if the 5 s cooldown holds), no field 4 on Party and Guild. What the game's own chat
UI shows after the refused World line, and whether the refused text appears anywhere, is watched on
screen. After the M2 tasks: the same steps, checking the composer's states, plus two game clients
sending to World within 1 s of each other.

## Tasks it would split into

For the recommended options; each is one PR, in order. The DLL is not touched by any of them.

| # | task | files | gate | blocked by |
|---|---|---|---|---|
| 0 | Kade's raw capture above (no code) | none | none | nothing; settles the layout before task 1 |
| 1 | The Return in `crates/core`: a `Return` frame kind for a plain `0x0003` that is not history, its header, the `SendChitChatMsgReply` reader, `verdict()` | `crates/core/src/protocol/framing.rs`, a new `crates/core/src/protocol/rpc_return.rs` (a new module: Kade's OK with Q1) | `just core-check` | Q1, task 0 |
| 2 | The matcher and the cooldown gate: pending and early maps, `VERDICT_WAIT`, the per-client, per-channel cooldown, `SendError::Cooldown`, `send_chat_flow` waiting for the verdict | `crates/core/src/ingame_send.rs`, `crates/core/src/send_text.rs` | `just core-check` | task 1, Q2, Q3, Q4, F-MC-3 (same functions) |
| 3 | The wiring: the sniffer hands Returns with their endpoint to the matcher; `send_chat` registers each send under the pid's chat endpoint; `SendChat` returns `SendVerdict` | `crates/types/src/ipc.rs`, `src-tauri/src/ingame/mod.rs`, the sniffer's pipeline in `src-tauri/src/services/sniffer/`, `crates/core/src/capture/pipeline.rs`; the ui's one `SendChat` call site kept compiling | `just core-check`, `just ui-check`, `just app-cross-check` (Linux, compile only; the app's own tests run on Windows CI) | task 2, F-GR (the capture pipeline) |
| 4 | The composer: the states, the countdown, the disabled button | `src/composer_view.rs`, `src/components/chat_composer.rs` | `just ui-check` and the `ui-preview` skill | task 3, Q3 |

With Q1 A or C instead, tasks 1 and 2 stay (the decoder lives in `crates/frame` for the DLL), a DLL
task (two hooks, `just ingame-cross-check`, `NOT VERIFIED: just ingame-build: Windows only`) and a
pipe task (section 4 C) come before task 3, and D-35 needs an ADR change first.

## Questions for Kade

Four choices are his: how far the app reaches into the game, how the composer behaves, and what a
user sees when the server is silent. Section 2's key and section 4's pipe follow from question 1
and are not asked.

1. **Where is the verdict read (section 1)?**
   - A. In the DLL: hooks on `ZTcpConnection.OnData` (and `ZRpcImpl.SendBytes`), reported over the
     pipe. Widens D-35 ("no other hook") by two hooks on the game's network thread.
   - **B. In the app, from the Return the sniffer already captures (Recommended).** No new hook, no
     pipe change, D-35 unchanged, and a game patch that renames methods does not touch it.
   - C. Both: B as the verdict and A's hooks as a cross-check. All of A's cost for verdicts on lines
     the game's own chat box already shows.
2. **When does the composer learn the verdict (section 3)?**
   - **A. The send waits for it, up to 1 s, and returns it (Recommended).** The draft stays until
     the server answers, so a refused line is never lost; one round trip slower to clear.
   - B. The send returns at once and the verdict comes as an event; a late refusal puts the line
     back in the input, over whatever was typed since.
   - C. A, plus the event for an answer later than 1 s.
3. **What does the composer do on a cooldown (section 3)?**
   - A. One sentence with the seconds left at that moment.
   - B. A countdown in the sentence, to zero.
   - **C. The countdown, plus the send button disabled for that character's channel until it ends,
     and the app not sending there before then (Recommended).** The server gives the exact end;
     not sending a line it will refuse is what the game's own chat box does.
4. **What does a send with no answer from the server show (section 2)?**
   - A. An error, and the draft kept: the user is told it failed, although it most likely arrived.
   - **B. A neutral line, "sent, not confirmed by the server", and the draft cleared (Recommended).**
     The line was handed to the game's connection; a missed answer is far more likely a missed
     packet than a lost line.
   - C. Nothing: treat it as sent.

## Summary

| section | recommendation | rests on |
|---|---|---|
| 1. Read point | B: the app decodes the sniffed `0x0003` Return; no new hook, D-35 unchanged | Question 1 |
| 2. Matching | C: `(connection, call_id)`, pending and early maps of 64, `VERDICT_WAIT` 1000 ms (estimated), "no verdict" after it | Question 4 |
| 3. States | the table: accepted, cooldown `n` s, too frequent, refused unknown, no verdict, not sent; server clock only; the command waits and returns the state | Question 2 |
| 3. Cooldown | C: countdown, button disabled and a backend gate per client and channel; Party/Guild only the 1500 ms spacing | Question 3 |
| 4. Pipe | B: no change, `PIPE_PROTOCOL_VERSION` stays 1 (C if question 1 is A) | Question 1 |
| 5. Several clients | A: key by the pid's chat endpoint, read once per send; cooldowns per pid and channel | Question 1 |

## Not verified

Only a run in the game on Kade's PC can show these (task 0 settles the first four):

- That the plain `0x0003` frame is an RPC Return with Stellar's 12-byte header (`stub_id`, `call_id`,
  `error_id`), and that the "echo" `framing.rs` drops is a `SendChitChatMsgReply`.
- That the Return for a DLL send carries the DLL's own upper-half `call_id`, on the same TCP
  connection, and that the game's own `call_id`s stay in the lower half.
- Every server code other than 0 and 5211, and what `error_id` holds.
- That World and Local reply with a cooldown end (5 s observed by Stellar) and Party and Guild with
  none.
- What the game's own UI does with the server's Return for a call it did not make (R0 already lists
  this), and what it does with the text after 5211.
- Whether the server can leave a send unanswered, and what a timeout looks like.
- That the sniffer sees the Return in practice often enough for "no verdict" to be rare (raw-socket
  segment loss on Kade's PC).
- That every M2 channel's sends stay on TCP (R0's World-over-UDP caveat applies to the send itself,
  not to reading the Return).
- The receive hook's thread and cost, which matter only for question 1 A or C
  (`…/Game/PandaWireTap.cs:9-17`, reported).

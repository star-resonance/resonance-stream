# In-game sender: design options

**Status:** options as written; the choices Kade made are in the Decided note. Task R0 (lane chat-send,
milestone M0). Written 2026-11-15. No code was written, and nothing was built or run in
the game for this document.

**Decided.** Hook-and-pump (section 2 A); `ilhook` 2.3.0 as the detour crate (section 3 D, which the DLL
pairs with its own suspension of the game's other threads); `crates/frame` and `crates/ingame` in the
workspace with a custom root profile that sets `panic = "unwind"` (section 4 A); loading by remote
thread (section 6 A). The DLL is downloaded from the app's own release into `<app data>/bin/ingame/<version>/` and loaded from there in place, with the
file held open across hash and load; no copy is made (not built: if Windows refuses `LoadLibraryW` on the held file the send fails with `LoadFailed`). The matching
rows are in [security_model.md](security_model.md) section 3.8. D-35 in [decisions.md](decisions.md)
is the record.

## How to read this

Each of sections 1-7 lists **facts** first, then **options** (A/B/C), then one line,
`Recommended: <letter>, <reason>`. The doc ends with **Questions for Kade** (at most 4, each
answered by picking a letter), a summary table, and **Not verified**: the claims that only a run in
the game can settle.

Evidence labels follow `docs/security_model.md` (line 6): **confirmed** means the code was opened and
the `file:line` says it; **reported** means a source says it but nobody here checked it by hand
(for example a library's own documentation, or a comment in Stellar describing the game);
**estimated** means reasoning, not reading. `NOT VERIFIED` marks a claim about the game's behaviour
at run time, which no agent can check.

Citations into this repository are `path:line` on `main` at `c7a733b`. Citations into the Stellar
framework are `Stellar@60a7772:<path>:<line>`. `<path>` is relative to the
StellarProtocol/StellarResonanceModSystem repository at commit
`60a7772c8de86ec62016703ede89079955f63166`, and `…/` stands for `src/Stellar.Infrastructure/`.
Stellar is AGPL, so this doc and the code that follows it take **facts** from it (names, numbers,
signatures, call order) and copy no code or comment text. Stellar is a C# BepInEx plugin, and its
own docs say it is read-only. The Rust DLL reimplements the one send path it needs (DS-0001: no Stellar plugin;
its features are analysed and reimplemented).

## Fixed decisions (not options)

These were decided in DS-0001 and DS-0002 and in the lane's shared values. This doc builds on them and does not reopen them.

- **Mechanism.** The app is already Administrator (`src-tauri/app.manifest:19`, `requireAdministrator`).
  It loads its own Rust DLL `resonance_ingame.dll` into the game process. The DLL calls the game's
  `ZCode.ZNet.ZTcpClient.Send(ReadOnlySpan<byte>)` on the live chat connection, as Stellar does. No
  Stellar, no C#, no keystrokes, no WinDivert/Npcap. The feature is an opt-in setting, off by
  default, and turning it on shows a terms-of-service warning.
- **Frame.** All integers are big-endian, and `size` counts the whole frame including itself. The
  header is 26 bytes (`Stellar@60a7772:…/Game/PandaChatProbe.Send.Builders.cs:141-169`):

  | field | type | value for a chat send | source |
  |---|---|---|---|
  | `size` | u32 | 26 + payload length | `…/Game/PandaChatProbe.Send.Builders.cs:143-149` |
  | `flags` | u16 | `1` (ZprotoMsgTypeId Call; bit 15 = zstd, never set) | `…/Game/PandaChatProbe.cs:112`, `Send.Builders.cs:151-152` |
  | `service_uuid` | u64 | `1321197368` (ChitChat), high 32 bits zero | `…/Game/PandaChatProbe.cs:96`, `Send.Builders.cs:154-155` |
  | `stub_id` | u32 | `0` | `…/Game/PandaChatProbe.Send.cs:290` |
  | `call_id` | u32 | a counter, first value 1 | `…/Game/PandaChatProbe.Send.cs:87-91`, `:287` |
  | `method_id` | u32 | `1` (SendChitChatMsg) | `…/Game/PandaChatProbe.cs:107` |
  | payload | bytes | protobuf, below | `…/Game/PandaChatProbe.Send.Builders.cs:34-60` |

  The payload is `SendChitChatMsg { 1: SendChitChatMsgRequest { 2: channel_type varint, 4: ChatMsgInfo
  { 1: msg_type 0, 2: target_id (whispers only), 3: msg_text string } } }`, with the text as UTF-8
  (`Send.Builders.cs:36-60`).
- **Channels.** M1 has `say` (the game's current scene channel, "Local", label 로컬), `world`, `party`
  and `guild`. Whispers are not in M1. The wire `channel_type` values come from `MapSendTarget`
  (`Stellar@60a7772:…/Game/PandaChatProbe.Send.cs:348-356`):

  | `SendChatChannel` (serde) | wire `channel_type` | game name |
  |---|---|---|
  | `world` | 1 | ChannelWorld |
  | `say` | 2 | ChannelScene |
  | `party` | 3 | ChannelTeam |
  | `guild` | 4 | ChannelUnion |
  | (whisper, M3) | 5 | ChannelPrivate |

  `SendChatChannel` lives in `crates/types`. Its `ALL` is in tab order World, Guild, Party, Say.
  `from_channel(Channel)` returns an `Option`, and Beginner maps to `None`.
- **Text.** Turn each control character (CR, LF, TAB, NUL, U+2028/U+2029 and the rest) into one space, then trim
  (DS-0008). Empty text is an error. The limit is
  140 UTF-16 code units (`encode_utf16().count()`), counted after that preparation. `prepare_text` and
  its errors live in `crates/types` (DS-0002).
- **Spacing.** The app refuses a send that comes less than 1500 ms after its last successful send.
  The check is pure and takes an explicit `now`. `SendSpacing` lives in `crates/core`.
- **Errors.** Each error reaches the ui as one Korean sentence, the `Err(String)` of the `send_chat`
  command (ipc, task R6: `SendChat = "send_chat" { channel: SendChatChannel, text: String, client: Option<u32> } -> ();`).

## Sections

1. Finding the live chat `ZTcpClient`
2. The thread `Send` must run on
3. The detour crate
4. Where the frame builder and the DLL crate live, and goldens
5. The app <-> DLL pipe protocol
6. Loading and unloading
7. Failure and patch resilience

Then: Questions for Kade, summary, Not verified.

## 1. Finding the live chat `ZTcpClient`

### Facts

- **Chat has no socket of its own.** The game multiplexes its RPC over transports owned by
  `ZCode.ZRpc.ZRpcCtrl`. ChitChat is a Lua RPC proxy with id `1321197368`, the same number as the
  service uuid (`Stellar@60a7772:…/Game/PandaChatProbe.Send.LiveClient.cs:18-28`, `:71`). Reported:
  Stellar's comments describe the game.
- **Active walk (Stellar's primary path since its 2.25.5 fix).** On every send, with no cache:
  `ZSingleton<ZRpcCtrl>.Instance` -> `ZRpcCtrl.GetRpcByProxyId(ulong 1321197368)` -> `ZRpcImpl.GetTransfer()`
  -> `ZRpcTransfer.GetTransport()` -> field `ZTransport.conn_` -> field `ZTcpConnection.client_`
  (`…/Game/PandaChatProbe.Send.LiveClient.cs:30-37`, code `:115-190`). Stellar's comment gives the field
  offsets as `conn_` at 0x10 and `client_` at 0x60 in its release_3.7 dump (`:35-36`). Offsets like these
  change from build to build.
- **`GetRpcByProxyId` works right after login.** It resolves by proxy-manager membership, which is
  filled at login. A second lookup (`GetChannelTypeByProxyId`) stays empty until the game's first
  native chat send (`…/Game/PandaChatProbe.Send.LiveClient.cs:21-28`). Reported.
- **The interface-wrapper trap.** Il2CppInterop hands each hop back typed as its interface (`IRpc`,
  `ITransfer`, `IConnection`). A member must be looked up on the concrete class, never on the
  interface (`…/Game/PandaChatProbe.Send.LiveClient.cs:38-51`). `GetRpcByProxyId` is bound on the concrete
  `ZRpcCtrl`, never through `IRpcCtrl` (`:59-62`). This trap belongs to Il2CppInterop's managed wrappers.
  The native il2cpp API gives a raw `Il2CppObject*` whose class (`il2cpp_object_get_class`) is the
  real one. The same rule still applies there: look methods up on the declaring concrete class
  (estimated).
- **TCP only.** If `conn_` is not a `ZTcpConnection`, the connection is UDP/KCP and there is no
  `client_` (`…/Game/PandaChatProbe.Send.LiveClient.cs:172-176`). Reported: the World channel can run over
  TCP or UDP/KCP (`…/Game/PandaChatProbe.Send.Hooks.Rpc.cs:28-31`).
- **The game never calls the one-argument `Send` itself.** The game's own path is
  `ZRpcImpl.ProxyCall` -> `ProxyCallInternal` -> `SendBytes(ulong uuid, uint stubId, uint callId, uint
  methodId, byte[] pbData, uint pbSize, bool framePack)` -> transport. A prefix on
  `ZTcpClient.Send(ReadOnlySpan<byte>)` fired 0 times for game traffic
  (`…/Game/PandaChatProbe.Send.Hooks.Tcp.cs:18-24`, `…/Game/PandaChatProbe.Send.Hooks.Rpc.cs:17-37`). Reported. So
  the fixed mechanism sends through a method that exists but that the game does not use itself.
  It works only while chat runs over TCP (section 7).
- **Finding `Send`.** `ZTcpConnection.OnData(ReadOnlySpan<byte>, ZTcpClient)` anchors the `ZTcpClient`
  type, and `Send` is its one-argument method whose parameter is `ReadOnlySpan`1`
  (`…/Game/PandaChatProbe.Send.Hooks.Tcp.cs:26-67`, `:94-152`).
- **Liveness.** `ZTcpClient` is a `ZNativeObject` with a native `handle_`. Stellar refuses a client
  whose handle is zero and fails closed when it cannot read the handle
  (`…/Game/PandaChatProbe.Send.Connection.cs:176-201`, `…/Game/PandaChatProbe.Send.LiveClient.cs:182`). Sending through a freed
  handle is a native access violation that kills the game (`…/Game/PandaChatProbe.Send.cs:131-134`). Reported.
- **Passive cache (Stellar's fallback).** The client is also taken from inbound chat packets and from
  the outbound `SendBytes` hook, and prefixes on `ZTcpClient.Close(Action<ZTcpClient>)` and
  `ZNetExtensions.CloseAndDelayDispose(ZTcpClient)` drop it
  (`…/Game/PandaChatProbe.Send.Connection.cs:69-130`, `:147-170`, `:225-290`). The outbound hook may run on a
  net thread (`:94-96`).
- **No per-frame polling.** Stellar resolves only inside a send, because reflecting into `ZRpcCtrl`
  on a timer once stalled the game's `ZServerTime` singleton (`…/Game/PandaChatProbe.Send.LiveClient.cs:52-57`).
  Reported.
- **What the native il2cpp API offers.** `GameAssembly.dll` exports the il2cpp embedding API by name:
  `il2cpp_domain_get`, `il2cpp_domain_get_assemblies`, `il2cpp_assembly_get_image`,
  `il2cpp_class_from_name`, `il2cpp_class_get_parent`, `il2cpp_class_get_method_from_name`,
  `il2cpp_class_get_field_from_name`, `il2cpp_field_get_offset`, `il2cpp_field_static_get_value`,
  `il2cpp_runtime_invoke`, `il2cpp_object_get_class`, `il2cpp_thread_attach`. The ones Stellar's code
  calls directly are `il2cpp_object_get_class`, `il2cpp_class_get_namespace` and `il2cpp_class_get_name`
  (`Stellar@60a7772:…/Game/PandaChatProbe.Receive.Resolution.cs:53-57`), `il2cpp_class_value_size`
  (`…/Hooks/Il2CppStructSizes.cs:29`), and `il2cpp_class_has_references`
  (`…/Unity/Il2CppClassInitFix.cs:28`). `il2cpp_thread_attach` is named for a pool thread
  (`…/Rendering/UnityFrameGrabber.Capture.cs:112`). The others are the public il2cpp API, which Il2CppInterop
  itself builds on (reported, not read here). Whether this game build exports each one by name is
  NOT VERIFIED. The DLL looks every export up with `GetProcAddress`, never with a signature scan:
  some builds of this game defeat Il2CppInterop's scan for `Class::Init`, seen on `StarSEA_STEAM`
  (`…/Unity/Il2CppClassInitFix.cs:8-14`, `…/Unity/UnityTickHost.cs:34-37`).

### Options

- **A. Active walk from Rust, no hook.** Inside each send, the DLL:
  1. finds the classes with `il2cpp_class_from_name` (namespace `ZCode.ZRpc` / `ZCode.ZNet`, searched
     over the domain's assemblies);
  2. reads `Instance` from `ZRpcCtrl`'s parent (the inflated `ZSingleton<ZRpcCtrl>`, through
     `il2cpp_class_get_parent`), calling `get_Instance` with a null `this`;
  3. calls `GetRpcByProxyId`, `GetTransfer` and `GetTransport` with `il2cpp_runtime_invoke`, each method
     taken from its concrete class;
  4. reads `conn_` and `client_` at offsets from `il2cpp_field_get_offset`, never hard-coded 0x10/0x60;
  5. checks `il2cpp_object_get_class(conn)` against `ZTcpConnection` (otherwise UDP: `no_client`);
  6. reads `handle_` and refuses zero.

  The classes, methods and field handles are resolved once and cached. The objects are not cached.
  `il2cpp_runtime_invoke` returns a managed exception through its `exc` out-parameter instead of
  throwing it. Calling `methodPointer` directly would let an il2cpp (C++) exception unwind through
  Rust frames, which is undefined behaviour. So the walk uses `runtime_invoke` everywhere. It costs a
  few boxed arguments on a cold, user-paced path (estimated: microseconds).
- **B. Passive cache through a detour.** Detour a method the game calls with the live object, for
  example `ZRpcImpl.SendBytes` (the game's own chat and history sends go through it; it runs at
  login) or `ZTcpConnection.OnData`. Store the `client_` reached from `this` in an atomic, and drop it
  through detours on `ZTcpClient.Close` and `CloseAndDelayDispose`. That is 1 to 3 hooks. A stale
  pointer is the main risk: a missed close path leaves a pointer to a freed object. The handle check
  narrows that window but does not close it, because reading `handle_` through a freed object is
  itself a use-after-free.
- **C. A first, B as fallback** (what Stellar does). This covers the most cases, but it has the most
  code and every hook from B.

| | hooks | stale pointer | works at fresh login | code |
|---|---|---|---|---|
| A | 0 | none: nothing cached across sends | yes (`GetRpcByProxyId`, reported) | small |
| B | 1-3 | possible (missed close path) | only after the first chat or `SendBytes` | medium |
| C | 1-3 | as B, fallback only | yes | largest |

`Recommended: A`. It needs no hook, caches no object (so nothing goes stale), and is the path Stellar
itself made primary. B is added only if Kade's in-game test shows A failing (for example,
`GetRpcByProxyId` returning null after a reconnect). It is the long-term option too: fewer patched
game bytes means fewer things for a game patch to break.

## 2. The thread `Send` must run on

### Facts

- **Stellar sends from the Unity main thread.** Its `IChat` contract says the not-sent result is raised
  before `Send` returns when the caller is on the main thread
  (`Stellar@60a7772:src/Stellar.Abstractions/Services/IChat.cs:19-22`). `ChatService.Send` calls
  `TrySend` on the caller's thread (`src/Stellar.Application/Services/ChatService.cs:96-114`). The
  framework's tick is a Unity MonoBehaviour driven by `InvokeRepeating`
  (`…/Unity/StellarTicker.cs:7-8`, `:17`, `:41-45`). Confirmed.
- **The game sends chat from the main thread too.** The `ZRpcImpl.SendBytes` prefix runs on the
  thread that issued the RPC, which is the Unity main thread for Lua-issued chat
  (`…/Game/PandaChatProbe.Send.Hooks.Rpc.cs:98-100`). Reported. Stellar's cache code allows for `SendBytes` running on a
  net thread in general (`…/Game/PandaChatProbe.Send.Connection.cs:94-96`).
- **Nothing in Stellar shows `Send` called from a foreign thread.** Whether `ZTcpClient.Send` (a
  wrapper over a native handle) is safe to call while the main thread uses the same connection is
  unknown. NOT VERIFIED.
- **il2cpp needs an attached thread.** A thread that touches managed objects or calls
  `il2cpp_runtime_invoke` must first call `il2cpp_thread_attach(il2cpp_domain_get())`, so the GC knows
  it exists. Stellar does this for a pool thread (`…/Rendering/UnityFrameGrabber.Capture.cs:112`). A
  pipe thread in the DLL is a foreign thread.
- **A per-frame hook costs little only when it is native.** Stellar deliberately does not hook
  `Game.Update`, because a per-frame Harmony postfix crosses into the managed runtime every frame,
  which it measured as costing roughly 12-18 fps (`Stellar@60a7772:src/Stellar.Host/BootstrapPlugin.cs:42-45`).
  A Rust detour is native code: per frame it costs one atomic load and a branch when the queue is
  empty (estimated: nanoseconds). Stellar does patch a method that a game `Update` calls, but
  only on mouse-press frames and as a managed prefix (`…/Game/GamePressCheckShield.cs:7-19`), so it is
  no proof of a per-frame native hook. The `StellarTicker` idea itself (an injected MonoBehaviour)
  is not open to a Rust DLL without a type injector.

### Options

- **A. Hook-and-pump on the main thread.** The pipe thread only parses and queues (a single-slot
  queue: one send in flight). A detour on a method the game runs on the main thread every frame
  drains the slot and runs section 1's walk and `Send` there, on the same thread the game sends its
  own chat from. The candidate is the `Update` of the game's net loop owner (per Stellar, `ZRpcCtrl` drives the game's
  network update loop, `…/Game/PandaChatProbe.Send.LiveClient.cs:54-56`), or a Unity per-frame entry if that
  method does not exist. R5 hooks `ZRpcCtrl.Update` (namespace `ZCode.ZRpc`; the method name is
  inferred, not read), with `Panda.Core.Game.Update` as the fallback, using ilhook's `JmpBack`. The exact
  method is named from a dump of the current build: NOT VERIFIED.
  It needs one hook, so it needs section 3's detour crate. During a loading screen the main thread
  may not tick, so a send can wait. The pipe reply then has a deadline (section 5).
- **B. Attach and call from the pipe thread.** The pipe thread calls `il2cpp_thread_attach` once and
  runs the walk and `Send` itself. It needs no hook and no detour crate. The risk is a data race
  inside the game's net client: two threads writing the same connection at once can interleave bytes,
  which desyncs the stream (the server drops the connection) or corrupts native state (a crash). It
  can also race with `ZRpcCtrl`'s own update loop, which Stellar has already seen stall the game
  (reported).
- **C. B now, A later.** Ship B for M1 behind the opt-in, and move to A if Kade's test shows a
  problem. A data race may not show in a short test, so a clean test proves little.

| | hooks | new dependency | race with the game's net thread | latency |
|---|---|---|---|---|
| A | 1 | detour crate | none: same thread as the game's own sends (reported) | up to one frame, more on a loading screen |
| B | 0 | none | unknown (NOT VERIFIED) | immediate |
| C | 0, then 1 | later | as B until moved | immediate |

**What is evidence and what is unknown.** Evidence: the game and Stellar both send chat from the
main thread (reported), and Stellar never sends from another thread. Unknown: whether the native
client locks around `Send`. **The in-game test that settles it (Kade's step in M1):** with the DLL
loaded and the main-thread option built, send 50 lines at the 1500 ms spacing while the game itself
sends and receives chat. Repeat with a loading screen in between. Pass: every line arrives once,
the game keeps its connection, and nothing crashes. The same run on B can only show that B did not
fail this time.

`Recommended: A`. It runs on the thread the game itself uses for chat, so no unknown locking is
needed, and a native detour costs nothing measurable per frame. This is the standard practice for
injected code that calls a game engine (marshal onto the engine's thread), and the long-term option
Kade prefers, at the price of one hook and one dependency (section 3).

## 3. The detour crate

A detour is a new dependency, so this is Kade's call (global rules). It is needed only for the
hooks that sections 1 and 2 choose:

| section 1 | section 2 | hooks | detour crate needed |
|---|---|---|---|
| A (walk) | A (pump) | 1 (the per-frame pump) | yes |
| A (walk) | B (attach) | 0 | **no** |
| B or C (cache) | A (pump) | 2-4 | yes |
| B or C (cache) | B (attach) | 1-3 | yes |

### Facts (crates.io API, read 2026-11-15)

| | `minhook` 0.9.0 | `retour` 0.3.1 | `detour` 0.11.1 | hand-rolled | `ilhook` 2.3.0 |
|---|---|---|---|---|---|
| what | Rust bindings that vendor the C MinHook library (built by `cc`) | maintained fork of `detour`, pure Rust plus `libudis86-sys` (C) | the original `detour`, revived in 2026 | our own code | pure Rust; `iced-x86` is its disassembler |
| last release | 2025-12-14 (0.8.0 2025-08, 0.7.x 2025-02/03) | stable 0.3.1 2023-07-18; only `0.4.0-alpha.4` since (2025-09-11) | 0.11.1 2026-11-09 (0.9.0 2026-09-23, 0.10.0 2026-09-30) | n/a | 2.3.0 |
| license | MIT (MinHook itself: BSD-2-Clause, reported) | BSD-2-Clause | BSD-2-Clause | MIT (ours) | not checked |
| normal deps | `tracing` (build: `cc`) | `cfg-if`, `generic-array`, `libc`, `libudis86-sys`, `mmap-fixed-fixed`, `once_cell`, `region`, `slice-pool2` | `iced-x86`, `libc`, `mach2`, `region`, `windows-sys` 0.61 | none (`windows-sys` already in the tree) | `iced-x86` (no C build step) |
| x64 Windows | yes (MinHook's main target, reported) | yes (reported) | yes (reported) | as written | yes (the x64 patch is `src/x64.rs`, source read) |
| other threads while patching | MinHook suspends the process's other threads and moves their instruction pointers out of the patched bytes (MinHook's documented behaviour, reported) | not documented (NOT VERIFIED) | not documented (NOT VERIFIED) | to be written: a suspend-all is about 100 lines | not documented, but it offers a `ThreadCallback` with `pre()`/`post()` around the write (NOT VERIFIED beyond the source read) |
| first bytes not relocatable | refuses with an error code (`MH_ERROR_UNSUPPORTED_FUNCTION`, reported) | returns an error (reported) | returns an error (reported) | we would need an instruction decoder | refuses with `HookError::MovingCodeNotSupported` or `Disassemble` |
| unhook | `MH_DisableHook` / `MH_RemoveHook` / `MH_Uninitialize` | drop or `disable()` | drop or `disable()` | to be written | `HookPoint::unhook`; `Drop` unhooks and swallows any error |

The repository's own license is MIT (`LICENSE`), and all three crates' licenses are compatible with
it. `windows-sys` 0.61.2 is already a dependency of the app (`src-tauri/Cargo.toml:53`).

`ilhook` 2.3.0, read from its source:
- It patches a 14-byte window at the target (`jmp_inst_size: 14`, `src/x64.rs:263`), written with plain
  byte copies, so the write is not atomic, and it puts the 14 original bytes back on unhook.
- Its README says it "is not thread-safe" unless a flag (`NOT_MODIFY_MEMORY_PROTECT`) is set, so one
  thread hooks and unhooks.
- It does not suspend other threads itself; `ThreadCallback` `pre()` and `post()` run around the write.

### Options

- **A. `minhook`.** MinHook is the de facto standard in Windows game modding and has been used in
  production for over a decade (reported). It has the smallest dependency tree and handles thread
  suspension while patching. Trade-off: a C library compiled by `cc`, which works on the Windows CI
  runner (MSVC) and with `gcc-mingw-w64-x86-64` for `just app-cross-check` (estimated), and Rust
  bindings by one maintainer.
- **B. `retour`.** Pure Rust apart from the disassembler. Trade-off: no stable release since 2023
  (only alphas), a larger tree, and its `static_detour!` macro needs nightly Rust (reported). The
  stable API is the generic/raw detour.
- **C. Hand-rolled.** A 14-byte absolute `jmp` patch plus a trampoline. It adds no dependency, but
  it needs an x64 instruction-length decoder to relocate the overwritten prologue (RIP-relative
  operands), and a thread suspend while patching. That is a few hundred lines of the most
  safety-critical code in the DLL, which nobody else tests. Not recommended.
- **D. `ilhook` 2.3.0.** Pure Rust, with `iced-x86` as its disassembler and no C build step. Trade-off:
  it has no thread suspension of its own (not documented), so the DLL's `ThreadCallback` suspends the
  game's other threads around enabling the hook, with `windows-sys` (R5), and allocates and locks
  nothing between suspend and resume.

(`detour` 0.11.1 is listed for completeness. Its revival is two months old, so it has no track
record yet.)

**Upkeep, in words Kade can weigh.** Each option adds one crate that Dependabot will raise PRs for.
A and B each add a C build step to the DLL crate only (not to the app or the ui). With A, a broken
release means pinning the previous one; MinHook's C code has barely changed in years (reported). With
C there is nothing to bump, but every bug is ours, and a bug here crashes the game. D adds one
pure-Rust crate (and `iced-x86`) and no C build step; it adds no thread suspension of its own, so the
DLL's thread callbacks carry that code.

`Recommended: D (ilhook 2.3.0)`, if section 2's A is chosen. Kade asked for a pure-Rust crate, and none
of the pure-Rust crates documents thread suspension, so the DLL suspends threads itself. `minhook`
stays described as the C-library option: the standard tool, with the smallest tree and its own
suspension. If Kade picks section 2's B, no detour crate is needed at all.

## 4. Where the frame builder and the DLL crate live, and goldens

### Facts

- **The workspace aborts on panic in release.** The root `Cargo.toml` sets `[profile.release]
  panic = "abort"`, `codegen-units = 1`, `lto = true`, `opt-level = "s"`, `strip = true`
  (`Cargo.toml:35-40`). Profiles take effect only at the workspace root (comment at `:32-34`). The
  workspace members are `src-tauri`, `crates/core`, `crates/llama` and `crates/types` (`Cargo.toml:25`).
- **Cargo rules (reported, from the Cargo book "Profiles"):** a per-package override
  (`[profile.release.package.<name>]`) cannot set `panic`, `lto` or `rpath`. A **custom profile**
  (`[profile.<name>] inherits = "release"`) can set `panic`, and it is chosen with `--profile <name>`.
  Under `panic = "abort"`, `std::panic::catch_unwind` catches nothing: a panic anywhere in a DLL built
  that way aborts the whole game process. Since Rust 1.81, a panic that unwinds out of an
  `extern "C"` function aborts as well (reported). So the DLL needs `panic = "unwind"` **and** a
  `catch_unwind` at every FFI edge (section 6).
- **`crates/core` is too heavy for the DLL.** It depends on `lindera` with embedded IPADIC (its build
  script downloads the dictionary from Lindera.dev, `crates/core/Cargo.toml:15-17`; `CLAUDE.md`,
  Guardrails: Trusted hosts), plus `kakasi`, `regex`, `ruzstd`, `etherparse`, `sha2`,
  `minisign-verify` and others (`crates/core/Cargo.toml:9-25`).
- **`crates/types` is light.** Only `serde` and `serde_with` (`crates/types/Cargo.toml`). It compiles
  to wasm for the ui. DS-0002 puts `prepare_text` and its errors there, and the lane puts
  `SendChatChannel` there.
- **Gates name packages.** `just core-check` runs `cargo test -p resonance-core -p resonance-llama
  -p resonance-types` (`justfile:21-22`), and `just coverage` names the same three (`justfile:48`). A new
  crate needs a `justfile` line and a CI line to be gated. Those changes are a follow-up, not R0's.
- **The corpus holds no outbound frames.** The recorder keeps only server-to-client data:
  `is_capturable` is `tcp.source_port == 5003` with a non-empty payload
  (`crates/core/src/capture/recorder.rs:18-29`). The pipeline drops anything else
  (`crates/core/src/capture/pipeline.rs:83-85`). `sanitise` reads lines through the same filter
  (`crates/core/src/capture/sanitise.rs:206-224`). The one fixture,
  `crates/core/tests/fixtures/oct03.sanitised.capture.log` (2164 lines, generated, `.gitattributes`),
  is inbound only. `crates/core/testdata/replay-sample.jsonl` is a hand-written chat sample for
  `--replay-chat`. **No real outbound frame exists anywhere in the repository to use as a golden.**
- **What the inbound parser can cross-check.** Every inbound frame is `[u32 length incl. header,
  BE][u16 type][body]` (`crates/core/src/protocol/framing.rs:3-12`, `PREFIX_LEN` 6 at `:29`). The inner
  header after the prefix is 16 bytes for type `0x0002` and 12 for the rest (`:187-193`). The outbound
  Call shares only the 6-byte prefix (`size`, then `flags` = 1). Its next 20 bytes (uuid, stub, call,
  method) are Stellar's layout, which the inbound parser never reads. So the parser can check the
  prefix, and through the protobuf reader (`crates/core/src/protocol/decoder.rs:4`, `:41`, `:136`, `:146`) the
  payload. It cannot check the 20 header bytes.

### Options: the frame builder

- **A. A new small crate, `crates/frame`** (package `resonance-frame`). It has no dependencies,
  is `#![no_std]` with `alloc`, and is pure. It holds the channel's wire value, the protobuf payload,
  the 26-byte header, and the DLL's second check of the text (empty, CR/LF, 140 UTF-16 units). Both
  the DLL and `crates/core`'s tests depend on it. It is tested on any OS under `just core-check` once
  that line names it.
- **B. In `crates/types`.** No new crate, but the DLL then compiles `serde` and `serde_with` for
  nothing, and wire details end up in a crate that exists for the Tauri boundary.
- **C. Inside the DLL crate, not shared.** The golden tests live in the DLL crate, but `crates/core`
  cannot round-trip the frame through its parser, and the DLL crate's tests run only where that
  crate is built.

`Recommended: A`. A pure, dependency-free crate is the easiest code in the lane to test, and the
standard shape for a wire format. The DLL stays small, and `core` can check every frame it builds.

### Options: the DLL crate (`crates/ingame`, package `resonance-ingame`, `cdylib` `resonance_ingame.dll`)

- **A. In the workspace, built with a custom profile.** Add `[profile.ingame] inherits = "release"`,
  `panic = "unwind"` at the root, and build the DLL with `cargo build -p resonance-ingame --profile
  ingame --target x86_64-pc-windows-msvc`. One lockfile, one Dependabot config, and the same
  `windows-sys` version as the app. Trade-off: a plain `cargo build --release` of the whole
  workspace also builds the DLL with `abort`. That copy must never be shipped, so the packaging step
  builds the DLL only through `--profile ingame` (a check in `package.bat` and Candidate: a follow-up).
- **B. Outside the workspace** (`ingame/` with its own `[workspace]` table, `Cargo.lock` and
  `[profile.release] panic = "unwind"`). The profile can never be wrong, but there are two lockfiles,
  a second Dependabot entry, and `crates/frame` is used through a path dependency across workspaces.
- **C. In the workspace with `abort`.** Remove every panic path instead (`#![deny(clippy::unwrap_used,
  clippy::expect_used, clippy::indexing_slicing, …)]`). It is simple to build, but an allocation
  failure or a panic in a dependency (a dependency's code, `serde_json`) still kills the game, and
  `catch_unwind` cannot help. Not recommended.

`Recommended: A`. It keeps one workspace and one lockfile, and the custom profile is Cargo's own
answer to "this one artifact needs a different panic strategy". The packaging check removes A's one
trap.

### Goldens: how an outbound golden can be made

The corpus has none, so the golden for M1 is a mix. Each kind proves something different:

1. **Derived bytes (unit test in `crates/frame`).** Hand-derive the bytes for fixed inputs (for
   example `world`, text `"hi"`, `call_id` 1) from the layout in the fixed decisions, and pin them as a
   test. This proves the builder matches the layout as documented. It is **derived, not observed**,
   and says nothing about whether the server accepts it.
2. **Round trip through `core` (test in `crates/core`).** Parse the built frame's 6-byte prefix with
   the framing constants, then decode the payload with `protocol/decoder.rs` (fields 1, 2, 4 and the
   string at field 3). The channel and text must come back. This proves the payload is well-formed
   protobuf with the right field numbers, read by the same code that reads real chat.
3. **One real frame, observed (Kade's step, later).** Either (i) the DLL logs the exact bytes it
   passed to `Send` in a debug build, and Kade's in-game test confirms the line appeared in chat, or
   (ii) the recorder also keeps client-to-server packets (`destination_port == 5003`). Option (ii)
   changes `is_capturable` and `sanitise` and needs a fresh capture: a later task, not R0's. A frame
   the game itself sent would also show the high 32 bits of `service_uuid` that the game uses (section
   7). Only this kind proves the server accepts the frame.

`Recommended: 1 and 2` in the frame crate's first PR (R3 or R4), and 3(i) as part of Kade's M1
in-game test. 3(ii) only if 3(i) leaves a doubt.

## 5. The app <-> DLL pipe protocol

This section proposes a specification rather than only options, because nothing like it exists
yet. Options appear only where the choice is real.

### Facts

- **No pipe or IPC code exists in the app today.** The `windows-sys` features in use are only
  `Win32_Networking_WinSock` and `Win32_UI_Input_KeyboardAndMouse` (`src-tauri/Cargo.toml:53`). A named
  pipe needs `Win32_System_Pipes`, `Win32_Storage_FileSystem` (`CreateFileW`, file flags),
  `Win32_Security` and `Win32_Security_Authorization` (security descriptor from SDDL), `Win32_System_IO`
  (overlapped I/O for timeouts) and `Win32_System_Threading`. These names are the module paths of
  windows-sys 0.61 (reported, from its documentation; the follow-up PR confirms each by compiling).
  The DLL needs the same, plus `Win32_System_LibraryLoader` (`GetProcAddress`, `GetModuleHandleW`).
- **The app is elevated, and the game probably is not.** The app has `requireAdministrator`
  (`src-tauri/app.manifest:19`). The game's integrity level is NOT VERIFIED (a launcher may start it
  elevated). A pipe created by an elevated process with the default security descriptor gives full
  access only to SYSTEM, Administrators and the creator owner, and read access to Everyone (Windows'
  documented default for named pipes, reported). The creator owner of an elevated token is usually
  the Administrators group (estimated). A non-elevated game would then be unable to write to the pipe.
- **The security model's rules apply.** Another process of the same user is "not trusted"
  (`docs/security_model.md:27`). The test bridge is the model to copy: a fixed list of commands, an
  unknown one refused, loopback only, a size cap (`docs/security_model.md:103-108`; `Command` at
  `crates/core/src/bridge.rs:71`, `MAX_PACKET_BYTES` at `:45`).

### Specification (proposed)

- **Transport: a named pipe** `\\.\pipe\resonance-ingame-<game pid>`.
- **The app creates the pipe, and the DLL connects.** The app is the elevated side, owns the
  lifecycle, and knows which pid it loaded the DLL into. It creates the pipe **before** loading the
  DLL, with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so if another process already holds the name, creation
  fails and the app refuses to go on. After `ConnectNamedPipe`, the app calls
  `GetNamedPipeClientProcessId` and drops any client whose pid is not the game's. The pid check is the
  real authentication. The name only needs to be predictable for the DLL, which reads its own pid with
  `GetCurrentProcessId`.
- **Flags:** `PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE`,
  `PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS`, one instance.
- **DACL (SDDL):** `D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;<current user SID>)S:(ML;;NW;;;ME)`.
  - `D:P` is protected (no inherited entries).
  - SYSTEM and Administrators get full access.
  - The interactive user, whose SID is the same in the elevated and the filtered token, gets read
    and write, so a non-elevated game can connect.
  - The explicit Medium mandatory label (`ME`, no-write-up) makes sure a Medium-integrity game can
    write even if the object would otherwise get the creator's High label (estimated; checked on
    Kade's PC).
  - The SID comes from `GetTokenInformation(TokenUser)`.
- **What the ACL does not stop.** Any process of the same user can open the pipe. The pid check stops
  it from being served, but a same-user process could also inject into the game itself. The pipe
  carries no secret and grants nothing a same-user process does not already have: it can only ask the
  DLL to post a chat line the user could post. That is stated in `docs/security_model.md` by task R1.
- **Framing: JSON lines.** UTF-8 JSON objects, one per line, `\n`-terminated, at most **4096 bytes**
  per line. 140 UTF-16 units are at most 840 bytes even with every unit escaped as `\uXXXX`. A longer
  line is answered with `bad_request`, and the connection is closed.
- **Who speaks.** Only the app sends requests, and the DLL only replies, one reply per request, in
  order. Each request carries an `id` (u32), and the reply echoes it.
- **Requests:**

  | request | reply when it works |
  |---|---|
  | `{"op":"hello","id":1,"version":1}` | `{"id":1,"ok":true,"version":1,"dll":"0.7.0"}` |
  | `{"op":"send","id":2,"channel":"world","text":"…"}` | `{"id":2,"ok":true,"call_id":17}` |
  | `{"op":"ping","id":3}` | `{"id":3,"ok":true}` |
  | `{"op":"bye","id":4}` | `{"id":4,"ok":true}`, then the DLL removes its hooks and unloads (section 6) |

  An error is `{"id":N,"ok":false,"code":"<code>","detail":"<short English reason for the log>"}`.
- **Version rule.** The protocol `version` is an integer, starting at 1. `hello` is the first request
  on every connection. If the DLL's version differs, it replies with `version`, and the app refuses
  to send, unloads that DLL and loads its own copy. In practice the DLL is downloaded by the app, per
  version (section 6), so a mismatch means a stale DLL from an older app is still loaded.
- **Timeouts.** The whole `send_chat` command must fit in **2 s**.
  - `hello` runs at load time, not per send.
  - Per send: the request write has 100 ms. The DLL holds the request for at most **1000 ms**
    waiting for the main thread (section 2 A), then gives up with `expired`. The app waits up to
    **1500 ms** for the reply, then treats the pipe as broken.
  - The DLL decides "send" or "expired" atomically (the pump takes the slot only if it is not
    expired). So `expired` always means **not sent**, and the app never has to guess.
- **The DLL re-checks everything** (the second check): it knows the channel, the text is non-empty,
  has no CR/LF, and is at most 140 UTF-16 units, using `crates/frame`'s check (section 4).
- **Error codes.** Each has a cause in this design. The app maps each code to the Korean sentence
  shown to the user by code, never by `detail`. Where R2's `prepare_text` errors cover the same case,
  R2's sentences win.

  | code | cause | sentence shown (ko) |
  |---|---|---|
  | `bad_request` | a line that is not JSON, too long, unknown `op`, missing field | 게임 내 전송 모듈과 통신하지 못했습니다. 앱을 다시 시작해 주세요. |
  | `bad_channel` | `channel` is not one of `world`, `say`, `party`, `guild` | 이 채널로는 보낼 수 없습니다. |
  | `empty` | text empty after the DLL's check | 보낼 내용이 없습니다. |
  | `too_long` | more than 140 UTF-16 units, or a CR/LF the app should have removed | 메시지가 너무 깁니다. 140자 이하로 줄여 주세요. |
  | `no_client` | `ZRpcCtrl` null (before login), `GetRpcByProxyId` null, UDP connection, `client_` null, `handle_` zero | 게임 채팅 서버에 연결되어 있지 않습니다. 로그인한 뒤 다시 시도해 주세요. |
  | `not_sent` | `Send` raised a managed exception (returned in `runtime_invoke`'s `exc`) | 게임이 메시지를 보내지 못했습니다. |
  | `busy` | a send is already waiting for the main thread (single slot) | 이전 메시지를 보내는 중입니다. 잠시 후 다시 시도해 주세요. |
  | `expired` | the main thread did not tick within 1000 ms (loading screen) | 게임이 응답하지 않아 보내지 않았습니다. 로딩이 끝난 뒤 다시 시도해 주세요. |
  | `unsupported` | a class, method, field or export the walk needs was not found: a game build the DLL does not know (section 7) | 이 게임 버전은 아직 지원되지 않아 채팅을 보낼 수 없습니다. 앱 업데이트를 기다려 주세요. |
  | `version` | `hello` version mismatch | 게임 내 전송 모듈의 버전이 맞지 않습니다. 앱을 다시 시작해 주세요. |

  The app has its own errors outside the pipe: the setting is off, the game is not running, loading
  failed, the pipe broke, and the 1500 ms spacing. Their sentences belong to the `send_chat` command
  (R6).
- **"Sent" means handed to the game's socket.** It does not mean the server accepted the line. The
  server answers our `call_id` with a ChitChat Return (Stellar records the call to read the reply,
  `…/Game/PandaChatProbe.Send.cs:235-236`), which the sniffer could read later. That is out of M1.

### Options where the choice is real

- **Transport. A: named pipe** (above). **B: loopback TCP.** B needs a port, may raise a firewall
  prompt for the game, and has no ACL or client-pid check (any local process can connect).
  `Recommended: A`, because Windows ACLs and `GetNamedPipeClientProcessId` give real access control at
  no cost.
- **Framing. A: JSON lines** (above, needs `serde_json` in the DLL, already in the workspace through
  `crates/core`). **B: length-prefixed binary**, no dependency, but every reader and log needs a decoder.
  `Recommended: A`. It can be read in a log, matches the bridge's style, and its cost is one crate the
  workspace already builds.

## 6. Loading and unloading

### Facts

- **The app is Administrator** (`src-tauri/app.manifest:19`). It can open the game with
  `PROCESS_CREATE_THREAD | PROCESS_VM_OPERATION | PROCESS_VM_WRITE | PROCESS_QUERY_INFORMATION`
  unless the game is a protected process or an anti-cheat driver strips the handle's rights. Stellar's
  docs imply the game has no anti-cheat today: they tell users to stop using the framework if a future
  patch adds one (`Stellar@60a7772:docs/getting-started.md:8-9`). Reported. NOT VERIFIED for the current build.
- **The app does not find the game process today.** Capture works by adapter and port, not by
  process: `find_game_interface` picks the routed adapter
  (`src-tauri/src/services/sniffer/network.rs:203-213`, `route_source_ip` at `:176-184`), and the
  pipeline keeps port 5003 (`crates/core/src/capture/pipeline.rs:83`). The only processes the app
  starts are `netsh` (`network.rs:255`, `:299`, `:322`), `explorer` (`src-tauri/src/io/fs.rs:20`),
  `llama-server.exe`, and its own updated exe (`src-tauri/src/services/downloader/app_updater.rs:145`).
  The repository contains no game exe name.
- **There is more than one game exe name.** Stellar matches the exe name by **prefix**: `StarSEA`
  is the SEA client (`StarSEA.exe` from the launcher, `StarSEA_STEAM.exe` on Steam) and `StarASIA` is
  the JP client (`Stellar@60a7772:src/Stellar.Application/Services/GameEnvironmentService.cs:19-32`,
  `src/Stellar.Abstractions/Domain/GameRegion.cs:8`, `FrameworkVersion.cs:374-375`). Reported. This
  app translates Japanese chat, so many of its users may run `StarASIA.exe`. The app must not
  hard-code `StarSEA.exe`.
- **New `windows-sys` features** (reported module paths in 0.61; the follow-up confirms each by
  compiling):

  | feature | for |
  |---|---|
  | `Win32_System_Diagnostics_ToolHelp` | process and module snapshots |
  | `Win32_System_Threading` | `OpenProcess`, `CreateRemoteThread`, `WaitForSingleObject`, `IsWow64Process2` |
  | `Win32_System_Memory` | `VirtualAllocEx`, `VirtualFreeEx` |
  | `Win32_System_Diagnostics_Debug` | `WriteProcessMemory` |
  | `Win32_System_LibraryLoader` | `GetModuleHandleW`, `GetProcAddress` for `LoadLibraryW` |
  | `Win32_NetworkManagement_IpHelper` | `GetExtendedTcpTable` |

  Each is a `src-tauri/Cargo.toml` change, which is not R0's to make.
- **Rule 1 of the security model:** "What runs elevated is verified when it is run" (hash or signature
  of the file about to be run, `docs/security_model.md:126`). `resonance_core::server_pins` does this
  for the AI server just before starting it (`src-tauri/src/services/translator/server_manager.rs:139-145`).
  The security model already names the gap between that check and the start: a same-user process can
  replace a file in a user-writable folder (`docs/security_model.md:66`).
- **A remote thread's exit code is 32 bits.** `GetExitCodeThread` returns a `DWORD`, so the `HMODULE`
  that `LoadLibraryW` returns in the game is truncated on x64 (documented Win32 behaviour, reported).
  The module base must be read from a module snapshot (`TH32CS_SNAPMODULE`).

### Options: loading

- **A. Remote thread (`LoadLibraryW`).** `OpenProcess` -> `VirtualAllocEx` + `WriteProcessMemory` (the
  DLL path as UTF-16) -> `CreateRemoteThread(LoadLibraryW, path)` -> `WaitForSingleObject` (5 s) ->
  check the module snapshot for `resonance_ingame.dll` -> `VirtualFreeEx`. This is the standard,
  documented way. It writes nothing into the game's folder, and loading happens only while the app
  runs and the setting is on.
- **B. Proxy DLL in the game folder** (the Stellar way: BepInEx IL2CPP through a doorstop
  `winhttp.dll`, `GameEnvironmentService.cs:21-22`). The game loads it at start-up, so no remote
  thread is needed. But it changes the game's install folder, loads before the app runs (and even
  when the setting is off), must be removed by hand or by the app, and a launcher repair may delete
  or flag it.
- **C. Manual map** (write the PE image and resolve imports by hand). It leaves no loader entry, which
  is what anti-cheat looks for, and it breaks Rust's TLS and unwinding tables unless all of that is
  redone by hand. Not recommended.

`Recommended: A`. It is the standard, it leaves the game's folder untouched, and it follows the
opt-in exactly (no setting, no DLL).

### Specification for A

- **Finding the pid.** A process snapshot (`CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS)`) keeps exe
  names that start with `StarSEA` or `StarASIA` and end in `.exe`, case-insensitive.
  - One match: use it.
  - More than one: keep the one that owns a TCP connection to remote port 5003
    (`GetExtendedTcpTable`, `TCP_TABLE_OWNER_PID_CONNECTIONS`). This also confirms that the process is
    the one the sniffer hears.
  - Still more than one, or none: refuse, with "게임이 실행 중이 아닙니다" or "게임이 둘 이상 실행 중입니다".
  - A 32-bit process (`IsWow64Process2`) is refused.
  - A game that is running but not logged in is fine to load into. Sends answer `no_client` until
    login.
- **When to load.** When the setting is on and the game process is found (on enable, and at the
  first send if it is not loaded yet). The DLL stays loaded until the setting is turned off, the app
  exits, or the game exits.
- **Checking the DLL is ours.** The DLL is downloaded by the app on opt-in: when the setting is turned
  on, at start-up with it on, and when a send finds no file (that send answers `DllDownloading`, a
  sentence asking the user to try again). It is kept per version as
  `%APPDATA%\com.enjay.bpsr.resonance-stream\bin\ingame\<version>\resonance_ingame.dll`
  (`<app data>/bin/ingame/<version>/` in this document), never beside the exe; there is no installer bundle. Its SHA-256 is computed at build time and built into the app
  as a pin, the way `server_pins` works. A build without a pin downloads nothing and loads nothing
  (`SendError::NoPin`).
  - **Loaded in place, no copy.** The app loads that file where it is; nothing is written to the game's
    folder, and the downloaded file is not copied again. It opens the file with `FILE_SHARE_READ` (nobody else may write
    or delete it), hashes through that handle, and keeps the handle open across the remote-thread
    `LoadLibraryW`. That closes the check-to-load gap the security model names, if Windows allows
    `LoadLibraryW` on a file held that way: NOT VERIFIED on Windows.
  - **Not built: a copy as the fallback.** The design allowed one if `LoadLibraryW` hit a sharing
    violation: copy the DLL into the app's own data folder (never the game's), hash the copy, load
    only on a match. No copy exists in the code. If Windows refuses `LoadLibraryW` on the held file,
    the send fails with `LoadFailed`. NOT VERIFIED on Windows.
  - **The pin contract.** The app's build script reads `RESONANCE_INGAME_DLL` (the absolute path of the
    built DLL) and `RESONANCE_INGAME_REQUIRED` (with it set and the file missing, the app build
    fails, so a Candidate can never ship without a pin; without it the build succeeds with no pin and a
    warning); `src-tauri/build.rs` writes the hash into the app. R8's Candidate builds the DLL first,
    then the app; the same bytes are the release asset the app downloads. In a debug build whose
    `RESONANCE_INGAME_DLL` was set at build time the path is that file, loaded instead of the
    downloaded module (dev only).
  - **A signed release asset.** Releases ship a bare exe, so the DLL is its own asset next to it: signed with
    the exe's key and version comment, listed in `SHA256SUMS` with its `.sig`, carried from the prerelease to
    the stable release by Promote. The app fetches it by tag pattern, not through `latest.json` (whose
    feed names only the newest stable release): `v<version>` when its `SHA256SUMS` lists the module with the
    pin, else the newest `v<version>-rc.<N>` prerelease that does (an rc exe has the stable version, whose
    tag does not exist before Promote). The signature and the pin are checked before the folder is moved
    into place; a build without a pin downloads nothing.
  - Having the DLL check which module loaded it is not worth it: nothing in the DLL is privileged.
- **No overwrite while loaded.** A loaded DLL file cannot be replaced: the updater swaps only the exe,
  and a still-loaded old module is waited out (see an app update, below). Each version has its own
  folder; the app deletes the folders of other versions that no process has loaded (a held one is tried
  again at the next run).
- **`DllMain` does almost nothing.** On `DLL_PROCESS_ATTACH` it calls `DisableThreadLibraryCalls` and
  `CreateThread(worker)`, and returns. The worker starts after the loader lock is released, then
  attaches to il2cpp, resolves exports, and connects the pipe. No il2cpp call ever runs inside
  `DllMain`.

### Unloading

- **On request (`bye`: setting off, app exit, app update).** The DLL:
  1. stops accepting sends (and answers a queued one `expired`);
  2. removes the ilhook detour (`HookPoint::unhook`, with the DLL's `ThreadCallback` suspending the game's
     other threads around it) before the drain;
  3. waits until an in-detour counter is zero and one more frame has passed, so no thread is inside
     the DLL's code;
  4. closes the pipe;
  5. calls `FreeLibraryAndExitThread` from its own worker thread.

  The app waits for the module to leave the module snapshot (2 s). If the unhook cannot be proven safe
  (a thread is in the patched window, the trampoline or the DLL's code), the DLL does not unload: it
  stays loaded (`KeepResident`, R5), closes its pipe and logs it. The app treats "module still in the
  snapshot after 2 s" as that case, never as a crash.
- **The app is killed first.** The pipe breaks. The DLL runs the same sequence and unloads itself. A
  fresh app loads the DLL again. This avoids a DLL left behind that no app can talk to.
- **The game is closed first.** The pipe breaks and the game's process handle is signalled. The app
  marks the DLL unloaded, and the next send finds no game.
- **An app update.** The old app sends `bye` on exit. The new app downloads its own version's DLL if it is not there yet and loads it again, so two versions are never loaded at once. If an old copy is found in the module
  snapshot (the old app crashed during the exchange), the app waits for its self-unload (it sees the
  broken pipe) before loading.

### Crash safety

- **`panic = "unwind"` for the DLL** (section 4) and `catch_unwind` at every FFI edge: `DllMain`, the
  worker thread's procedure, every detour callback, and any exported function. A caught panic in the
  worker turns into an error reply. A caught panic in a detour calls the original and disables the
  feature. No panic ever crosses `extern "C"`: since Rust 1.81 that aborts anyway (reported).
- **The detour's hot path** (the per-frame pump) does one atomic load when idle. It never allocates,
  never takes a lock, and never logs on that path.
- **Faults in game code are not panics.** An access violation from reading a freed game object is a
  structured exception, and `catch_unwind` does not catch it. `AddVectoredExceptionHandler` could see
  it, but it is process-wide, sees every exception the game raises, and recovering means unwinding
  game frames by hand. Not worth the risk. The default is to **refuse to call** whenever the liveness
  check fails (`handle_` zero, any step null), which is what Stellar relies on too.

## 7. Failure and patch resilience

### Facts

- **Every step can go missing after a patch.** Stellar's design note says the framework supports only one
  game version at a time and reruns recon after each patch
  (`Stellar@60a7772:docs/architecture.md:168`). Reported. The names in section 1 come from Stellar's
  release_3.7 dump (`…/Game/PandaChatProbe.Send.LiveClient.cs:30`).
- **TCP only.** The fixed mechanism needs the chat connection to be TCP. The World channel can run
  over UDP/KCP (`…/Game/PandaChatProbe.Send.Hooks.Rpc.cs:28-31`). `ZRpcImpl.SendBytes` sits above the
  TCP/UDP split (`:31-33`) and is the transport-agnostic alternative. It takes the protobuf body and
  frames it itself, but sending through it means calling a game method with a `byte[]` from Rust: a
  possible M2 task, not M1.
- **Two wire differences from the game's own sends** (risks, not known failures):
  - The game's own ChitChat `service_uuid` has non-zero high 32 bits (a per-stub id), while Stellar's
    frame sends the bare `1321197368` with a zero high half
    (`…/Game/PandaChatProbe.Send.Hooks.Rpc.cs:10-14`, `…/Game/PandaChatProbe.Send.Builders.cs:154-155`).
  - Stellar's `call_id` counter starts at 1 and is separate from the game's own counter
    (`…/Game/PandaChatProbe.Send.cs:87-91`), so our call and one of the game's can carry the same id,
    and the server's Return for ours may reach the game's pending-call table.

  Stellar sends this way in its releases, which suggests the server accepts it (reported, not
  observed here). Mitigation for the second: start the DLL's counter at a random value in the upper
  half of `u32`, far from the game's own ids (a placeholder for R3/R4). Kade's M1 test watches for a
  wrong reply in the game's chat UI.
- **How the DLL learns the game build.** From the exe's file version (`GetFileVersionInfoW`) and the
  size and timestamp of `GameAssembly.dll`, which both change with every patch. Stellar parses a
  `release_<ver>` folder name, which the Steam build does not have
  (`Stellar@60a7772:src/Stellar.Application/Services/GameEnvironmentService.cs:26-27`).

### What fails, and what each layer does

The DLL never crashes the game. Every step returns a typed result, nothing is called unless every
earlier step resolved, and resolution failures are cached per game build.

| what goes missing | DLL does | pipe reply | user sees (by code) | kind |
|---|---|---|---|---|
| game not running | (not loaded) | (app side) | 게임이 실행 중이 아닙니다 | self-healing: next send |
| `GameAssembly.dll` export not found | stays loaded, sends refused | `unsupported` | `unsupported` sentence | permanent for this build |
| class not found (`ZRpcCtrl`, `ZRpcImpl`, `ZRpcTransfer`, `ZTransport`, `ZTcpConnection`, `ZTcpClient`) | stops resolving | `unsupported` | `unsupported` sentence | permanent for this build |
| method not found, or wrong parameter count (`GetRpcByProxyId(ulong)`, `GetTransfer()`, `GetTransport()`, one-argument `Send`) | stops resolving | `unsupported` | `unsupported` sentence | permanent for this build |
| `Send`'s parameter is no longer `ReadOnlySpan<byte>` | stops resolving | `unsupported` | `unsupported` sentence | permanent for this build |
| field not found (`conn_`, `client_`, `handle_`) | stops resolving | `unsupported` | `unsupported` sentence | permanent for this build |
| field offsets moved | nothing: offsets come from `il2cpp_field_get_offset` every build | - | - | handled |
| singleton `null` (before login) | no call | `no_client` | `no_client` sentence | self-healing: after login |
| `GetRpcByProxyId` returns `null` | no call | `no_client` | `no_client` sentence | self-healing (if it persists after login: Kade's test, then section 1 B) |
| `conn_` is not `ZTcpConnection` (UDP/KCP) | no call | `no_client` (`detail: "udp"`) | `no_client` sentence | self-healing if the game moves back to TCP; else M2 (`SendBytes`) |
| `client_` `null` or `handle_` zero | no call | `no_client` | `no_client` sentence | self-healing: after reconnect |
| `Send` raises a managed exception | caught by `runtime_invoke` | `not_sent` | `not_sent` sentence | retry allowed |
| main thread does not tick in 1000 ms | send dropped, never made | `expired` | `expired` sentence | self-healing |
| hook install fails (first bytes not relocatable) | feature off, nothing patched | `unsupported` | `unsupported` sentence | permanent for this build |
| DLL already loaded (old copy) | (app side) waits for its self-unload, then loads | - | - | handled |
| DLL loaded twice (same file) | `LoadLibraryW` returns the same module: refcount only | - | - | handled: the app checks the snapshot first and never loads twice |

- **Permanent failures are reported once per game build.** The DLL logs the missing name and the
  build key (exe file version plus `GameAssembly.dll` size and timestamp). Every later send gets
  `unsupported` without walking again, until the build key changes.
- **Logging.** The DLL writes `%LOCALAPPDATA%\<app data folder>\ingame\ingame.log`, capped at 1 MiB
  (one rotated file). Each resolution failure gets one line with the step name. Chat text is never
  logged, only its length and channel. The app logs each pipe error code with `detail`.
- **No allow-list of game builds.** Refusing an unknown build would turn every patch day into an
  outage for a feature that resolves everything by name anyway. The DLL logs the build key and relies
  on structural resolution. This is a placeholder choice, open to Kade if he wants a stricter one.

`Recommended: the table above`. Resolve everything by name each build, report permanent failures
once with the step name, retry self-healing ones on the next send, and refuse to call on any doubt.
It is the long-term option because it survives patches that move offsets, and it fails safely on
patches that rename things.

**Patch-day check (Kade's step).** After a game patch, with the setting on and logged in: send one
line each to `world`, `say`, `party` and `guild` (a party and a guild are needed for the last two),
and check that each arrives once. Then read `ingame.log` for any `unsupported` line, and include its
step name in the bug report. The same check is the M1 acceptance run.

## Questions for Kade

Four choices are truly his: a new dependency, a new crate layout, an unknown runtime risk, and how
far the app reaches into the game. Each is answered by picking a letter.

1. **Which thread sends (section 2)?**
   - **A. Hook-and-pump on the game's main thread (Recommended).** It sends on the same thread as the
     game's own chat, so no unknown locking is relied on. Costs one hook and one dependency (question 2).
   - B. Attach the DLL's pipe thread to il2cpp and call `Send` from it. No hook and no new dependency,
     but a data race with the game's net code is possible and cannot be ruled out by a test.
2. **Which detour crate, if 1 is A (section 3)?**
   - A. `minhook` 0.9.0. The standard tool for this job: MIT, the smallest
     dependency tree, and it suspends threads while patching.
   - B. `retour` 0.3.1. Pure Rust apart from the disassembler, but no stable release since 2023.
   - C. Hand-rolled patch and trampoline. No dependency, but a few hundred lines of crash-critical
     code that only we test.
   - **D. `ilhook` 2.3.0 (Recommended).** Pure Rust, no C build step; the DLL suspends the game's other
     threads itself, because none of the pure-Rust crates documents doing it.
3. **Where do the frame builder and the DLL crate live (section 4)?**
   - **A. Two new crates in the workspace (Recommended):** `crates/frame` (pure, no dependencies) and
     `crates/ingame` (the DLL), with a custom `[profile.ingame]` that sets `panic = "unwind"`. One
     lockfile, and every frame is checked against `core`'s parser.
   - B. `crates/frame` in the workspace, and the DLL in its own workspace outside it, with its own
     lockfile and `panic = "unwind"`.
   - C. The DLL in the workspace with `panic = "abort"` and no panics allowed. Any panic, including
     one in a dependency, kills the game.
4. **How does the app load the DLL (section 6)?**
   - **A. From the running app, by remote thread (`LoadLibraryW`) (Recommended).** Nothing is written
     to the game's folder, and the DLL is loaded only while the setting is on. It works for the
     `StarSEA*` and `StarASIA*` exes alike.
   - B. A proxy DLL (`winhttp.dll` or `version.dll`) placed in the game's folder, loaded by the game at
     start-up. No remote thread, but it changes the game's install and loads even when the setting is
     off.

## Summary

| section | recommendation | rests on |
|---|---|---|
| 1. Live `ZTcpClient` | A: active walk, `il2cpp_runtime_invoke`, `il2cpp_field_get_offset`, no hook | - |
| 2. Thread | A: hook-and-pump on the main thread; hook point `ZRpcCtrl.Update` (NOT VERIFIED) | Question 1 |
| 3. Detour crate | D: `ilhook` 2.3.0 (pure Rust; the DLL suspends other threads itself), needed only if question 1 is A | Questions 1, 2 |
| 4. Frame builder | A: `crates/frame`, no dependencies, `no_std` | Question 3 |
| 4. DLL crate | A: in the workspace, custom `ingame` profile with `panic = "unwind"` | Question 3 |
| 4. Goldens | derived bytes + round trip through `core` now, one observed frame in Kade's test | - |
| 5. Pipe | named pipe created by the app, user-only DACL with Medium label, client-pid check, JSON lines, ten error codes | - |
| 6. Loading | A: remote thread; pid by `StarSEA`/`StarASIA` prefix and the port-5003 owner; build-time SHA-256 pin; loaded in place with the file held open (no copy: not built) | Question 4 |
| 6. Unloading | `bye` -> ilhook unhook -> drain -> `FreeLibraryAndExitThread`; `KeepResident` if the unhook is not provably safe; self-unload on a broken pipe | Question 1 (hooks) |
| 7. Resilience | resolve by name each build, report permanent failures once, no build allow-list | - |

## Not verified

Only a run in the game on Kade's PC can show these. No agent can run the game.

- `GameAssembly.dll` of the current build exports each il2cpp function in section 1 by name.
- The class, method and field names of section 1 still exist in the current build (they come from
  Stellar's release_3.7 dump), and `ZRpcCtrl`'s parent is the inflated `ZSingleton<ZRpcCtrl>` with a
  static `get_Instance`.
- `GetRpcByProxyId(1321197368)` resolves at fresh login and after a reconnect.
- The chat connection is TCP (not UDP/KCP) for every M1 channel, in particular World.
- **The `ReadOnlySpan<byte>` ABI.** Through `il2cpp_runtime_invoke`, a value-type argument is passed
  as a pointer to the struct. A `ReadOnlySpan<byte>` is a 16-byte struct (a pointer or by-ref field
  and an `int` length). On Win64 a 16-byte struct is passed by reference to a caller-made copy
  (Microsoft x64 calling convention, reported), which matches passing a pointer. Unverified: how this
  il2cpp build lays out the span (field order, whether the reference is a raw pointer into a managed
  `byte[]` that must stay pinned or alive across the call), and whether the bytes must live in a
  managed array (`il2cpp_array_new`) rather than Rust memory.
- What `ZTcpClient.Send` returns and whether it can fail without an exception.
- Whether `ZTcpClient.Send` is safe from a foreign thread (only matters for question 1 B).
- The name of a per-frame main-thread method for the pump (the candidates are `ZRpcCtrl.Update` and `Panda.Core.Game.Update`), and that a native detour on it is not
  bypassed by inlining.
- `LoadLibraryW` loading a file the app holds open with `FILE_SHARE_READ` (no copy fallback is built: if not, the send fails with `LoadFailed`).
- The game's integrity level (elevated or not), and whether the pipe's DACL and Medium label let it
  connect.
- That the game has no anti-cheat that blocks `OpenProcess`, `CreateRemoteThread` or code patching
  (Stellar's docs say none today: reported).
- That the server accepts a frame whose `service_uuid` high half is zero and whose `call_id` comes
  from our own counter, and that the game's UI does not react to the server's Return for our call.
- The `windows-sys` 0.61 feature names in sections 5 and 6 (checked by compiling in the follow-up).
- Every crate fact marked "reported" in section 3 (thread suspension, behaviour on non-relocatable
  prologues), read from the crates' own documentation, not tested.

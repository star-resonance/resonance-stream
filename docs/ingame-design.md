# In-game sender: design options

**Status:** options for Kade, nothing decided. Task R0, star-resonance/tasks#13 (lane chat-send,
tasks#1, milestone M0). Written 2026-11-15. No code was written, and nothing was built or run in
the game for this document.

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
own docs say it is read-only. The Rust DLL reimplements the one send path it needs (DS-0001: "Don't
use Stellar plugin itself, just analyze feature").

## Fixed decisions (not options)

These were decided in DS-0001 (tasks#1, comment 6898425349) and in the lane's shared values (plan
`plans/chat-send/R0.md`, tasks@ce0f6f4). This doc builds on them and does not reopen them.

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
- **Text.** Trim, then turn each CR and each LF into one space. Empty text is an error. The limit is
  140 UTF-16 code units (`encode_utf16().count()`), counted after that preparation. `prepare_text` and
  its errors live in `crates/types` (DS-0002).
- **Spacing.** The app refuses a send that comes less than 1500 ms after its last successful send.
  The check is pure and takes an explicit `now`. `SendSpacing` lives in `crates/core`.
- **Errors.** Each error reaches the ui as one Korean sentence, the `Err(String)` of the `send_chat`
  command (ipc, task R6: `SendChat = "send_chat" { channel: SendChatChannel, text: String } -> ();`).

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

- **Stellar sends from the Unity main thread.** Its `IChat` contract raises the not-sent result
  "before this returns, when called on the main thread"
  (`Stellar@60a7772:src/Stellar.Abstractions/Services/IChat.cs:18-21`). `ChatService.Send` calls
  `TrySend` on the caller's thread (`src/Stellar.Application/Services/ChatService.cs:96-114`). The
  framework's tick is a Unity MonoBehaviour driven by `InvokeRepeating`
  (`…/Unity/StellarTicker.cs:7-8`, `:17`, `:41-45`). Confirmed.
- **The game sends chat from the main thread too.** The `ZRpcImpl.SendBytes` prefix runs on the
  thread that issued the RPC, which is the Unity main thread for Lua-issued chat
  (`…/Game/PandaChatProbe.Send.Hooks.Rpc.cs:98-100`). Reported. `SendBytes` "may run on a net thread" in
  general (`…/Game/PandaChatProbe.Send.Connection.cs:94-96`).
- **Nothing in Stellar shows `Send` called from a foreign thread.** Whether `ZTcpClient.Send` (a
  wrapper over a native handle) is safe to call while the main thread uses the same connection is
  unknown. NOT VERIFIED.
- **il2cpp needs an attached thread.** A thread that touches managed objects or calls
  `il2cpp_runtime_invoke` must first call `il2cpp_thread_attach(il2cpp_domain_get())`, so the GC knows
  it exists. Stellar does this for a pool thread (`…/Rendering/UnityFrameGrabber.Capture.cs:112`). A
  pipe thread in the DLL is a foreign thread.
- **A per-frame hook costs little only when it is native.** Stellar deliberately does not hook
  `Game.Update`, because a per-frame Harmony postfix crosses into the managed runtime every frame,
  which it measured as a "~12-18 fps tax" (`Stellar@60a7772:src/Stellar.Host/BootstrapPlugin.cs:42-45`).
  A Rust detour is native code: per frame it costs one atomic load and a branch when the queue is
  empty (estimated: nanoseconds). Stellar's own UI work hooks a game `Update` path the same way
  (`…/Game/GamePressCheckShield.cs:7-19`). The `StellarTicker` idea itself (an injected MonoBehaviour)
  is not open to a Rust DLL without a type injector.

### Options

- **A. Hook-and-pump on the main thread.** The pipe thread only parses and queues (a single-slot
  queue: one send in flight). A detour on a method the game runs on the main thread every frame
  drains the slot and runs section 1's walk and `Send` there, on the same thread the game sends its
  own chat from. The candidate is the `Update` of the game's net loop owner (`ZRpcCtrl` runs "the net
  echo/Update loop", `…/Game/PandaChatProbe.Send.LiveClient.cs:54-56`), or a Unity per-frame entry if that
  method does not exist. The exact method is named from a dump of the current build: NOT VERIFIED.
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

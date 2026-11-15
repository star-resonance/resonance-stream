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

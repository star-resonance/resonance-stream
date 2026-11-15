# Roadmap: chat sending through an in-game DLL (2026-11-16)

Status: **M0 in progress** (R0 design, R1 docs, R2 text rules: wave 1). Nothing is built.

Decision: D-35 in `docs/decisions.md`. Design: `docs/ingame-design.md` (planned, R0 tasks#13).
Lane and approvals: star-resonance/tasks#1 (DS-0001, DS-0002) and the roadmap on tasks#2.
Threats: `docs/security_model.md` section 4, T-10 to T-12.

The app (already Administrator) loads its own Rust DLL `resonance_ingame.dll` (planned) into the game;
the DLL calls the game's `ZTcpClient.Send` on the live chat connection. It is opt-in, off by default,
with a terms-of-service warning. No Stellar, no keystrokes.

## M0. Design (wave 1)

The R0 options doc answers, with file:line facts: how the DLL finds the live chat `ZTcpClient`; the thread
`Send` runs on; the detour crate (a new dependency: Kade's call); where the frame builder lives; the pipe
protocol; load and unload; DLL integrity; app updates while the DLL is loaded. Plus D-35 and the security
model (R1) and the send text rules (R2, tasks#15).

**Done when:** `docs/ingame-design.md`, D-35 and the text rules in `crates/types` / core are merged and Kade
has read the options doc.

## M1. Send chat (wave 2)

Opt-in setting (off by default, terms-of-service warning); the app loads `resonance_ingame.dll`; the composer
sends to World, Guild, Party and Local (`say`), at most 140 UTF-16 units, 1.5 s apart; the DLL builds the frame
(ChitChat service `1321197368`, `SendChitChatMsg` method `1`) and calls `ZTcpClient.Send`.

**Done when:** on Windows a message reaches each of the four channels (Kade's step; CI cannot run the game).

## M2. Server verdict

Hook `ZRpcImpl.SendBytes` and correlate `call_id` with the Return; the composer shows sent / too frequent /
cooldown; the DLL is built in Candidate and bundled in the installer.

**Done when:** a send during the cooldown shows it; the Candidate run's assets hold the DLL; the installer
contains it.

## M3. More

Whispers, stickers, Korean -> Japanese draft translation before sending.

**Done when:** a whisper reaches the named peer; a sticker is sent; the draft is translated before sending
(each Kade's check on Windows).

## Facts with a number

- 140 UTF-16 code units per message, counted after preparation (trim, line breaks become spaces).
- 1500 ms between successful sends.
- Channels, in tab order: World, Guild, Party, Say (Say is the game's Local, label 로컬).
- ChitChat service 1321197368, `SendChitChatMsg` method 1.

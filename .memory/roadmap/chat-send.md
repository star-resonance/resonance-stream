# Roadmap: chat sending through an in-game DLL (2026-11-16)

Status: **M0 design merged** (R0 #398, R1 #396; R2's text rules are in `crates/types`: `SendChatChannel`); wave 2 (R3 to R9) planned; nothing built.

Decision: D-35 in `docs/decisions.md`. Design: [`docs/ingame-design.md`](../../docs/ingame-design.md).
Lane decisions: DS-0001 to DS-0004 (recorded in D-35).
Threats: `docs/security_model.md` section 3.8 and section 4, T-10 to T-13.

The app (already Administrator) loads its own Rust DLL `resonance_ingame.dll` (planned) into the game;
the DLL calls the game's `ZTcpClient.Send` on the live chat connection. It is opt-in, off by default,
with a terms-of-service warning. No Stellar, no keystrokes.

Design choices (DS-0003): hook-and-pump on the game's main thread; `ilhook` 2.3.0 (pure Rust; the DLL suspends
the game's other threads itself); `crates/frame` and `crates/ingame` in the workspace with a custom
`panic = "unwind"` profile; loading by remote thread.

## M0. Design (wave 1)

The R0 options doc answers, with file:line facts: how the DLL finds the live chat `ZTcpClient`; the thread
`Send` runs on; the detour crate (a new dependency: Kade's call); where the frame builder lives; the pipe
protocol; load and unload; DLL integrity; app updates while the DLL is loaded. Plus D-35 and the security
model (R1) and the send text rules (R2).

**Done when:** `docs/ingame-design.md`, D-35 and the text rules in `crates/types` / core are merged and Kade
has read the options doc.

## M1. Send chat (wave 2)

Opt-in setting (off by default, terms-of-service warning); the app loads `resonance_ingame.dll`; the composer
sends to World, Guild, Party and Local (`say`), at most 140 UTF-16 units, 1.5 s apart; the DLL builds the frame
(ChitChat service `1321197368`, `SendChitChatMsg` method `1`) and calls `ZTcpClient.Send`. The Candidate run
builds the DLL first, the app pins its SHA-256, and the DLL is signed like the exe and listed in
`SHA256SUMS`, and the prerelease and the stable release carry it; the app loads it in place from `<exe dir>/ingame/`.

**Wave 2 tasks:**
- R3 frame builder
- R4 DLL skeleton and pipe
- R5 DLL send
- R6 app loader and `send_chat`
- R7 composer
- R8 CI and Candidate build (the DLL is built first, pinned into the app and attached to the prerelease only,
  outside `SHA256SUMS`)
- R9 agent rules

**Done when:** on Windows a message reaches each of the four channels (Kade's step; CI cannot run the game);
the Candidate run builds the DLL, the app pins it, and the prerelease holds the DLL asset.

## M2. Server verdict

Hook `ZRpcImpl.SendBytes` and correlate `call_id` with the Return; the composer shows sent / too frequent /
cooldown. The stable-release path for the DLL is split: the release path (signed, listed, carried by Promote,
announced in `latest.json`) is F-R8a-1; the updater fetching it is F-R8a-2; the opt-in download for an exe without it is
F-R8a-3.

**Done when:** a send during the cooldown shows it.

## M3. More

Whispers, stickers, Korean -> Japanese draft translation before sending. No task yet.

**Done when:** a whisper reaches the named peer; a sticker is sent; the draft is translated before sending
(each Kade's check on Windows).

## Facts with a number

- 140 UTF-16 code units per message, counted after preparation (trim, line breaks become spaces).
- 1500 ms between successful sends.
- Channels, in tab order: World, Guild, Party, Say (Say is the game's Local, label 로컬).
- ChitChat service 1321197368, `SendChitChatMsg` method 1.
- Placeholders, to be settled by R4 and R6: line cap 4096 bytes (placeholder); write 100 ms (placeholder); DLL hold
  1000 ms (placeholder); app wait 1500 ms (placeholder); the whole `send_chat` within 2 s (placeholder).

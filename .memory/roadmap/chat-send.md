# Roadmap: chat sending through an in-game DLL (2026-11-16)

Status: **M0 design merged** (R0 #398, R1 #396; R2's text rules are in `crates/types`: `SendChatChannel`); **M1 done**
(wave 2 and its follow-ups are merged; Kade confirmed it in game on v0.6.5-rc.4 and rc.5); several clients: F-MC-1
and F-MC-2 merged, F-MC-3 and F-GR open; **M2 next**: its options doc R-M2 first.

Decision: D-35 in `docs/decisions.md`. Design: [`docs/ingame-design.md`](../../docs/ingame-design.md).
Lane decisions: DS-0001 to DS-0004 (recorded in D-35); DS-0006 (where the DLL lives), DS-0007 and DS-0009 item 2 (several clients).
Threats: `docs/security_model.md` section 3.8 and section 4, T-10 to T-13.

The app (already Administrator) loads its own Rust DLL `resonance_ingame.dll` into the game;
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

## M1. Send chat (wave 2): done

Opt-in setting (off by default, terms-of-service warning); the app loads `resonance_ingame.dll`; the composer
sends to World, Guild, Party and Local (`say`), at most 140 UTF-16 units, 1.5 s apart; the DLL builds the frame
(ChitChat service `1321197368`, `SendChitChatMsg` method `1`) and calls `ZTcpClient.Send`.

The DLL's release and place (DS-0006): the Candidate run builds the DLL first, the app pins its SHA-256, and the
DLL is signed like the exe and listed in `SHA256SUMS`; the prerelease and the stable release carry it (`latest.json`
names it, which the app does not read). The installer and the app bundle carry no DLL (F-R8c). The DLL lives per
version in `%APPDATA%\com.enjay.bpsr.resonance-stream\bin\ingame\<version>\resonance_ingame.dll`; when the user
turns the setting on (and at start-up with it on, and when a send finds no file) the app downloads it from its
release, checks the signature and the exe's pinned hash, and loads it from there. Other versions' folders are removed.

**Wave 2 tasks (all merged):** R3 #400 frame builder; R4 #401 DLL skeleton and pipe; R5 #404 DLL send; R6 #403 app loader
and `send_chat`; R7 #406 composer; R8 #405 CI and Candidate build; R9 #402 agent rules.
**Follow-ups (merged):** F-R5 #408, F-R5b #409 (the pump hook); F-R6a #410, F-R6b #411 (loader); F-R8b #413 (static C
runtime); F-R8a-1 #412 (signed DLL in the release), F-R8a-2 #417 (download), F-R8c #421 (no DLL in the installer);
F-R6c #425 (agent rules); F-R2 #422 (control characters); F-TAB #423 (the picker starts on the last channel).

**Done when:** on Windows a message reaches the game's chat (Kade's step; CI cannot run the game). **Met:** Kade
confirmed in game that sending works on v0.6.5-rc.4, and on v0.6.5-rc.5 that the app's own DLL download works.

## Several game clients (DS-0007, DS-0009)

Done: F-MC-1 #414 (one in-game session per pid, an optional send target; with several clients and no target the
composer's answer is a Korean "choose a client" sentence); F-MC-2 #424 (`game_clients`: each client named by its
character, "이름 확인 중" while unknown).
Open: F-MC-3, the composer's nickname picker (shown only with 2 or more clients, never a pid or ip:port; duplicate
names get " (1)", " (2)" in pid order; each client has its own 1500 ms spacing); F-GR, the guild relay with one
`own_player` per connection.

**Done when:** with two clients the composer picks the character by nickname and the guild relay does not flap
(each Kade's check on Windows).

## M2. Server verdict

Hook `ZRpcImpl.SendBytes` and correlate `call_id` with the Return; the composer shows sent / too frequent /
cooldown. Next: R-M2, an options doc for the hook and the `call_id` correlation; no build task before Kade has read it.

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

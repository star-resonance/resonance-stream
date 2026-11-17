---
paths:
  - "crates/**"
---

# Layout (crates)

What each file does; read the file or `rg` a symbol to go deeper.

```
crates/core/           resonance-core — pure logic, tested on any OS
  src/protocol/         port 5003: stream framing (framing.rs) + protobuf-style decoding; rpc_return.rs: the ChitChat Return (call_id, cd_end_time, err_code)
  src/capture/          ChatPipeline: raw IPv4/TCP bytes → dedup/blocked ChatMessages
  src/text.rs           translation pre/post-processing, Dictionary, emotes, romaji
  src/history.rs        ChatHistory (backend chat log) + load_recent (daily chat_logs reload)
  src/flow.rs           a chat line from capture to translation: dispatch + job behind a Sink (app and llama harness run it)
  src/workers.rs        worker decisions: translator on/off/restart, stale jobs, port
  src/download.rs       download checks: HTTPS, length + SHA-256, progress, versions
  src/sniffer_net.rs    sniffer network setup: per-exe firewall rule name, adapter pick (route first)
  src/send_text.rs      SendSpacing: 1500 ms between sends (the text rules are in types)
  src/ingame_send.rs    chat sending's pure logic: pick the game, pin check, pipe sentences, connect, send_chat_flow, which waits for the verdict (src-tauri/src/ingame/ runs it)
  src/send_verdict.rs   VerdictMatcher: a send's Return matched by call_id within VERDICT_WAIT (1000 ms) -> Verdict
  src/send_cooldown.rs  CooldownGate: the server's chat cooldown per (client, channel); checked before a send
  src/send_hub.rs       VerdictHub: matcher + gate + endpoint-to-client route + the wait; From<Verdict> for SendVerdict
crates/frame/          resonance-frame — SendChitChatMsg frame builder, no_std, no dependencies, any OS
crates/llama/          resonance-llama — llama-server HTTP client (/completion, /health), any OS
crates/ingame/         resonance-ingame — the in-game DLL (cdylib, Windows; rules: .claude/rules/ingame.md)
crates/types/          resonance-types — DTOs shared across the Tauri boundary (serde, serde_json)
  src/send_chat.rs      SendChatChannel, prepare_text, SendTextError: what the composer may send (ui and app share it)
  src/ingame_pipe.rs    the app-DLL pipe: names, ErrorCode, Request, LineBuffer
```

## Chat send verdict

- A Return is matched to a send by `call_id` within `VERDICT_WAIT` (1000 ms, Kade's value); none in time is `NotConfirmed`.
- The cooldown is per client and channel: the gate answers `Cooldown` before a send and is armed by a Return's `cd_end_time`.
- `rpc_return`, `send_verdict`, `send_cooldown` and `send_hub` are pure: `now` is passed in, no OS calls; the wiring stays in `src-tauri/src/ingame/`.
- The Return's layout and the 5211 cooldown code are NOT VERIFIED against a real capture; a Return has no service or method.

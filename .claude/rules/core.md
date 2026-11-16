---
paths:
  - "crates/**"
---

# Layout (crates)

What each file does; read the file or `rg` a symbol to go deeper.

```
crates/core/           resonance-core — pure logic, tested on any OS
  src/protocol/         port 5003: stream framing (framing.rs) + protobuf-style decoding
  src/capture/          ChatPipeline: raw IPv4/TCP bytes → dedup/blocked ChatMessages
  src/text.rs           translation pre/post-processing, Dictionary, emotes, romaji
  src/history.rs        ChatHistory (backend chat log) + load_recent (daily chat_logs reload)
  src/flow.rs           a chat line from capture to translation: dispatch + job behind a Sink (app and llama harness run it)
  src/workers.rs        worker decisions: translator on/off/restart, stale jobs, port
  src/download.rs       download checks: HTTPS, length + SHA-256, progress, versions
  src/sniffer_net.rs    sniffer network setup: per-exe firewall rule name, adapter pick (route first)
  src/send_text.rs      SendSpacing: 1500 ms between sends (the text rules are in types)
  src/ingame_send.rs    chat sending's pure logic: pick the game, pin check, pipe sentences, connect, send_chat_flow (src-tauri/src/ingame/ runs it)
crates/frame/          resonance-frame — SendChitChatMsg frame builder, no_std, no dependencies, any OS
crates/llama/          resonance-llama — llama-server HTTP client (/completion, /health), any OS
crates/ingame/         resonance-ingame — the in-game DLL (cdylib, Windows; rules: .claude/rules/ingame.md)
crates/types/          resonance-types — DTOs shared across the Tauri boundary (serde, serde_json)
  src/send_chat.rs      SendChatChannel, prepare_text, SendTextError: what the composer may send (ui and app share it)
  src/ingame_pipe.rs    the app-DLL pipe: names, ErrorCode, Request, LineBuffer
```

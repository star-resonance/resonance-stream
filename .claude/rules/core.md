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
crates/llama/          resonance-llama — llama-server HTTP client (/completion, /health), any OS
crates/types/          resonance-types — DTOs shared across the Tauri boundary (serde only)
```

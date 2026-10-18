---
paths:
  - "src-tauri/**"
---

## Conventions (app crate)

- **Tauri commands are `pub` and reach `generate_handler!` through the crate-root
  `pub use <module>::*`.** A new command module needs both. A private `use` of the same
  name in a module shadows its glob re-export (rustc warns) — the item then silently
  stops being exported; call it by path instead.

- **The test bridge (`bridge/live.rs`) never calls a `#[tauri::command]`.** A command the
  bridge also runs keeps its body in a plain function beside it that takes `&AppHandle`
  (`get_chat_history` -> `chat_history`); the command and the bridge both call that. The
  bridge's commands run on one async task, so an async function is awaited, never `block_on`.
  The test `the_bridge_calls_no_tauri_command` fails on a bridge arm that calls a command.

- **Events are emitted with `resonance_types::ipc::event::*`, never a string literal.** A new
  event gets its constant there, a place in `event::ALL` (the ui listens to each one) and an entry
  in the test `the_event_names_are_the_ones_on_the_wire`; a new command's struct also goes in
  `ipc.rs` (see `.claude/rules/ui.md`).

- **Favorites change only through `save_favorites`** (`FavoritesState`, then the
  `favorites-changed` event reaches every window); `save_config` keeps the stored ones and
  ignores the ones in its payload. A popup window (`open_popup`) saves only what it owns,
  never the whole config -- its copy of the settings may be older than the main window's.

## Updates and remote metadata

- **Remote metadata:** a public gist (`downloader/gist.rs`) carries model/dictionary
  versions and the custom dictionary; the app's own update comes from the newest stable
  release's `latest.json` (see `.claude/rules/release.md`), checked against signing keys built into
  the app. The gist's `app` entry is ignored by the app and kept only for copies that
  predate the signed updater (the 0.6.0 bridge release). Public URLs, not secrets.

## Layout (src-tauri/)

```
src-tauri/            app crate (resonance-stream, lib resonance_stream_lib)
  src/lib.rs            module list, crate-root re-exports, run() — start-up wiring only
  src/events.rs         inject_system_message / store_and_emit: emit to UI + keep history
  src/commands.rs       history + translator commands; window.rs, tray.rs, shortcut.rs
  src/protocol/types.rs AppState and backend-only types; re-exports resonance-types
  src/services/         owner.rs (Services: who runs, the one start/stop/restart) sniffer/ (sockets, workers)
                          translator/ (llama server) downloader/
  src/config/ src/io/   config + metadata persistence, archive writer
```

## Capture and translator

- **Capture:** raw socket with `SIO_RCVALL` (`windows-sys`,
  `src-tauri/src/services/sniffer/network.rs`). Needs **Administrator**. Port 5003
  carries chat. No Npcap / WinDivert.
- **Translation:** llama.cpp server (Vulkan build, downloaded at runtime from this
  repo's releases) on `127.0.0.1:8080`, or a free port if 8080 is taken; only the PID the
  app started is ever killed. Pre/post-processing in `crates/core/src/text.rs`.

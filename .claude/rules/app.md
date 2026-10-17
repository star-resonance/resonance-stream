---
paths:
  - "src-tauri/**"
---

## Conventions (app crate)

- **Tauri commands are `pub` and reach `generate_handler!` through the crate-root
  `pub use <module>::*`.** A new command module needs both. A private `use` of the same
  name in a module shadows its glob re-export (rustc warns) — the item then silently
  stops being exported; call it by path instead.

- **Favorites change only through `save_favorites`** (`FavoritesState`, then the
  `favorites-changed` event reaches every window); `save_config` keeps the stored ones and
  ignores the ones in its payload. A popup window (`open_popup`) saves only what it owns,
  never the whole config -- its copy of the settings may be older than the main window's.

## Updates and remote metadata

- **Remote metadata:** a public gist (`downloader/gist.rs`) carries model/dictionary
  versions and the custom dictionary; the app's own update comes from the newest stable
  release's `latest.json` (see the `release` skill, *Stable releases*), checked against signing keys built into
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

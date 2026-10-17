# 2026-09-29 — phase 2: crates/core and crates/types

**Impact analysis:** a caller search on every moving symbol showed
the pure code was used from only three places: `sniffer/mod.rs` (`ChatPipeline`,
`PipelineAction`, `convert_to_romaji`), `translator/mod.rs` (`load_dictionary`,
`preprocess_text`, `postprocess_text`) and `lib.rs` (`pub use protocol::parser::*`,
which nothing consumed through the crate root).

**Found on the way:** `parser.rs` imported `tauri::AppHandle` and `inject_system_message`
and used neither — the only thing tying it to the Windows crate. The sniffer's
`pipeline`/`message_processor`/`stream_tracker` were already written as pure logic
("100% Pure Logic" in a comment) and moved as-is.

**Split into two crates, not one:** the ui needs the DTOs in wasm; it must not pull
etherparse/kakasi/regex. `resonance-types` is serde-only.

**Not moved: `AppConfig`** — see `active-issues/duplicate-appconfig.md`.

**Wrong-turn avoided / discovery:** assumed the app gate could never run on Linux.
`rustup target add x86_64-pc-windows-gnu` + `gcc-mingw-w64-x86-64` makes
`cargo check -p resonance-stream --target x86_64-pc-windows-gnu` pass (1m42s cold) —
baseline taken on the pre-move tree first (22 warnings, 0 errors), then on the moved
tree (19 warnings, 0 errors). `cargo test --no-run` fails at link, as expected.

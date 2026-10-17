# Refactor — 2026-09

Plan agreed 2026-09-29. One phase, one commit. Push is Kade's.

| # | phase | done when |
|---|---|---|
| 0 | CLAUDE.md, MEMORY.md, `.memory/`, a plan-first skill (since removed) | files exist, index < 40 lines |
| 1 | `justfile` gates + GitHub Actions (ui on Linux, app on Windows); `cargo fmt` baseline | `just check` green on Linux; CI file valid |
| 2 | `crates/core`: protocol decoding, text processing, shared DTOs; ui + app depend on it | core tests run on Linux; one definition of `ChatMessage`/`SystemMessage` |
| 3 | split `src-tauri/src/lib.rs` into events / commands / window / tray / shortcut / logging; `run` only wires | no behaviour change; app gate (CI) |
| 4 | split `Settings` (920-line fn) and `App` (950-line fn) into section components | ui gate green; UI unchanged (manual run on Windows) |
| 5 | hygiene: commit `Cargo.lock`, one version source, `package.json` metadata, template leftovers, stale docs (`stream_traacker.rs` typo fixed in phase 2 during the move) | gates green |

Baseline numbers (2026-09-29): 48 files, 274 symbols, 265 edges.
Largest: `src/app.rs` 971 lines (one fn), `src/components/settings.rs` 933 (one fn),
`src-tauri/src/protocol/parser.rs` 638, `src-tauri/src/lib.rs` 517.
Hotspots: `inject_system_message` 19 callers, `read_varint` 14, `skip_field` 9.
`cargo fmt --check`: 8 files differed (fixed in 8b15f92, listed in `.git-blame-ignore-revs`).

## Results

- **Phase 2:** 31 tests (decoder, packet_buffer, parser, pipeline, message_processor,
  text) moved into `resonance-core`; `cargo test -p resonance-core` 31/31 on Linux.
  12 DTOs de-duplicated into `resonance-types`. App crate dropped `etherparse`, `kakasi`,
  `regex` (their only users moved). App compile-checks for Windows from Linux via
  mingw (discovered this phase; `just app-cross-check`).
- **Phase 3:** `lib.rs` 517 → 158 lines. Bodies copied verbatim; setup order unchanged
  (tray built at the same point, via `tray::setup_tray`). `generate_handler!` list
  byte-identical; 11 `#[tauri::command]`s before and after. `kill_orphaned_servers`
  moved to `translator/server_manager.rs`. App cross-check: 0 errors, warnings 19 → 16
  (only unused-import warnings of `lib.rs` disappeared).
- **Phase 4:** `App` 1033 → 136 lines, `Settings` 933 → 126 (+6 section files).
  Checked mechanically: every moved range is a line-for-line subsequence of its new
  file (whitespace-insensitive), except two re-wrapped by rustfmt, which match with all
  whitespace removed (one trailing comma differs). Settings markup: 755/755 lines
  identical. Call order in `App` unchanged. ui warnings: 8 before, 8 after.
- **Phase 5a (build hygiene):** version set once (`[workspace.package]`), removed from
  `tauri.conf.json` (falls back to src-tauri's Cargo version) and `package.bat` (unused
  var); title bar reads `CARGO_PKG_VERSION`. `Cargo.lock` tracked. `package.json` /
  lock metadata: name `resonance-stream`, MIT (matches LICENSE), private, no stub scripts.
  `public/leptos.svg`, `public/tauri.svg` removed (unreferenced).
- **Phase 5b (docs):** BUILD (ko/en) rewritten to the real toolchain; README_EN synced
  to README.md; TROUBLE_SHOOTING (ko/en) bannered as outdated. **Refactor plan complete.**
- **Follow-up (Kade's decisions, 2026-09-29):** `pcap` + `windivert` + direct `windows` dep removed (Cargo.lock
  −136 lines, removals only); TROUBLE_SHOOTING deleted; Settings footer shows the app
  version; `AppConfig` split between ui and app is intentional architecture.

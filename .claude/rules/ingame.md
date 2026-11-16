---
paths:
  - "crates/ingame/**"
  - "crates/frame/**"
---

# Rules for the in-game DLL (crates/ingame, crates/frame)

The DLL runs inside the game's process. Design: `docs/ingame-design.md`, D-35.

- **Scope (D-35):** the DLL only sends chat. No other hook, no memory write beyond the send path.
  The opt-in is the app's setting (off by default); never bypass it here.
- **No panic path:** no `unwrap`/`expect`/failing indexing/`unreachable!`/unchecked slicing on game
  or pipe data; return a `StepError` or an error reply. A panic in the game's process must not be the failure mode.
- **Every entry point runs under `catch_unwind`:** `DllMain`, the worker's thread procedure, every
  detour callback, every other `extern` function. A caught panic in a detour calls the original and
  disables the feature; none crosses `extern "C"` (that aborts since Rust 1.81). `catch_unwind`
  catches nothing under `panic = "abort"`.
- **Build only with `--profile ingame`** (`just ingame-build`; root `[profile.ingame]`,
  `panic = "unwind"`). Never `cargo build --workspace --release`: its profile is `panic = "abort"`, and the
  `compile_error!` under `cfg(all(windows, panic = "abort"))` in `crates/ingame/src/lib.rs` stops it;
  never remove or `allow` that guard. The installer never carries the DLL: the app downloads the DLL
  built from that profile, signed, per version into APPDATA (`.claude/rules/app.md`).
- **No allocation, no locks, no logging between suspending the game's other threads and resuming them**
  (the hook install, DS-0003: `ilhook`, the DLL's own suspend/resume via `windows-sys`): a suspended
  thread may hold the allocator or a lock, so either can deadlock the game. Prepare everything before
  the suspend; resume on every path, errors included. The hook's hot path (the per-frame pump) does
  one atomic load when idle, and never allocates, locks or logs.
  A thread's context is read only into a 16-byte aligned `CONTEXT` (`ContextBuf`, `read_rip` in
  `win/threads.rs`): `GetThreadContext` fails with 998 on an unaligned one. A failed read is a refusal
  kept in atomics and logged after the resume.
- **Resolve game names at run time, never offsets:** il2cpp exports by name with `GetProcAddress`,
  classes, methods and fields by name, field offsets from `il2cpp_field_get_offset`. No hard-coded
  address, RVA, field offset or signature scan: a game patch must give `unsupported`, not a crash.
  A missing name is logged once and answered as an error; the DLL stays loaded.
- **Hook candidates ask for `Update(1)`,** the only `Update` the game has (`HOOK_CANDIDATES` in
  `names.rs`, pinned by `names::tests::hook_candidates`); argc 0 finds no method. A new candidate takes
  its argc from the game's metadata.
- **No Stellar code copied** (AGPL; this repository is MIT): take facts (names, numbers, layouts),
  write the code and comments in our words, cite the Stellar file only as the source of a fact, as
  `docs/ingame-design.md` does. Applies to `crates/frame` too (goldens are hand-derived bytes).
- **Gates:** pure modules are tested on any OS (`just core-check`); `win/**` is compile-checked on
  Linux (`just ingame-cross-check`). Loading the DLL and sending are Kade's check on Windows: report
  `NOT VERIFIED: just ingame-build: Windows only` rather than pass it.

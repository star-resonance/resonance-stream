# Structure and Architecture Review

- Baseline commit: `1478528` (`main`, right after PR #203); measured 2026-10-06, decisions of the same day added in section 9.
- Scope: repository structure, layers, refactoring candidates, weak points and risks, inefficient logic, recommendations by project concept and feature, documents and process.
- Related documents: [`security_model.md`](security_model.md) (trust boundaries), [`testing.md`](testing.md) (test layers and commands), [`decisions.md`](decisions.md) (decision log).
- **This document only proposes.** No code was changed. (What the work then changed, measured on both commits: [`refactoring_report.md`](refactoring_report.md).)
- Every claim carries its source:
  - **measured**: a value obtained by running something.
  - **confirmed**: the code was opened this session and the cited `file:line` says it.
  - **reported**: a read-only review agent read the code and reported it; not checked by hand. Verify with a reproducing test before acting.
  - **estimated**: a judgement from reading the code.
  - Anything that could not be run is listed in section 10 (`NOT VERIFIED`).

Severity is **High / Medium / Low**, effort is **Small / Medium / Large**.

- High: can lose user data, stop the app from starting, or run untrusted code with administrator rights.
- Medium: silently drops a feature (for example chat that is never saved), slows development, or hides errors.
- Low: tidying, or not a felt problem today.
- Effort: Small = half a day or less, Medium = one to two days, Large = more (several PRs).

## Contents

1. Summary
2. Structure and architecture snapshot
3. Inefficient logic (P)
4. Refactoring candidates (R)
5. Weak points and risks (W)
6. Recommendations by project concept and feature (A)
7. Documents and process (M)
8. Priorities and the next roadmap
9. Decisions made
10. Method and `NOT VERIFIED`

---

## 1. Summary

### State today (measured)

| Item | Value |
|---|---|
| Rust | core 13,843 lines (of which `kanji_on_table.rs` 2,982, `test_env.rs` 1,324, `bridge.rs` 1,102) · types 1,261 · llama 137 · ui (`src/`) 11,095 · app (`src-tauri/src`) 5,530 |
| Python (runbook) | 8,501 lines, 11 pipelines |
| Tests | `cargo test -p resonance-core -p resonance-llama -p resonance-types`: **495 pass** (2026-10-06); ui host tests: **157 pass**. `#[test]` counts: core 385 · types 49 · llama 61 · ui 157 · app 21 |
| Golden (snapshot) tests | 8 (`crates/core/tests/snapshots/`, PR #201) |
| Mutation tests | `text.rs`, 87 mutants: 75 caught · **8 missed** · 4 unviable; 6 minutes on a hosted runner (PR #202) |
| Dependency audit | `cargo audit`: 0 vulnerabilities after reqwest 0.12 (PR #199) |
| CI workflows | 9 (`ci`, `audit`, `mutants`, `bridge-smoke`, `release*`, `auto-merge`, ...) |
| Memory index | `MEMORY.md` **287 lines / 103 KB** (rule: 40 lines and 6 KB, see decision D-2) |

The layering (pure logic in `crates/core`, boundary DTOs in `crates/types`, screens in `src/`, Windows-only code in `src-tauri/`), the gate (`just check`),
real-app verification through the bridge, and the golden / property / mutation tests are in good shape.
The problems below are not "the structure is collapsing". They are **gaps in the trust boundary of an administrator-rights app, a few places where data is silently dropped, and service start/stop logic that is spread out.**

### The five most important points

1. **[High] W-1 A self-update that fails halfway leaves no app.** The current exe is renamed to `.old`; if the second rename fails nothing renames it back
   (`app_updater.rs:150-153`, confirmed). Antivirus or a file lock makes this a real path.
2. **[High] W-2 An app that asks for administrator rights (`requireAdministrator`, confirmed) runs a user-writable `llama-server.exe` without checking it.**
   `Command::new(data_dir/bin/ai-server/llama-server.exe)` is spawned as is (`server_manager.rs:131-139`, confirmed). Only the zip is hash-pinned, once, at download (`server.rs:6-8`, confirmed).
3. **[Medium] W-4 Japanese chat is not archived on some paths.** If the server exe cannot be launched, the worker returns without draining its queue
   (`translator/mod.rs:99-103`, confirmed; the other failure paths at `:134` and `:180` do drain). The root is that archiving is tied to the translation step.
4. **[Medium] W-3 A half-extracted AI server counts as installed.** If `llama-server.exe` exists the function returns success at once (`server.rs:38-40`, confirmed).
5. **[Medium] W-5 A panic is a crash with no log.** `panic = "abort"` (`Cargo.toml:37`, confirmed), no panic hook, and an `unwrap()` on a timestamp that comes from the network
   in the export path (`io/fs.rs:72`, confirmed).

### Already done in this session (for reference)

- PR #203: the test bridge's MQTT packet limit of 10 KiB blocked the `get-chat-history` answer (108 KB) and made the client reconnect. The limit is now 16 MiB and an over-limit answer becomes an error ack (`fit_ack`).
  This was the cause of `CS-restart-nodup`. Real-app confirmation is still open (next manual `bridge-smoke` run).
- PR #199: reqwest 0.11 -> 0.12, four `cargo audit` findings closed.
- PR #201: golden tests for the text pipeline. PR #202: weekly mutation-testing workflow.

### Not problems

- **Capture and text performance are not a problem today.** Regexes are compiled once with `lazy_static` (`text.rs:12-23`, confirmed). The frame assembler is capped at 1 MiB per stream (reported).
  Preprocessing a line costs hundreds of microseconds at most against 100+ ms of LLM time (estimated).
- **Keeping two `AppConfig` types is right.** (*Superseded 2026-10-07: Kade merged them into one, `decisions.md` D-25.*) It is a recorded architecture decision and the round-trip fixture (`app_config_full.json`) catches drift.
- **Test-only code living in core is not a big problem.** Stable exes are built without the `test-env` feature, so the bridge and its MQTT client are not in them (`cargo tree` showed no `rumqttc`, measured in an earlier session).

---

## 2. Structure and architecture snapshot

### 2.1 Directories (lines are `.rs`, measured)

```
crates/core/        13,843 lines  pure logic: protocol decoding, capture pipeline, text processing, history, worker decisions, download checks, test-bridge contract
crates/types/        1,261 lines  DTOs that cross the Tauri boundary (serde only, wasm-compatible)
crates/llama/          137 lines  llama-server HTTP client
src/                11,095 lines  Leptos 0.8 CSR screens (wasm) and pure modules
src-tauri/src/       5,530 lines  Tauri 2 app: sockets, translator server, downloader, windows, tray (Windows only)
runbook/             8,501 lines  Python: 11 bridge pipelines, notebooks, stand-in app
.github/workflows/   9 files      gate, audit, mutation, smoke, release
.memory/, MEMORY.md               working memory (17 session notes, 9 roadmaps, 4 active-issue files)
```

### 2.2 Data flow and layers

```
 game server --TCP 5003--> [raw socket, SIO_RCVALL]   src-tauri/services/sniffer   (Windows, administrator)
                               |
                               v
              [frame assembly -> decode -> de-duplicate]   crates/core/{protocol,capture}   (pure)
                               |
               +---------------+----------------+
               v                                v
   [history + archive (daily jsonl)]    [translator worker: preprocess (shield) -> llama-server -> postprocess]
   crates/core/history, io/archive         crates/core/text, crates/llama, src-tauri/services/translator
               |                                |
               +---------------+----------------+
                               v
                 Tauri events (camelCase JSON)  ->  ui (Leptos) overlay
```

- Pure logic is tested on every OS in `crates/core`; sockets, processes and windows live only in `src-tauri/` (CLAUDE.md rule).
- Screen state lives in the `AppSignals` context (`store.rs`, `config_signals.rs`, `status_signals.rs`, `view_signals.rs`).
- A test MQTT bridge (`test-env` feature) drives and observes the app from outside: `crates/core/bridge.rs` (contract) and `src-tauri/src/bridge/live.rs` (wiring).

### 2.3 Module sizes (measured)

| Module | Lines | Note |
|---|---|---|
| `crates/core/kanji_on_table.rs` | 2,982 | a data table, not a refactoring target |
| `crates/core/test_env.rs` | 1,324 | test-flag parsing |
| `crates/types/lib.rs` | 1,261 | every DTO in one file |
| `crates/core/bridge.rs` | 1,102 | bridge command contract, parser, tests |
| `crates/core/text.rs` | 1,048 | translation pre/post-processing, dictionary, emotes, romaji |
| `src/chat_view.rs` | 908 | per-tab lists, filtering, paging |
| `crates/core/protocol/parser.rs` | 908 | protocol decoding |
| `src/components/chat_row.rs` | 597 | one chat row component (reported: one function of 481 lines) |
| `src-tauri/config/app_config.rs` | 581 | model + file I/O + commands + apply (reported) |
| `src-tauri/services/sniffer/mod.rs` | 450 | start / stop / restart, worker, block commands |
| `src-tauri/services/translator/mod.rs` | 442 | translator worker (thread body about 140 lines, reported) |

### 2.4 State ownership

| State | Where | Written by |
|---|---|---|
| Settings | `config.json` + app `AppConfig`, ui `AppConfig` / `ConfigSignals` | `save_config`; favorites only through `save_favorites` |
| Block list | `AppState.blocked_users`, `config.blocked_users`, ui `ConfigSignals` (reported: three copies) | block/unblock commands, `save_config` |
| Chat history | `ChatHistory` (backend, per-channel limits) + ui `ChatStore` | `store_and_emit`; `load_recent` at start |
| Service lifetime | scattered over `lib.rs`, `sniffer/mod.rs`, `commands.rs`, `app_config.rs`, `model.rs` | UI calls and start-up code (confirmed: the firewall check is duplicated) |

### 2.5 What is done well

- The gate (`just check`) bundles formatting, core tests, the ui compile check and the app cross-check; CI runs the same things per OS.
- **Real-app verification:** a hosted Windows runner starts the real exe and drives it over the bridge (`bridge-smoke.yml`, release tags). It just found the packet-limit bug.
- Property tests (`proptest`), wire-format and config-parity fixtures, golden tests and mutation tests were added in that order.
- Signed self-update (`TRUSTED_UPDATE_KEYS`, a verification example) and a release-feed watcher (`release-feed-check.yml`).
- One task, one branch, one PR, merged by CI, plus the memory tree keep context across sessions.

---

## 3. Inefficient logic (P)

Mostly **reported** (a review agent estimated from reading the code); nothing was benchmarked. Only P-1 can be felt, and only in long sessions.

### P-1 The chat list never shrinks after scrolling up (**fixed 2026-10-07, S6a**: back at the bottom the list is cut to the first page again)

- **Where:** `src/components/chat_container.rs:16` (`display_limit`, starts at 50), `:26` (reset only on tab or search change), `:160` (`+= 50` on every scroll within 50 px of the top).
- **Evidence (confirmed):** the limit only grows, and returning to the bottom does not shrink it. The "all" tab's limit is the sum of the channel limits (reported: 3,200 by default).
  Each row is a large component with memos, effects and closures (`chat_row.rs`).
- **Effect (estimated):** DOM size and per-row state grow the longer the overlay stays open; CPU is not the issue.
- **Proposal:** reset the limit to 50 when the view returns to the bottom (small). To go further, see the windowed list in A-4.2.
- **Severity:** Low · **Effort:** Small

### P-2 Every row runs its own scroll-correction effect (**fixed 2026-10-07, S6c**: the list watches one `rows_changed` signal)

- **Where:** `chat_row.rs:48-62` (reported).
- **Evidence:** each translated row schedules a `request_animation_frame` and a `get_element_by_id`. A tab switch or hydration repeats this N times.
- **Proposal:** one container-level signal.
- **Severity:** Low · **Effort:** Small

### P-3 Study view makes one furigana IPC call per row (**fixed 2026-10-07, S6b**: rows asking within 50 ms share one call)

- **Where:** `chat_row.rs:332-357`, command `annotate_furigana` (`commands.rs:66`, reported).
- **Evidence:** the command takes a `Vec<String>` but each row sends one `[text]`. Turning the study view on in a 1,000-row tab means up to 1,000 round trips (the 2,000-line cache helps afterwards).
- **Proposal:** batch the visible rows.
- **Severity:** Low · **Effort:** Small

### P-4 Block / unblock emits one event per message while holding the history lock (**fixed 2026-10-07, S5d**: the rows are changed under the lock and emitted after it is released; still one event per row)

- **Where:** `sniffer/mod.rs:404-410`, `:424-430` (reported).
- **Evidence:** the `chat_history` lock is held while an event is emitted for each of the user's rows; the capture path (`store_and_emit`) needs the same lock. For a spammer that is hundreds of rows.
- **Proposal:** collect the changes, release the lock, emit once.
- **Severity:** Low · **Effort:** Small

### P-5 The parser builds `unknown_fields` that are then thrown away

- **Where:** `crates/core/protocol/parser.rs:196,215,233,276,300`; cleared in `capture/pipeline.rs:120-122` (reported).
- **Evidence:** five to ten small allocations per message (`format!` + `to_vec`); they are cleared unless debug mode is on. Microseconds at hundreds of messages per minute (estimated).
- **Proposal:** pass the flag into the parser so it does not build them. **Not urgent.**
- **Severity:** Low · **Effort:** Small

### P-6 Translation preprocessing scans the whole dictionary and nickname cache inside a lock (**fixed 2026-10-07, S5e**: the lock is held only to pick the names the message contains)

- **Where:** `text.rs:266-290`; the lock is taken at `translator/mod.rs:390-393`; the capture path takes the same lock at `sniffer/mod.rs:353` (confirmed for the latter; the rest reported).
- **Evidence:** a `contains` per dictionary term and a scan of the nickname cache run under the `nickname_cache` lock. Hundreds of microseconds at thousands of entries (estimated), lost in LLM latency.
- **Proposal:** shorten the lock scope only (copy what is needed, release).
- **Severity:** Low · **Effort:** Small

### P-7 Mutation-testing time

- **Evidence (measured):** 87 mutants of `text.rs` take 6 minutes on a hosted runner (4 locally with `-j 2`). Fine per file; a whole crate would be much longer.
- **Proposal:** rotate files (the workflow's `file` input) and keep it weekly. The present shape is enough.
- **Severity:** Low · **Effort:** Small

---

## 4. Refactoring candidates (R)

### R-1 Service start/stop logic is spread over several places

- **Where:**
  - The firewall check and its warning block exist **twice** in `sniffer/mod.rs:76-90` and `:120-138` (confirmed). One start runs `netsh` twice (reported).
  - The sniffer is started by the UI only: `start_sniffer_command` is invoked from `src/app/hydration.rs:86` and `src/app/setup_flow.rs:42`; `lib.rs` merely registers the command (confirmed by grep). A restart-on-config-change path is in `app_config.rs:327`.
  - Four translator start sites: `lib.rs:136-143`, `commands.rs:51-60`, `app_config.rs:338-368`, `model.rs:129-144` (reported). Kill, retire and generation are handled differently in each.
- **Effect:** a change to start/restart rules must be made in several places, and a mismatch gives double starts or dead workers (W-10).
- **Proposal:**
  1. Add `sniffer_change(old, new, init_done, alive) -> WorkerChange` next to the existing `translator_change` in `crates/core/workers.rs` (pure, table tests).
  2. Give the app one `Services` owner (`start / stop / restart`) and route the call sites through it.
  3. Do the firewall check in one place only.
- **Protected by:** core table tests plus the existing bridge rows (`restart-sniffer`, `start-translator`).
- **Severity:** Medium · **Effort:** Medium

### R-2 Thread-body functions are long and mix decisions with wiring

- **Where:** `sniffer/mod.rs:108-276` (168 lines), `translator/mod.rs:49-188` (140 lines) (reported).
- **Proposal:** move the decisions (watchdog `now - last > 15`, the "Active" state toggle) into pure core functions with an injected clock (`StallWatch` in `download.rs` is the precedent).
- **Protected by:** core tests with a fake clock.
- **Severity:** Medium · **Effort:** Medium

### R-3 The 2-second duplicate window is global-state glue without tests

- **Where:** `src-tauri/src/events.rs:13-17` and `:79-100` (confirmed).
- **Evidence (confirmed):** a `lazy_static` deque of `(fingerprint, Instant)`; entries older than 2 s are pruned, and a repeat of a fingerprint inside the window is dropped. The fingerprint function is already in core (`resonance_core::capture::fingerprint`); only the window logic is in the app.
  It is a second layer next to the pipeline's `(uid, time, sequence)` cache, with a different key. The bridge's `chat-rules` rows check both layers on the real app, but there is no unit test.
- **Proposal:** move the window to `crates/core/capture` as `RecentFingerprints::check(fp, now)` and test it with fake time. That test also decides whether the second layer is needed.
- **Severity:** Medium · **Effort:** Small

### R-4 The per-channel limit defaults are written in three places and disagree (decided: 1000)

- **Where (confirmed):**
  - `crates/core/history.rs:92`: unset means WORLD 200, everything else 1000.
  - `src/chat_view.rs:158-162`: the same rule written again.
  - `src-tauri/config/app_config.rs:92-102` (`default_tab_limits`): WORLD 200, **Local 500**, Party / Guild 1000, **Beginner 500**.
- **Effect:** when the key is missing, Local and Beginner get 1000 (core, ui); a newly created config gets 500. A comment at `history.rs:70` pointing at the ui file is a symptom of the duplication.
- **Decision (D-1):** Local and Beginner are **1000**. So `default_tab_limits` changes from 500 to 1000 for those two (only new configs are affected; saved configs already hold the keys).
- **Proposal:** move `ChannelLimits` and the default table to `crates/types` (wasm-compatible) so it is defined once.
- **Protected by:** one table test plus the existing `chat_view` / `history` tests.
- **Severity:** Medium · **Effort:** Small

### R-5 UI components are long (**`ChatRow` done 2026-10-07, S6d**: pure helpers moved and tested, the row split into `chat_row/{mod,menus,ruby}.rs`; the other three views are left)

- **Where (reported):** `ChatRow` `chat_row.rs:29-510` (481 lines), `FavoritesWindow` 481, `DictionaryModal` 477, `NavBar` 375, `AppUpdateModal` about 200. The relative-time block (`chat_row.rs:88-129`) is pure.
- **Proposal:** move the pure helpers into `utils.rs` / `readability.rs` and test them on the host; split the big views into sub-components. **No behaviour change.**
- **Protected by:** host unit tests and `ui-preview` screenshot comparison (A-4.5).
- **Severity:** Low · **Effort:** Medium

### R-6 Update install (swap / rollback) is inline and cannot be tested

- **Where:** `app_updater.rs:127-168` (confirmed: two renames, then spawn).
- **Proposal:** move `install_swap(current, temp, old, rename)` to core with the rename function injected. A temp-directory test that injects a failure on the second rename reproduces and prevents W-1.
- **Severity:** Medium · **Effort:** Small (one bundle with W-1)

### R-7 The block list has several copies (**fixed 2026-10-07, S5d**: the config is the one copy in the backend; the capture reads it from there)

- **Where (reported):** `AppState.blocked_users` (`protocol/types.rs:40`), `config.blocked_users`, ui `ConfigSignals.blocked_users`; writers are the block / unblock commands and `save_config`.
- **Evidence:** `apply_config` does not refresh the runtime map. Only the block commands write it today, so this is a smell, not a live bug.
- **Proposal:** read through the config at runtime and drop the extra map; test a pure `is_blocked` in core.
- **Severity:** Low · **Effort:** Small

### R-8 Split `app_config.rs`

- **Where:** `src-tauri/config/app_config.rs`, 581 lines (measured): model, defaults, file I/O, `save_*` commands and the 88-line `apply_config` (`:291-378`, reported).
- **Proposal:** split into `config/{model,store,apply}.rs` with **no logic change**. The two-`AppConfig` round-trip fixture is the safety net. Do it together with W-7.
- **Severity:** Low · **Effort:** Small

### R-9 `crates/types/lib.rs` is one file of 1,261 lines

- **Evidence (measured):** every DTO in one file. The wire-format test (`tests/wire_format.rs`) guards it, so splitting is safe.
- **Proposal:** split into `chat`, `service`, `config`, `window` modules with `lib.rs` re-exporting (no path changes). **No hurry:** do it when R-4 touches `types`.
- **Severity:** Low · **Effort:** Small

---

## 5. Weak points and risks (W)

### W-1 Self-update: no rollback between the two renames

- **Where (confirmed):** `app_updater.rs:150-153`
  ```rust
  fs::rename(&current_exe, &old_exe)...?;
  fs::rename(&temp_exe, &current_exe).map_err(...)?;
  ```
- **Trigger (estimated, traced not reproduced):** antivirus or a file lock makes the second rename fail -> the current exe is already `.old` and only an error is returned -> the next launch or shortcut finds no exe.
- **Fix:** on the second error rename `.old` back. With R-6, a temp-directory test (second rename fails) reproduces and prevents it.
- **Severity:** High · **Effort:** Small

### W-2 An administrator-rights app starts an unchecked executable from a user-writable folder

- **Where (confirmed):** the manifest asks for `requireAdministrator` (`src-tauri/app.manifest:19`). The server is started with `Command::new(<app data>/bin/ai-server/llama-server.exe)` and no check (`server_manager.rs:131-139`, `:179`). The same folder's DLLs are loaded by it.
  The only pin is the SHA-256 of the **zip**, checked at download (`server.rs:6-8`).
- **Trigger (estimated):** any process of the same user replaces the exe or a DLL; it then runs as administrator at the next translator start.
- **Fix:** pin per-file SHA-256 values and verify them right before spawning (`sha256_file` already exists), or lock the folder's ACL. See A-6.2.
- **Severity:** High · **Effort:** Medium

### W-3 An interrupted AI-server extraction stays "installed"

- **Where (confirmed):** `server.rs:38-40`: if `llama-server.exe` exists, return `Ok` immediately. The extraction (`:64-80`, reported) writes in place and is not atomic.
- **Trigger (estimated):** the app is killed mid-extract -> the exe exists but DLLs are missing -> llama-server dies at once and the supervisor gives up after three tries (`workers.rs:88`). Translation stays broken until the user deletes the folder.
- **Fix:** extract into a `.part` folder and rename it, or write a completion marker.
- **Severity:** Medium · **Effort:** Small

### W-4 Japanese chat that never reaches the archive

- **(a) Server launch fails (confirmed):** when `launch_ai_server` returns `None` the worker `return`s (`translator/mod.rs:99-103`). The other failure paths (`:134`, `:180`) call `drain_untranslated`, which archives the lines (`:272-276`); this one does not.
  (Reported) `translator_tx` stays `Some`, so `launch_translator` is a no-op, and later `tx.send` errors are discarded with `let _ =`. Nothing is archived for the rest of the run.
- **(b) Catch-up (confirmed mechanism):** `catch_up` empties the queue (`translator/mod.rs:292`). Its doc says those messages are "in the ledger", and the ledger then keeps only the newest `translation_catch_up_limit` (`workers.rs:211-219`). Whether the passed-over lines are archived another way was not checked.
- **(c) Ledger (reported):** the default limit is 100 (`types/lib.rs:497`); older owed lines are dropped.
- **Root:** archiving is tied to the translation step.
- **Fix:** A-3.1: archive on arrival and add a second line when a translation exists; `load_recent` already merges the two.
- **Severity:** Medium · **Effort:** Medium

### W-5 A panic aborts the app and leaves no log

- **Where (confirmed):** `Cargo.toml:37` `panic = "abort"`; `io/fs.rs:72` `Local.timestamp_opt(log.timestamp as i64, 0).unwrap()`. The timestamp is a raw varint from the wire (`parser.rs:192`, reported); an out-of-range value gives `None`, so Export panics and the app dies without a log.
- **Fix:** `.single().unwrap_or_default()` and a panic hook that writes to a file (A-8.1). The other `unwrap` / `expect` calls in this crate are on trusted values (reported).
- **Severity:** Medium · **Effort:** Small

### W-6 `start_sniffer_command` runs `netsh` on the main thread

- **Where (confirmed):** `start_sniffer_command` is a plain `#[tauri::command]` (`sniffer/mod.rs:70-71`) and calls `check_firewall_rule` twice (`:76`, `:120`, R-1). The UI calls it at every start (`hydration.rs:86`). Of 43 commands in the app, 4 are `async` (measured).
  `restart_sniffer_command` is also a plain command but already does its work on its own thread (`:433-450`, the comment says why), so it does not block the window.
- **Effect (estimated):** the window freezes for hundreds of milliseconds up to seconds.
- **Fix:** `#[tauri::command(async)]` and remove the duplicate check. `ensure_firewall_rule_command` (`network.rs:237-300`) runs `netsh` three times and is plain too (reported).
- **Severity:** Medium · **Effort:** Small

### W-7 Settings can be reset silently or written half

- **Where (confirmed):** `app_config.rs:226` `Err(_) => AppConfig::default()`: a failed read (antivirus lock, permissions) makes no backup (the parse-error branch at `:219-225` does). The first save from the UI then overwrites the real file.
- **Where (reported):** `apply_config` logs a failed `write_atomic` but still updates in-memory state (`:299-301`). `metadata.rs:39-47` resets to defaults silently, including model version "0.0.0", which prompts a model re-download. `metadata.rs:60` and `gist.rs:186,234` use plain `fs::write`.
- **Fix:** `write_atomic` everywhere, a backup on any read error, and `apply_config` returns a `Result` that the UI shows (A-5.2).
- **Severity:** Medium · **Effort:** Small

### W-8 Download and metadata trust gaps, no timeouts

- **(confirmed)** `download_model` takes its URL and hash from the UI, which gets them from a mutable gist (`model.rs:57-64`, "from the gist, via the UI"). An empty hash is refused, but there is no host allow-list and no independent signature.
- **(confirmed)** the update-feed, gist-metadata and dictionary fetches use `reqwest::Client::new()` with no timeout (`gist.rs:37,78,172`); a stalled host can hold the start-up "checking updates". Only the update feed has a size cap (256 KB, `gist.rs:20,51`); the metadata and dictionary bodies have none. The model / update downloads do have connect and stall timeouts (`fetch.rs:61,133`).
- **(checked, no gap)** a downgrade is not possible through the app update: only the release announced by the last check is installed (`app_updater.rs:65-70`), and the announcement is filtered for newer versions (`gist.rs:112-120`). A review-agent claim of a re-check gap is dropped.
- **Fix:** verify gist metadata with the minisign keys already built into the app, take the model URL and hash only from signed metadata, add timeouts and size limits (A-6.3, A-6.5).
- **Severity:** Medium · **Effort:** Medium

### W-9 No webview hardening

- **Where (confirmed):** `tauri.conf.json` has `"csp": null` (`:23`) and `"withGlobalTauri": true` (`:12`). `capabilities/*.json` still grants `shell:allow-spawn` and `shell:allow-execute` with a sidecar `bin/translator` (whether anything uses them was not checked).
  `open_browser(url)` hands the string to the opener as is (`io/fs.rs:93-97`, confirmed).
- **Evidence:** no `inner_html` on chat text was found (reported). There is no demonstrated XSS; what is missing is a second layer in front of an administrator process.
- **Fix:** set a CSP, remove unused shell permissions, allow only `https` in `open_browser`.
- **Severity:** Low to Medium · **Effort:** Small

### W-10 Sniffer start / restart races

- **Where (confirmed structure, rest reported):** `restart_sniffer_command` sets `sniffer_tx` to `None`, sleeps 500 ms, then starts a new worker, taking the lock separately for each step (`sniffer/mod.rs:433-450`). `start_sniffer_command` can interleave. The old thread lives up to the 500 ms receive timeout (`network.rs:111`, reported) and both share the statics `LAST_TRAFFIC_TIME` and `IS_SNIFFER_ACTIVE` (`mod.rs:25-26`).
  A socket-create or `SIO_RCVALL` failure (`network.rs:126-136,169-173`) only logs; no Error state is sent, so the watchdog later says "no game traffic" (reported).
- **Note:** the bridge's `capture-spike` confirmed on the hosted runner that after a restart new lines arrive once each.
- **Fix:** one owner with a join handle (R-1) and an Error state on every setup failure.
- **Severity:** Low to Medium · **Effort:** Medium

### W-11 A `[P0]` typed in chat collides with a real placeholder (confirmed; **fixed 2026-10-07, S5b**)

- **Repro (confirmed):** with the dictionary `火力 -> 딜러`, preprocessing `"[P0]火力"` gives `[P0][P0]` and restoring gives `딜러딜러`: the player's own `[P0]` is replaced.
  The golden test `a_placeholder_typed_in_chat_collides_with_a_real_one_known_defect` pinned that behaviour; it is now `a_placeholder_typed_in_chat_keeps_its_own_text` and shows the fixed result (`[P0]딜러`).
- **Effect:** rare, but chat text interferes with the structure of the translation input (prompt control tokens such as `<end_of_turn>` are already stripped: `chat_text_cannot_inject_turn_markers`).
- **Fix:** shield any `[P<n>]` already present in the input like the other literals so numbers cannot clash; the golden snapshot then changes on purpose.
- **Severity:** Low · **Effort:** Small (a task card exists)

### W-12 A leaked `<start_of_turn>model` leaves the word "model" (confirmed; **fixed 2026-10-07, S5c**: the whole header goes, so the output is `번역`)

- **Repro (confirmed):** model output `"<start_of_turn>model\n번역</end_of_turn><eos>"` post-processes to `"model 번역"`: the tag is removed but the role word stays.
- **Fix:** decide whether to remove a leaked role header as a whole (a behaviour decision), then update the golden.
- **Severity:** Low · **Effort:** Small

### W-13 Gaps and limits of real-app verification

- **(confirmed)** the smoke runs automatically only on release tags (Kade's decision). When one step fails the later steps **do not run**: in the last run `capture-spike` failed one row, so chat-rules, persistence, download-integrity, translator-stub and popups were all skipped.
  Rows not yet read on the real app: `CP-fav-*`, `CR-ruby-*` (the 一人 reading, K16), `TS-dict-*`, the K8 `<bos>` count.
- **(confirmed)** the stand-in app had no MQTT packet limit, so dry runs could not show this bug. The `small-packets` switch now catches it.
- **Fix:** keep later steps running after one fails (`if: always()`), while the release still requires every step to pass (decision D-3, A-7.1).
- **Severity:** Medium · **Effort:** Small

### Lower items (reported)

- A hostile `0x8003` frame can make `ruzstd` reserve up to 100 MB before the 1 MiB output cap applies (`frame_decoder.rs:22`, `decode_buffer.rs:49`); it needs an on-path attacker. Cap it with `set_max_window_size`.
- `nickname_cache` is never evicted (`sniffer/mod.rs:354`): small, but scanned under a lock (P-6).
- After a crash a torn last JSONL line can make the next append glue onto it (one extra line lost).
- With `panic = "abort"`, `llama-server` stays running until the next start (`kill_orphaned_servers` and a PID file mitigate); a Windows job object (kill-on-close) would remove it.
- `logging.rs:25-26` sets the app's own crate to Trace in all builds (confirmed code); whether `config.log_level` filters it was not checked.
- No lock-order cycle was found in the locks read. The decoder is clamped and panic-safe on corrupt input (`decoder.rs:29-37,105-133`).

---

## 6. Recommendations by project concept and feature (A)

**Project concept:** "capture game chat -> translate Japanese to Korean -> show it as an overlay", an administrator-rights Windows app, with translation done by a local llama.cpp server.
Four principles follow from that concept: **(1) never lose a chat line, (2) only verified code runs with administrator rights, (3) users must be able to see what went wrong (logs, diagnostics), (4) problems that only show in the real app are found by real-app verification.**
Below, the recommendations per feature (module) with the finding IDs they come from. "Effect" is what a user or developer gains.

### A-1 Capture and protocol (`sniffer`, `core/protocol`, `core/capture`)

| # | Recommendation | Source | Effect | Effort |
|---|---|---|---|---|
| A-1.1 | One owner for sniffer start/stop (`Services`) and a `sniffer_change` table in core | R-1, R-2, W-10 | no double starts or dead workers; rules pinned by tests | Medium |
| A-1.2 | Send an `Error` state on every socket-setup failure | W-10 | no misleading "no game traffic" message | Small |
| A-1.3 | Set a `ruzstd` window limit | lower items | a hostile frame cannot reserve memory | Small |
| A-1.4 | A **sanitised capture corpus** in the repo, replayed in CI (QA method 3) | plan table | decoder regressions on real traffic; needs a sanitiser for nicknames and text and **a real capture from Kade** | Small to Medium |
| A-1.5 | Start capture from the backend, not from the webview (today `hydration.rs:86` and `setup_flow.rs:42` start it; confirmed) | R-1 | a webview failure does not become a capture failure | Medium |

### A-2 Translation (`core/text`, `crates/llama`, `translator`)

| # | Recommendation | Source | Effect | Effort |
|---|---|---|---|---|
| A-2.1 | Gather the four translator start sites into `Services` with one kill / retire / generation handling | R-1 | no restart races or duplicated clean-up | Medium |
| A-2.2 | Fix the `[P<n>]` collision and update the golden | W-11 | chat cannot interfere with the translation structure | Small |
| A-2.3 | Decide how a leaked role header is handled | W-12 | no stray "model" in output | Small |
| A-2.4 | Close the six gaps the mutation run showed (`TranslationCache` eviction order, `is_empty`, `Dictionary` accessors) | PR #202 | the cache's eviction is pinned by tests | Small |
| A-2.5 | Shorten the preprocessing lock scope | P-6 | less waiting on the capture path | Small |
| A-2.6 | (proposal) A **golden set of real sentences** with dictionary terms, emotes and number units, as input and expected masking, 20 to 50 lines | concept | the effect of preprocessing or dictionary changes shows as a diff | Small |
| A-2.7 | **Short dictionary keys** (N-4): `Dictionary` shields a key wherever its characters stand, inside longer words too, because Japanese has no word boundary to match on, so a boundary rule is not possible. Instead `crates/core/tests/repo_dictionary.rs` pins the keys of one character (`〆 凸 ＠`) and of two (25, each with its reason), so a new short key has to be added there on purpose | N-4 | a short key cannot slip in and hit ordinary words unseen | Small |

### A-3 Chat storage and history (`core/history`, `io/archive`, `events.rs`)

| # | Recommendation | Source | Effect | Effort |
|---|---|---|---|---|
| A-3.1 | **Archive on arrival**, then a second line when a translation exists (`load_recent` already merges them) | W-4 | no lines lost to a server failure, catch-up or ledger limit | Medium |
| A-3.2 | Move the 2-second duplicate window to core and test it with fake time | R-3 | the need for the second de-duplication layer is decided by a test | Small |
| A-3.3 | Repair a torn JSONL line (check for a newline before appending) | lower items | one less line lost after a crash | Small |
| A-3.4 | Remove the timestamp `unwrap` in export | W-5 | export cannot crash the app | Small |
| A-3.5 | Tidy block-list copies (`is_blocked` in core) and emit block / unblock events in one batch | R-7, P-4 | one source of truth; less capture stalling | Small |

### A-4 Overlay UI (`src/`)

| # | Recommendation | Source | Effect | Effort |
|---|---|---|---|---|
| A-4.1 | Reset `display_limit` to 50 on return to the bottom | P-1 | limits DOM growth in long sessions | Small |
| A-4.2 | A windowed chat list | P-1, P-2, R-5 | constant DOM at 3,000 rows; risk around anchor scroll, drag, unread logic (`ui-preview` required) | Medium |
| A-4.3 | A container-level scroll effect; batched furigana IPC | P-2, P-3 | less lag on tab switch and study view | Small |
| A-4.4 | Split big views; move pure functions to `utils.rs` | R-5 | behaviour-neutral tidying | Medium |
| A-4.5 | Visual regression with `ui-preview` screenshots | plan table | layout breakage found in a PR | Medium |

### A-5 Settings, favorites, dictionary (`config`, `favorites`, `Dictionary`)

| # | Recommendation | Source | Effect | Effort |
|---|---|---|---|---|
| A-5.1 | `config_version` and a migration chain in core (generalise `parse_config` and `favorites_migration`) | W-7 | old files stay safe when fields change | Small |
| A-5.2 | `apply_config` returns a `Result` shown in the UI; back up on any read error | W-7 | settings are not reset silently | Small |
| A-5.3 | One definition of the channel-limit defaults in `types` (decided: 1000 for Local and Beginner, D-1) | R-4 | core / ui / app agree | Small |
| A-5.4 | Split `app_config.rs` into `model / store / apply` | R-8 | readability, no logic change | Small |
| A-5.5 | Pin "favorites change only through `save_favorites`" with tests (the bridge's `CP-fav-*` rows exist; read their real-app result) | CLAUDE.md | a config write cannot overwrite favorites | Small |

### A-6 Download, update, security (`downloader`, `server_manager`, `update_signature`)

| # | Recommendation | Source | Effect | Effort |
|---|---|---|---|---|
| A-6.1 | `install_swap` in core and rename back on a second failure | W-1, R-6 | a failed update cannot leave no app | Small |
| A-6.2 | **Integrity of what runs elevated:** per-file SHA-256 pins verified right before spawn, or a locked folder ACL | W-2 | blocks privilege gain by a same-user process | Medium |
| A-6.3 | Verify the gist metadata with the built-in minisign keys; model URL and hash only from signed metadata | W-8 | stronger model / dictionary supply chain | Medium |
| A-6.4 | Extract the AI server into a `.part` folder and rename (atomic) | W-3 | an interrupted extraction never counts as installed | Small |
| A-6.5 | Timeouts and size limits on remote calls | W-8 | start-up cannot hang on "checking" | Small |
| A-6.6 | Webview hardening: CSP, remove unused shell permissions, `open_browser` scheme allow-list, optionally a job object | W-9 | a second line of defence in front of an administrator process | Small |

### A-7 Tests and QA (bridge, golden, mutation, corpus)

| # | Recommendation | Source | Effect | Effort |
|---|---|---|---|---|
| A-7.1 | Run smoke pipeline steps independently (`if: always()`); **the release job still needs every step to pass** (D-3) | W-13 | one run reads every row; a red step still blocks a release | Small (CI change, own PR) |
| A-7.2 | Next manual `bridge-smoke.yml` run: read `CS-restart-nodup`, `CP-big-ack` and the five skipped steps | W-13, PR #203 | real-app proof of the packet-limit fix; shows whether the Node broker (aedes) has a limit | Small |
| A-7.3 | Visual regression (Playwright + `ui-preview`) | plan table | unintended layout changes found in a PR | Medium |
| A-7.4 | Rotate mutation runs over files (`text.rs` -> `history.rs` -> `capture/*`) and close the holes with tests | PR #202 | a measure of whether tests notice real bugs | Small |
| A-7.5 | Fault injection over the bridge (network down, llama dies, disk full) | plan table | stuck states and crashes on failure paths | Medium |
| A-7.6 | Soak test and a read-only `stats` command (memory, handles) | plan table | leaks and slow growth (P-1 would show here) | Medium |
| A-7.7 | Move known real-app limits into the stand-in (packet limits and the like) | W-13 | dry runs reproduce more real failures | Small |

### A-8 Operations and observability (logs, diagnostics, CI, release)

| # | Recommendation | Source | Effect | Effort |
|---|---|---|---|---|
| A-8.1 | **A log file and a panic hook:** a rotating `logs/app.log`; panics written to it (`panic = "abort"` makes this necessary) | W-5 | diagnostics a user can send; today only stderr and 200 in-memory lines (`events.rs:67`, reported) | Small |
| A-8.2 | A "copy diagnostics" button: version, settings (secrets excluded), recent log | plan table | reproducible tester reports | Medium |
| A-8.3 | Check the release log level (`logging.rs:25-26` Trace) | lower items | no needless log cost or exposure | Small |
| A-8.4 | A routine for reading the weekly `cargo audit` result | PR #199 | new advisories are not missed | Small |

### A-9 Documents and process

| # | Recommendation | Source | Effect | Effort |
|---|---|---|---|---|
| A-9.1 | Bring `MEMORY.md` back to the rule (40 lines and 6 KB, D-2), with a gate check; move details to `.memory/` | M-1 | cheap session start, no stale contradictions | Small |
| A-9.2 | Prune closed items from `sessions/` (17 files) | M-2 | easier to find context | Small |

---

## 7. Documents and process (M)

### M-1 `MEMORY.md` greatly exceeds its rule (Medium / Small)

- **Measured:** `MEMORY.md` is **287 lines, 103,173 bytes**. The Definition of Done in CLAUDE.md and `.memory/README.md` say "an index, about 40 lines". The *Now* section still starts from 2026-10-04 and each session added entries in front (including four from this session).
- **Effect:** the first file a new session reads is 100 KB; stale sentences that contradict newer facts (for example "`CS-restart-nodup` cause unknown") stay in.
- **Decision (D-2):** the rule is **40 lines and 6 KB**. Proposal: one or two lines per item with links into `.memory/sessions/` and `roadmap/`, closed items deleted (history lives in git and the session notes), and a gate check that fails above the limit.

### M-2 Some notes carried a wrong date (Low / Small)

- **Confirmed:** part of this session's notes said `2026-10-07`; the actual date is 2026-10-06. Fixed in PR #204.
- **Proposal:** write note dates with `date -u +%F`.

### M-3 Only one document in `docs/` (Low / Small)

- **Measured:** before this review, `docs/` held only `code-review-2026-09-29.md`. Structure information lived in the repository-layout section of CLAUDE.md.
- **Proposal:** this review fills that gap; the new `security_model.md`, `testing.md` and `decisions.md` add the rest. Refresh the numbers whenever a roadmap stage closes.

### M-4 Real-app results are not collected in one place (Low / Small)

- **Confirmed:** real-app (hosted runner) results are written in session notes, `test-automation.md` and `MEMORY.md` separately.
- **Proposal:** keep a "rows not yet read on the real app" table in the roadmap (stage S0).

---

## 8. Priorities and the next roadmap

### 8.1 Full table (severity, then effort)

| ID | Title | Severity | Effort | Behaviour change | Stage |
|---|---|---|---|---|---|
| W-1 | Self-update has no rollback | High | Small | yes (bug fix) | S1 |
| W-2 | Integrity of the server exe run with admin rights | High | Medium | yes | S3 |
| W-5 | Panic `unwrap` and no log or hook | Medium | Small | yes | S1 |
| W-3 | Non-atomic AI-server extraction | Medium | Small | yes | S1 |
| W-7 | Settings reset silently / half written | Medium | Small | yes | S2 |
| W-6 | `netsh` on the main thread | Medium | Small | yes | S2 |
| W-13 | Smoke steps independent; unread real-app rows | Medium | Small | n/a (CI) | S0 |
| W-4 | Japanese chat not archived on some paths | Medium | Medium | yes | S2 |
| W-8 | Metadata trust gaps, no timeouts | Medium | Medium | yes | S3 |
| R-1 | Service lifetime scattered | Medium | Medium | no | S4 |
| R-2 | Separate decisions from thread bodies | Medium | Medium | no | S4 |
| R-3 | 2-second duplicate window to core | Medium | Small | no | S4 |
| R-4 | Channel-limit defaults in three places (decided: 1000) | Medium | Small | yes (500 -> 1000 for new configs) | S2 |
| R-6 | Install swap to core | Medium | Small | no | S1 |
| W-9 | CSP, permissions, `open_browser` | Low to Medium | Small | yes | S3 |
| W-10 | Sniffer races and error state | Low to Medium | Medium | yes | S4 |
| W-11 / W-12 | `[P<n>]` collision, stray "model" | Low | Small | yes (golden update) | S5 |
| P-1 to P-3 | Chat list and IPC tidying | Low | Small | no | S6 |
| P-4 to P-6 | Batched block events, parser, preprocessing lock | Low | Small | no | S5 |
| R-5 | Split big views | Low | Medium | no | S6 |
| R-7 to R-9 | Block copies, config split, types split | Low | Small | no | S5 |
| M-1 to M-4 | `MEMORY.md` rule and the like | Medium to Low | Small | n/a | S0 |
| A-1.4 | Sanitised capture corpus | Low | Small to Medium | n/a | S7 (needs Kade's capture) |
| A-4.2 | Windowed chat list | Low | Medium | no (UI) | optional after S6 |
| - | Merge the two `AppConfig`s, full tokio port, bridge out of core | **not doing** | - | - | 8.4 |

### 8.2 Stages (one feature = one branch = one PR)

Common conditions for every stage are those of CLAUDE.md: plan first (impact analysis, then Kade's go), **test first** (a bug fix starts with a reproducing test), the gate of each part touched
(`just core-check`, `just ui-check` for ui, the app cross-check plus Windows CI), `cargo fmt`, a `MEMORY.md` update, a `claude/<name>` branch -> PR -> auto-merge, then the next stage.
The app (Windows) code cannot be linked or run in this environment, so commits say `NOT VERIFIED: app gate` and leave it to Windows CI.
Refactoring stages (S4 to S6) **do not change behaviour** (the wire format is the protocol: `ChatMessage` / `SystemMessage` stay camelCase).

**S0. Docs, memory and reading the real app** (M-1 to M-4, W-13, D-2, D-3) - Small
- S0a Shrink `MEMORY.md` to 40 lines and 6 KB, move details to `.memory/`, add the gate check (decision D-2).
- S0b Smoke steps run independently (`if: always()`) while the release job needs all of them to pass (decision D-3). First check the pipelines do not depend on each other's state (each uses its own folder).
- S0c Run `bridge-smoke.yml` by `workflow_dispatch` on `main`: read `CS-restart-nodup`, `CP-big-ack` and the five skipped steps (`CP-fav-*`, `CR-ruby-*` and the 一人 reading, `TS-dict-*` and the K8 `<bos>` count, popups, download). A red row becomes a new task.
- Done when: `MEMORY.md` within the cap and the gate check passing; a table of real-app rows that are still unread.

**S1. Do not lose the app or data - update, extraction, panic** (W-1, R-6, W-3, W-5) - Medium, 3 PRs
- S1a `install_swap` to core and rename back when the second rename fails (a temp-directory test with an injected failure first).
- S1b Extract the AI server into `.part` and rename (a test that an interrupted extraction is not "installed" first; the pure part in core).
- S1c Remove the export timestamp `unwrap`, add a panic hook and a log file (A-8.1).
- Done when: the exe survives a failing second rename, an interrupted extraction does not count as installed, an out-of-range timestamp cannot kill export, and a panic reaches the log.

**S2. Stop silent drops - storage, settings, limits** (W-4, W-7, W-6, R-4) - Medium, 3 to 4 PRs
- S2a Archive on arrival (a core test first: server launch failure, catch-up and ledger limit still archive).
- S2b Settings: backup on read failure, `apply_config` returns a `Result`, `write_atomic` everywhere, `config_version`.
- S2c Channel-limit defaults defined once in `types`, with **Local and Beginner = 1000** (decision D-1); a table test first.
- S2d `start_sniffer_command` and `ensure_firewall_rule_command` become `async`; remove the duplicate firewall check.

**S3. Trust boundary of the administrator app** (W-2, W-8, W-9) - Medium to Large, 3 PRs
- S3a Per-file SHA-256 verification of the server exe and DLLs before spawn (or ACL).
- S3b Minisign verification of gist metadata, timeouts and size limits.
- S3c CSP, remove unused shell permissions, `open_browser` scheme allow-list (check the CSP with `ui-preview`).
- Done when: a tamper test (a changed file is not started) passes and the existing mock-feed flow still passes.

**S4. Service-lifetime owner** (R-1, R-2, R-3, W-10) - Large, several PRs
- `sniffer_change` table and watchdog decisions to core (fake-clock tests) -> the 2-second duplicate window to core -> a `Services` owner, moving call sites one at a time.
- Done when: bridge `capture-spike` / `translator-stub` give the same results on the hosted runner and the number of start call sites drops.

**S5. Small tidying and defects** (W-11, W-12, P-4 to P-6, R-7 to R-9, A-2.4) - Small, several PRs
- Fix the `[P<n>]` collision (golden update), decide the stray "model" handling, close the six mutation gaps with tests, batch block events, parser `unknown_fields`, block-list copies, split `app_config.rs` and `types`.

**S6. UI tidying and efficiency** (P-1 to P-3, R-5, A-4) - Medium
- `display_limit` reset, per-row effects and IPC, split big views (host tests for pure functions). UI changes need `ui-preview` screenshots and `cargo tauri dev` (Windows, administrator). Decide the windowed list from the result.

**S7. Sanitised capture corpus** (A-1.4) - Small to Medium
- Kade's real capture and a nickname / text substitution tool, replayed in CI by `capture_replay.rs`.

### 8.3 Recommended order and why

`S0 -> S1 -> S2 -> S3 -> S4 -> S5 -> S6 (-> S7)`

- S0 first: reading the rows the real-app verification hid is the cheapest step and may show new problems early; the two decisions that are CI / process changes also land here.
- S1 next: the only path where a user can lose the app itself (W-1), plus small fixes.
- S2 before S4: the chat-storage rule (W-4) should be pinned before the service-lifetime refactor so the refactor cannot break it.
- S3 is larger but the risk it covers is large. The server-exe check (W-2) can be moved right after S1 if you prefer.
- S4 is a big change: start it when no other PR is open (conflicts).
- Content and feature work is independent. S0 to S2 can run alongside it with few conflicts.

### 8.4 Not doing

- ~~Merge the two `AppConfig`s: a recorded decision (Kade, 2026-09-29)~~ -- done 2026-10-07 at Kade's request (`decisions.md` D-25).
- Move `test_env`, `replay` and the bridge out of core (about 2,800 lines): they are not in stable builds (`test-env` feature) and moving costs more than it gains.
- A full tokio port of the worker threads and blocking reqwest: it works and the dedicated threads are well isolated.
- Micro-optimising the parser and text path (P-5, and P-6 beyond lock scope): microseconds against LLM latency.
- Regex caching (already `lazy_static`), parallel llama slots (`--parallel 1` is deliberate), `tauri-plugin-updater` (a signed updater already exists).
- UI click automation (`tauri-driver` / WebDriver): Kade's decision (2026-10-06); `ui-preview` is the UI check.

---

### 8.5 Second pass after the refactor (2026-10-08)

Measured on `main` after #283 (the refactoring report's script): 36,969 Rust lines, 15.0% Windows-only, 772 tests. The layering
(`types <- core <- llama <- src-tauri`, the ui on `types` only) holds and is kept. What is left is at the edges between the parts:

| # | Finding (confirmed in code) | Change | Effort |
|---|---|---|---|
| E-1 | `metadata.json` read-modify-written from five places with no lock (`config.json` has `config_lock`) | `modify_metadata` under `AppState::metadata_lock` (S8a, done) | Small |
| E-2 | 15 process statics hold lifetimes: three answers to "is the sniffer alive" (`SnifferHandle.alive`, `IS_SNIFFER_ACTIVE`, `service_states`), the translator split between `Services` and `WORKER_GENERATION` / `SERVER_PID`, update state in five statics | owners in `AppState` (S8b, done: 10 statics; the other 5 stay on purpose, see the roadmap) | Medium |
| E-3 | the ui <-> backend contract is strings: about 66 `invoke("...")` sites (41 names), 26 hand-built `json!` argument objects, event names in three places | `crates/types::ipc` (S8c, done) | Medium |
| E-4 | the bridge is a second, hand-kept command surface: 17 of its 25 commands call Tauri commands, some through `block_on` | one service layer both call (S8d) | Medium |
| E-5 | the sniffer dispatch and the translation job live in the Windows-only app; `crates/llama/tests/support/harness.rs` is a hand copy ("keep it in step") | core behind a `Sink` trait (S8e) | Medium |

Considered and not chosen: `tauri-specta` bindings (a heavier dependency for what E-3's module does by hand), an actor / tokio rewrite of the
services (E-2 reaches one owner with less churn), several processes or another ui framework (no problem calls for them), splitting the big
views first (worth doing, but cosmetic next to E-1..E-5). Moving the bridge / `test_env` out of core stays "not doing" (8.4).

## 9. Decisions made

| # | Decision | Date | Effect |
|---|---|---|---|
| D-1 | Local and Beginner tab-limit default is **1000** | 2026-10-06 (Kade) | `default_tab_limits` changes from 500 to 1000 for those two; the defaults are defined once (R-4, S2c) |
| D-2 | `MEMORY.md` is limited to **40 lines and 6 KB** (the size is my recommendation, accepted by Kade: "I'll follow your recommendation") | 2026-10-06 | S0a shrinks it and adds a gate check |
| D-3 | Smoke steps keep running after one fails, **but a release is blocked unless all of them succeed** | 2026-10-06 (Kade) | S0b: `if: always()` on pipeline steps; `release.yml` keeps `needs: [check, build, smoke]` and the job fails when any step fails |
| D-4 | Keep two `AppConfig` types | 2026-09-29 (Kade) | not changed |
| D-5 | No UI click automation for now | 2026-10-06 (Kade) | `ui-preview` stays the UI check |
| D-6 | Smoke runs automatically only from a release tag | 2026-10-06 (Kade) | manual `workflow_dispatch` otherwise |
| D-7 | Documents are written in English | 2026-10-06 (Kade) | `docs/` and `.memory/roadmap/` in English; release notes for users stay Korean |

The full log, with the reasons and the open questions, is in [`decisions.md`](decisions.md).

---

## 10. Method and `NOT VERIFIED`

### 10.1 Method

| Purpose | Method |
|---|---|
| Line counts | `find <dir> -name '*.rs' \| xargs cat \| wc -l`, big files sorted with `wc -l` |
| Tests | `cargo test -p resonance-core -p resonance-llama -p resonance-types` (495 pass); counts of `#[test]` by `grep -rn` |
| Mutation | `cargo mutants -p resonance-core --file crates/core/src/text.rs -j 2` (4 minutes locally); hosted run through `mutants.yml` (6 minutes) |
| Audit | `cargo audit` (0 vulnerabilities) |
| Code confirmation | W-1, W-2 (manifest and spawn), W-3, W-4(a), W-5, W-6, W-7 (read path), W-8 (download and timeouts, downgrade), W-9, R-1 (firewall duplicate, who starts the sniffer), R-3, R-4, P-1 (`display_limit`): the file was opened and the line numbers checked |
| Reported | everything else, from a read-only review agent (`general-purpose`) reading the code. Not checked by hand |
| Defect reproduction | W-11 reproduced with a temporary test (`"[P0]火力"`) and then pinned by a golden test; W-12 seen in the golden table |
| Smoke | a manual `bridge-smoke.yml` run on `main` (run 37495056171); the packet-limit text was read from the `CS-restart-nodup` log |

### 10.2 `NOT VERIFIED` / limits

- **`src-tauri/` (Windows only) was not compiled for behaviour or run here.** Only the cross-check (compile) was done; app tests run in Windows CI. Statements about app behaviour come from reading the code.
- **The ui gate was run later in the same session**: `cargo check -p resonance-stream-ui --target wasm32-unknown-unknown` passes and `cargo test -p resonance-stream-ui` passes 157 tests (measured). The app part is still only cross-checked.
- **Nothing was benchmarked.** All effects in the P items are estimates.
- **Items marked "reported" were not checked by hand.** Confirmed items are marked in the text. Before implementing a reported item, reproduce it with a test first.
- The scenarios of W-1, W-3 and W-4 were traced in code, **not reproduced**.
- The `ruzstd` memory claim comes from reading the 0.9.0 source; a hostile frame was not run.
- Not opened (reported scope): `tray.rs`, core `furigana` / `kanji_on` / `replay` / `sniffer_net` / `paste` / `window` / `favorites_migration`, most ui components, `.github/` and the release scripts.
- Not checked: whether `config.log_level` reaches the backend logger, whether the SVG input of `icons.rs` is static, the write permissions of the NSIS install folder, and whether anything uses the `shell:allow-spawn` and `bin/translator` capabilities.
- The packet-limit fix (PR #203) was verified with the stand-in and unit tests only. The real-app result is read in the next manual smoke run.

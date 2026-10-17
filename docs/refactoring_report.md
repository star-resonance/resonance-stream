# Refactoring Report - Before / After the Architecture Review

- Basis: **before** = `1478528` (`main` right after PR #203, the commit the review measured), **after** = `e97e36c` (`main` after PR #244). Between them: 41 pull requests (#204 to #244), 83 commits.
- Measured on 2026-10-07. Both commits were measured with the same scripts (structure metrics, the gate's test counts, behaviour scripts, a mutation run). Every number is **measured** unless it says *estimate*; things that could not be run are listed in section 8.
- The review it answers: [`architecture_review.md`](architecture_review.md) (finding IDs W / R / P / M / A are its IDs). The plan that was followed: `.memory/roadmap/refactor.md` (stages S0 to S7). The decisions taken on the way: [`decisions.md`](decisions.md) (D-25 to D-29 are new).
- Severity is High / Medium / Low and effort is Small / Medium / Large, as in the review.

## 1. At a glance

| Item | Before | After | Change |
|---|---|---|---|
| Rust lines (all crates, integration tests included) | 32,361 | 36,319 | +3,958 (+12%) |
| `crates/core` (pure logic, any OS) | 13,843 | 16,479 | +2,636 |
| `src-tauri` (Windows-only app) | 5,530 | 5,746 | +216 |
| Windows-only share of the Rust lines | 16.1% | 15.0% | the new logic went to the portable crates |
| Python (runbook) | 8,501 | 8,888 | +387 |
| Tests the gate runs (`core` + `llama` + `types`) | 495 | **629** | +134 |
| UI host tests | 157 | **175** | +18 |
| `#[test]` in the Windows-only app (run only in Windows CI) | 21 | 15 | -6 (logic moved to `core` and `types`, where it runs everywhere) |
| Mutation run on `text.rs` | 87 mutants: 75 caught, **8 missed**, 4 unviable | 86 mutants: 81 caught, **0 missed**, 5 unviable | 8 gaps closed |
| Golden (snapshot) tests | 8 | 8 (the two changed on purpose, section 2) | rule kept: a refactor does not change them silently |
| CI wall time of a PR run (one run each) | 2 min 02 s | 1 min 57 s | unchanged, with 150 more tests |
| Functions of 80 lines or more | 32 | 34 | +2 (not a goal of this work, section 4.1) |
| Functions with complexity 15 or more (*estimate*) | 25 | 25 | unchanged |
| Longest component, `ChatRow` | 478 lines, complexity 91 | 272 lines, complexity 52 | -43% lines |
| Call sites that start the translator / sniffer | 4 + 3 in 6 files | 2 + 2 in **1** file (`owner.rs`) | one owner |
| `AppConfig` type definitions | 2 | **1** | merged (D-25) |
| Remote calls with no timeout (`reqwest::Client::new()`) | 3 | **0** | 30 s call / 15 s connect limits, body size caps |
| Non-atomic file writes in the app (`fs::write`) | 4 | 1 (a pid file) | the metadata and dictionary writes use `write_atomic` |
| Webview permissions | 8 (`shell:allow-spawn`, `opener:default` among them) | 5 (none from shell or opener) | the unused shell plugin is gone |
| Model / dictionary metadata | read unsigned from a gist | **signed**, rollback-guarded, revision 2 published | D-28 |
| `MEMORY.md` | 287 lines / 103,173 bytes | 31 lines / 5,983 bytes | rule (40 lines, 6 KB) met, and CI fails above it |
| Documents in `docs/` | 1 | 5 (6 with this report) | review, security model, testing guide and decision log added |
| CI workflows / shell test scripts | 9 / 3 | 10 / 5 | + `metadata.yml`; shell tests for the memory check and the metadata helpers |
| Real-app smoke rows (`.auto(` calls in the pipelines) | 135 | 140 | + the signed-metadata rows `TS-meta-*` |

## 2. Behaviour reproduced on both commits (same script)

A scratch test that calls the public functions of `resonance-core`, run unchanged on each commit. It prints results and asserts nothing.

| Scenario | Before | After |
|---|---|---|
| The same chat message arrives from two game clients (same sender, text and send time, each client's own sequence id), through the capture pipeline | **emitted 2 of 2** (the duplicate check was a 2-second clock in the Windows-only app) | emitted **1 of 2** (the pipeline drops it, with no clock; D-27) |
| `[P0]火力` said in chat, with the dictionary term 火力 -> 딜러 | `딜러딜러` (the typed `[P0]` was taken for the dictionary's placeholder) | `[P0]딜러` |
| `火力[P0]` | `딜러딜러` | `딜러[P0]` |
| The model leaks its turn header: `<start_of_turn>model\n번역</end_of_turn><eos>` | `model 번역` | `번역` |
| The model leaks `<start_of_turn>user\n번역` | `user 번역` | `번역` |

The two text bugs (W-11, W-12) were reproducible, silent, and visible to every user of the translator. The duplicate fix also moved the rule from a Windows-only file to a function that is tested on every OS.

Not reproducible on the *before* commit, because the code did not exist: the update rollback (`install_swap`), the interrupted-extraction check, the pinned server files, the config migration, the metadata verifier. They are covered by tests in section 4.3.

## 3. Per finding: before / after

Legend: ✅ solved · 🟡 partly, or in another way · ⏸ left out on purpose · ❓ not seen in the real app (needs Windows, or a release).

### High

| ID | Before | After | Status |
|---|---|---|---|
| W-1 Self-update | the exe is renamed to `.old`; if the second rename fails nothing renames it back, and there is no app | `install_swap` in `core` renames back when the second rename fails; four temp-directory tests, two of them with a rename that fails on purpose (#210) | ✅ (the real swap on Windows ❓) |
| W-2 Server exe run with administrator rights | a user-writable `llama-server.exe` is started as is; only the zip was hash-checked, once, at download | per-file SHA-256 pins (`server_pins`, table made by `.github/scripts/ai-server-pins.py`); the server starts only when its files are the pinned ones, a mismatch is refused and reported; extra `.exe` / `.dll` files are refused too (#213) | 🟡 the check-to-spawn window is still open (an ACL would close it); the real mismatch message ❓ |

### Medium

| ID | Before | After | Status |
|---|---|---|---|
| W-5 Panic | `panic = "abort"`, no hook, no log; an `unwrap` on a network timestamp in Export | `panic.log` (capped at 256 KiB plus one rotation), the `unwrap` removed, six tests (#212) | ✅ (a general rolling `app.log` not done) |
| W-3 AI-server extraction | the exe exists, so it counts as installed, even half-extracted | extracted into `ai-server.part`, then published; three tests (#211) | ✅ (an install broken by an old version is not detected) |
| W-7 Settings | an unreadable or unparsable `config.json` reset the settings silently; writes not atomic; no version | retried, kept as `.bad`, announced in the system log; `apply_config` / `save_config` return a `Result`; `write_atomic`; `config_version` with a migration chain, a newer file kept as `config.json.v<N>` (#216, #218); **one** `AppConfig` for the app and the ui (#217) | ✅ (a real locked file and a real downgrade ❓) |
| W-6 `netsh` on the main thread | starting the sniffer froze the window on the firewall check | both commands are `async`, one firewall check (#219) | ✅ ❓ |
| W-13 Smoke steps | one red row hid the rest; the real-app rows were unread | steps are independent, a release still needs all of them; five manual runs on `main`, all eight pipelines green (#208, #209) | 🟡 only step results were read, not the row values |
| W-4 Chat not archived | Japanese chat was not archived when the server could not start | every message is in the daily chat log when it arrives (#215) | ✅ (the real reload ❓) |
| W-8 Metadata trust | the gist was trusted; no timeouts, no size caps | timeouts and body caps (#220); signed metadata end to end: verifier (#237), signing workflow (#238), the app reads the signed branch, refuses a bad or missing signature and a rollback, checks the dictionary's SHA-256 (#240 to #242); revisions 1 and 2 published | ✅ for this build; 🟡 copies built before still read the gist |
| R-1 Service lifetime | 7 start call sites in 6 files | a `Services` owner starts, stops and restarts; 4 call sites in 1 file; the sniffer restart waits for the old capture thread (bounded, 3 s) (#225, #226) | ✅ |
| R-2 Decisions in thread bodies | mixed with wiring, untested | `sniffer_change` and `watchdog_check` are pure `core` functions with a passed-in clock (#223) | 🟡 the thread bodies are still long (169 and 140 lines) |
| R-3 Duplicate window | a 2-second clock in the app | a second key in the pipeline's `MessageProcessor` (#222, measured in section 2) | ✅ |
| R-4 Limit defaults | written in three places and disagreeing | one definition in `types`; WORLD 500, every other tab 1000 (#214) | ✅ |
| R-6 Install swap | inline, untestable | in `core` (see W-1) | ✅ |

### Low to medium and low

| ID | Before | After | Status |
|---|---|---|---|
| W-9 Webview | shell and opener permissions nobody used; `open_browser` took any URL | 8 -> 5 permissions, the shell plugin removed, `open_browser` opens only `https` (#221) | 🟡 the CSP is a candidate PR into `rc` (a wrong one blanks the window, so it needs a test on Windows) |
| W-10 Sniffer races | a failed socket setup showed nothing; two captures could overlap | every setup failure is an `Error` state; a dead capture stops the watchdog from talking over it (#224, #226) | ✅ ❓ |
| W-11, W-12 Text bugs | `딜러딜러`, `model 번역` | `[P0]딜러`, `번역` (section 2; #228, #229) | ✅ |
| P-1 List never shrinks | the display limit stayed raised after scrolling up | back at the bottom it is cut to one page (#232) | ✅ (confirmed in a real window, 2026-10-07) |
| P-2 Per-row scroll effect | one effect per row | one `rows_changed` signal (#234) | ✅ (confirmed in a real window, 2026-10-07) |
| P-3 Furigana per row | one backend call per row | rows asking within 50 ms share one call (#233) | ✅ (confirmed in a real window, 2026-10-07) |
| P-4 Block events under the lock | emitted while holding the history lock | emitted after it is released; still one event per row (#230) | 🟡 |
| P-5 Parser `unknown_fields` | built and thrown away | not changed (microseconds against LLM latency) | ⏸ |
| P-6 Preprocessing under a lock | the whole dictionary scan under the nickname lock | the lock is held only to pick the names (#231) | ✅ |
| R-5 Long views | four views of 370 to 480 lines | `ChatRow` split into `chat_row/{mod,menus,ruby}.rs` with its helpers tested (#236) | 🟡 `FavoritesWindow`, `DictionaryModal`, `NavBar` remain |
| R-7 Block-list copies | several | one, in the config (#230) | ✅ |
| R-8, R-9 Split `app_config.rs` / `types` | - | R-8 is moot after the `AppConfig` merge; `types` is still one big file (1,423 lines) | ⏸ |
| A-2.4 Mutation gaps | 8 missed mutants in `text.rs` | 0 missed (section 1; #227) | ✅ |
| M-1 to M-4 Documents | `MEMORY.md` 103 KB; one document | 6 KB index with a CI check; five documents (#206, #207) | ✅ |
| A-1.4 Capture corpus (S7) | - | needs a real capture from the maintainer | ⏸ |
| A-4.2 Windowed list | - | not needed after P-1 to P-3 | ⏸ |

## 4. Metric detail

### 4.1 Functions (*estimate*: a text scan, not a parser; see section 8)

The work did not set out to shorten functions, and the numbers say so: the count of functions of 80 lines or more went from 32 to 34, and complexity 15 or more stayed at 25. What changed:

| Function | Before: lines / complexity | After: lines / complexity |
|---|---|---|
| `ChatRow` | 478 / 91 | 272 / 52 (plus `MessageMenu` 84, a new function that was part of it) |
| `check_all_updates` | 90 / 8 | 108 / 10 (now verifies the signed metadata) |
| `launch_ai_server` | 84 / 6 | 104 / 7 (now checks the pinned files) |
| `apply_config` | 88 / 12 | 99 / 12 (now reports a failed write) |
| `start_download` (wizard) | 126 / 7 | 143 / 12 (every stop now shows its reason) |
| `start_data_factory_worker` | 77 / 11 | 85 / 13 (archive on arrival) |
| `FavoritesWindow`, `DictionaryModal`, `NavBar` | 482, 478, 373 lines | unchanged |

The new checks cost lines in the places that cross a trust boundary, which is where the review asked for them. The long views are the same long views.

### 4.2 Modules

| | Before | After |
|---|---|---|
| Largest files | `kanji_on_table.rs` 2,982 (a table), `test_env.rs` 1,324, `types/lib.rs` 1,261, `bridge.rs` 1,102, `text.rs` 1,048 | the same table, `types/lib.rs` 1,423, `test_env.rs` 1,395, `text.rs` 1,258, `bridge.rs` 1,102 |
| New portable modules | - | `config_migration`, `crash_log`, `server_pins`, `signed_metadata` (core); `app_config` (types); `setup_plan`, `chat_row/{menus,ruby}` (ui) |
| New Windows-only modules | - | `services/owner.rs` (the `Services` owner), `panic_hook.rs` |
| Where a start decision is made | 6 files | `services/owner.rs` |

### 4.3 Safety net: where the 146 new tests went (all `#[test]`, 673 -> 819)

`signed_metadata` +24 · `download` +19 (install swap, `.part`, URL and size rules) · `workers` +13 (sniffer and watchdog decisions) · `types::app_config` +11 · `server_pins` +11 · `text` +11 · `config_migration` +8 · `history` +7 · `chat_view` +6 · `repo_dictionary` +6 (new; one meaning per key, no stray one-character terms) · `crash_log` +6 · `message_processor` +6 · `setup_plan` +4 · and the signed-metadata rows of the real-app smoke run. The Windows-only app lost 9 tests from `config/app_config.rs`; the logic they covered now lives in `types` and `core`.

"Behaviour unchanged" for the refactors was checked by the golden tests (8, unchanged except the two on purpose in section 2), the property tests (10), and `ui-preview` screenshots before and after (`ChatRow`: 9 of 15 states byte-identical, the rest within the 30 to 104 pixels that two runs of one build differ by).

### 4.4 Panic sites (*estimate*: `.unwrap()` / `.expect(` before the test module of each file)

`src-tauri`: 9 + 6 -> 7 + 6. `core`: 7 + 2 -> 8 + 3 (new code). `ui`: 30 + 27 -> 30 + 29. The point was not the count but that a panic now leaves a log (W-5), and that a mouse press in compact mode no longer panics the ui (section 5).

## 5. New findings (not in the review)

1. **Compact mode: every mouse press panicked the ui** (`title_bar.rs`, a disposed signal). leptos 0.8's `window_event_listener` is not removed when its component goes; compact mode removes the title bar. Found while taking baseline screenshots for S6d; fixed in #235 with `utils::window_listener`.
2. **The first-run wizard's last step showed no error at all.** Failures went to the title bar's small status text and the log, so the person saw the button come back. Found in M3c; the reason is now shown under the button (#242).
3. **`sync_dictionary` recorded the version the UI passed.** It now records the version of the verified metadata (#241).
4. **A tag-triggered workflow runs the file from the tagged commit.** The first `metadata-v1` tag sat on a commit that had no `metadata.yml`, so nothing ran; it was re-tagged on `main`. `tauri signer sign` was run on a `.json` file for the first time and works.
5. **Two dictionary entries were wrong:** `光砕型` and `光盾型` were the other way round, and `ギルミーメイジ` was `마법사사`. Fixed in dictionary 1.0.7 (#243), with a test that a key never has two meanings across categories.
6. **The base model moves from TranslateGemma to hy-mt2 (D-29)**, so K8 (the `<bos>` count) is moot. The Gemma-specific code to revisit then: the prompt and its pinned test, `sanitize_input`, the turn-tag patterns, the stop tokens, the prompt goldens, the model hash in the metadata, and whether the pinned llama.cpp build runs the model.

## 6. Remaining weak points (after)

| Area | What is left | Severity | Effort |
|---|---|---|---|
| Views | `DictionaryModal` 478 lines (complexity 82), `FavoritesWindow` 482 (77), `NavBar` 373 (53), `ChatContainer` 299 (56) | Medium | Medium |
| Thread bodies | `start_sniffer_worker` 169 lines, `start_translator_worker` 140, the bridge's `handle` 208 | Medium | Medium |
| Webview CSP (W-9) | a candidate PR into `rc` exists (branch `candidate/webview-csp`), checked in Chromium and pinned by a test; the exe is untested on Windows | Medium | Small |
| Server files | the window between the hash check and the spawn (an ACL would close it) | Low to Medium | Medium |
| Installed copies | 0.6.x reads the unsigned gist until it is updated | Medium | - |
| Not seen in a real window | the wizard's refusal line, the update check, how the new dictionary terms read in real chat (S6 and the signed first-run path were confirmed by the maintainer on 2026-10-07) | Low | Small |
| Smoke evidence | only step results were read; the row values (K8, K16) are unread | Low | Small |
| `types` | one 1,423-line file | Low | Small |
| Capture corpus (S7) | waits for a real capture | Low | Small |
| Class trees in the dictionary | eight trees where the cheat sheet and the dictionary still disagree (月影 氷牙 霜天 狼弓 鷹弓 剛身 威咲 森癒) | Low | Small |
| Cheat sheet vs dictionary | two hand-maintained copies of the same names, with nothing linking them: the cause of the eight trees above (roadmap N-3) | Low | Small |
| Dictionary matching | a term matches anywhere inside a word; only one-character keys are guarded by a test (N-4) | Low | Small |
| Dictionary in use | only the editor's badge shows the dictionary version; nothing shows the signed revision or whether the file was edited (roadmap N-6) | Low | Small |
| Sync button | the settings "update dictionary" button shows no result; success and failure are only in the system log (N-1) | Low | Small |
| `sync_dictionary` | took a `version` argument it ignored (N-2, removed 2026-10-07) | Low | Small |
| `download_model` (M4) | took its URL and hash from the UI (N-5, fixed 2026-10-08: the backend verifies the metadata at every download and takes them from it) | Low to Medium | Small |
| Tooling | the `ui-preview` skill had no recipe for the first-run wizard (added) | Low | Small |

## 7. Assessment from the comparison

- **Biggest gains:** an update, an extraction, a settings write or a crash no longer destroys the app or its data silently (W-1, W-3, W-4, W-5, W-7); the trust boundary of an administrator-rights app is checked instead of assumed (pinned server files, signed and rollback-guarded metadata, no unbounded remote call, fewer webview permissions); service start and stop has one owner; three user-visible bugs are gone and shown gone by the same script on both commits.
- **Safety net:** +134 gate tests and +18 UI tests, 8 mutation gaps closed, golden tests kept, and the gate still takes about two minutes. The share of Rust that only Windows CI can run fell from 16.1% to 15.0%.
- **What did not improve, and was not meant to:** the size and complexity of the big views and the thread bodies; `types` is still one file.
- **What was left out had a reason:** P-5 (microseconds), R-9 (no hurry), the windowed list (not needed), UI click automation (a decision), the CSP (a wrong one blanks the window, so it needs a Windows test first).
- **The honest gap:** this is verified on Linux and in Windows CI, and by the maintainer in a real window for S6 and the signed first-run path (2026-10-07). What is still unseen is in section 6.

## 8. Method and limits

- **Structure metrics:** `scripts/refactoring_metrics.py <worktree>`, run on a checkout of each commit. A function is found by its `fn` signature and its body by brace matching, comments and strings are blanked, and anything after `#[cfg(test)]` in a file is left out; `kanji_on_table.rs` (a table) is not counted as logic. Complexity is 1 + `if` + `while` + `for` + `loop` + `&&` + `||` + match arms: an *estimate* that will differ from clippy's. Same script, same container, both commits.
- **Tests:** `cargo test -p resonance-core -p resonance-llama -p resonance-types` and `cargo test -p resonance-stream-ui` on each commit (495 + 157 before, 629 + 175 after; the "before" total is the review's own number, which the method reproduces). The Windows-only app's tests were counted (`#[test]`), not run.
- **Behaviour:** the scratch test of section 2, run on both commits.
- **Mutation:** `cargo mutants -p resonance-core --file crates/core/src/text.rs` on `e97e36c`. The "before" figures are the review's own measurement from the weekly mutation job added in PR #202.
- **CI time:** one PR run before (`claude/bridge-packet-limit`) and one after (`claude/dict-season3`); a single measurement each, so only "about the same" is claimed.
- **Real app:** `bridge-smoke` was dispatched on `main` five times (all eight pipelines green: runs 37545082187, 37567018818, 37573382046, 37588905867, 37613640483); only step results were read.
- **NOT VERIFIED:** the app's own tests (Windows CI only); the real update swap, extraction, locked-file and downgrade paths; the real firewall and capture failures; the wizard's refusal line and the update check in a real window; the CSP (a candidate PR into `rc`, not yet tested). Confirmed by the maintainer on 2026-10-07 in `cargo tauri dev`: S6a to S6d, the compact-mode click, and the first-run wizard with the real signed metadata.

## 9. Follow-up after the roadmap (2026-10-07)

| Item | Result | PR |
|---|---|---|
| Signed metadata, app side | the app reads the signed `metadata` branch; the core pieces, the app, then the wizard's error line | #240, #241, #242 |
| Dictionary 1.0.7 | season 3 dungeon names and the classes' trees from the cheat sheet; the 실드 나이트 trees and the `마법사` typo fixed; published as `metadata-v2`, verified with the app's own verifier (accepted after revision 1, refused as a replay after revision 3) | #243, #244 |

**Needs the maintainer:** the CSP test through `rc`; a real capture for S7; the K16 readings; the eight class trees above; whether to update the gist for installed 0.6.x copies; the hy-mt2 prompt (section 5, item 6).

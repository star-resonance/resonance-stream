# Testing

The test layers of Resonance Stream, what each one catches and cannot catch, how to run it, and when CI runs it.
Companion to [`architecture_review.md`](architecture_review.md) (finding IDs refer to it), [`runbook/README.md`](../runbook/README.md) (how Kade runs the pipelines by hand) and the rules in [`CLAUDE.md`](../CLAUDE.md).

Numbers are **measured** on 2026-10-06 on `main` unless marked otherwise.

## 1. The layers

| # | Layer | What it catches | Where | Runs |
|---|---|---|---|---|
| 1 | **Unit tests** (`#[test]`: core 385, types 49, llama 61, ui 157, app 21) | logic errors in pure code | next to the code (`mod tests`) and in `tests/` | every PR (`just core-check`, `just ui-check`; the app's 21 only in Windows CI) |
| 2 | **Property tests** (`proptest`) | panics and wrong output on random or damaged bytes and text; frames independent of how TCP cut the stream | `crates/core/tests/properties.rs` | with `just core-check` |
| 3 | **Wire-format and config-parity fixtures** | a renamed field (a protocol change); a setting added without a value in the fixture, or one the type does not read or write | `crates/types/tests/wire_format.rs`, `crates/types/testdata/app_config_full.json` (round-tripped by the one `AppConfig`, `crates/types/src/app_config.rs`) | with the unit tests |
| 4 | **Golden (snapshot) tests** (`insta`, 8 snapshots) | an accidental change to shielding, restoring, the prompt, emotes or romaji | `crates/core/tests/golden_text.rs`, `crates/core/tests/snapshots/` | with `just core-check` |
| 5 | **Capture replay** | the decoder and pipeline on a recorded packet stream | `crates/core/tests/capture_replay.rs` and `fixtures/` | with `just core-check` |
| 6 | **llama client tests** against a fake server | the `/completion` and `/health` client, scenarios | `crates/llama/tests/` (`llama.rs`, `scenarios.rs`, `capture.rs`, `support/`) | with `just core-check` |
| 7 | **Mutation tests** (`cargo-mutants`) | tests that would not notice a real bug | `.github/workflows/mutants.yml`; `text.rs`: 87 mutants, 75 caught, 8 missed, 4 unviable | weekly, on demand, and when the workflow file changes; **not** in `ci.yml` |
| 8 | **Dependency audit** (`cargo audit`) | known vulnerabilities (the app downloads and runs executables) | `.github/workflows/audit.yml` | weekly, on demand, when a `Cargo.toml` / `Cargo.lock` changes; **not** in `ci.yml` |
| 9 | **Real-app bridge pipelines** | behaviour of the real exe: capture, chat rules, persistence, downloads, translator, popups, window restore | `runbook/` (Python pipelines) + `runbook/bridge/` (Node broker) + a `test-env` exe | `bridge-smoke.yml`: from a release tag, or by `workflow_dispatch` |
| 10 | **UI preview** | how a view looks and reacts | the `ui-preview` skill (wasm build + mocked Tauri bridge + Playwright) on Linux | by hand, on every ui change |
| 11 | **Manual checks** | what needs a person: the game, anti-cheat, clipboard, a second monitor, a VPN, the real GPU, tray and shortcuts | `.memory/active-issues/unverified-on-windows.md` and its triage `checklist-triage.md` | by Kade on Windows |

Layers 1 to 8 need no Windows machine except the app's own unit tests. Layer 9 is the only automatic check of the real app.

## 2. Commands

```bash
just check            # fmt-check, core-check, ui-check, then the app gate for this OS
just core-check       # cargo test -p resonance-core -p resonance-llama -p resonance-types
just ui-check         # cargo check -p resonance-stream-ui --target wasm32-unknown-unknown  +  cargo test -p resonance-stream-ui
just app-check        # Windows: cargo test -p resonance-stream
just app-cross-check  # Linux: compile only against x86_64-pc-windows-gnu (cannot link or run)
just coverage         # cargo llvm-cov summary of core + llama + types
```

Setup on Linux: `rustup target add wasm32-unknown-unknown x86_64-pc-windows-gnu` and `apt install gcc-mingw-w64-x86-64`.
The core tests build `lindera-ipadic`, which downloads its dictionary from `Lindera.dev` (the host must be reachable).
`cargo test` of the ui crate runs on the host; only the `cargo check --target wasm32-unknown-unknown` needs the wasm target.

Cross-check variants used for the bridge (compiled only with the `test-env` feature):

```bash
cargo check -p resonance-stream --target x86_64-pc-windows-gnu --tests
cargo check -p resonance-stream --target x86_64-pc-windows-gnu --tests --features test-env
cargo check -p resonance-stream --target x86_64-pc-windows-gnu --release --features test-env
```

**Golden tests:**

```bash
cargo test -p resonance-core --test golden_text                    # compare
INSTA_UPDATE=always cargo test -p resonance-core --test golden_text # accept what the code does now
git diff crates/core/tests/snapshots/                              # READ it; accept only what you meant
```

**Mutation tests** (one file at a time; a whole crate takes far longer):

```bash
cargo install cargo-mutants --locked
cargo mutants -p resonance-core --file crates/core/src/text.rs --no-shuffle -j 2   # about 4 minutes
# results: mutants.out/{caught,missed,timeout,unviable}.txt ; every "missed" line is a test to write
```

**Real-app pipelines** (a `test-env` exe is needed; the hosted runner builds one):

```bash
# build:  npx @tauri-apps/cli@2 build --no-bundle --features test-env
pip install pytest nbformat nbclient ipykernel      # once
(cd runbook/bridge && npm ci)                       # once
cd runbook && python -m runbook.run <pipeline> --exe ../target/release/resonance-stream.exe
# pipelines: updater-mock, interface, window-restore, capture-spike, chat-rules, persistence,
#            download-integrity, translator-stub, popups   (replay-chat: node runbook/bridge/cli.mjs run replay-chat)
cd runbook && python -m pytest -q                   # the pipelines against the stand-in app: a courtesy, 3 to 10 minutes
```

## 3. How the real-app checks work

- **`test-env` build.** The bridge (`src-tauri/src/bridge/live.rs`) and its MQTT client exist only with this feature; a stable exe holds neither (a release workflow test pins that the stable build never mentions `test-env`).
  Test flags (`--bridge-url`, `--data-dir`, `--llama-url`, `--metadata-url`, `--feed-url`, `--dictionary-url`, `--metadata-trust-key`, `--replay-chat`, ...) are parsed in `crates/core/test_env.rs`; URLs must be `https://` or loopback `http://`.
- **The contract** is `crates/core/bridge.rs`: a fixed list of named commands (`ping`, `replay-chat`, `block-user`, `get-chat-history`, `save-favorites`, `restart-sniffer`, `start-translator`, `sync-dictionary`, `open-popup`, ...), the topics, the envelope and ack format, and `MAX_PACKET_BYTES` (16 MiB; an answer over it becomes an error ack).
- **A pipeline** (Python, `runbook/runbook/pipelines/*.py`) starts the exe against a Node MQTT broker (`runbook/bridge/`, `aedes`), sends commands, waits for the app's events and acks, reads the files the app wrote, and records **rows** (`CS-...`, `CP-...`, `CR-...`, `TS-...`, `PP-...`) as pass / fail / skip.
- **The stand-in app** (`runbook/tests/fake_app.py`) is a Python model of the app that speaks the same bridge; every pipeline is developed against it, and **every row has a bug switch** (`FAKE_APP_*_BUG`) that must turn exactly that row red. A green dry run proves the pipeline, never the app.
- **The stand-in llama server** (`runbook/runbook/llama_stub.py`) answers like llama-server (`/completion`, `/health`) with no GPU; `--llama-url` points the real app at it.
- **The hosted runner** (`windows-latest`) is elevated, interactive, has the WebView2 runtime and a 1024x720 work area. It is not Kade's PC: GPU, the game, tray, DPI scaling and anti-cheat remain his.
- **`bridge-smoke.yml`** builds the exe and runs: bridge unit tests, `replay-chat`, `window-restore`, `capture-spike`, `chat-rules`, `persistence`, `download-integrity`, `translator-stub`, `popups`. Only a stable release (`release.yml`) calls it, as a hard gate on the tagged commit; neither a pull request nor a push to `rc` runs it.

## 4. When CI runs what

| Workflow | Trigger | Runs |
|---|---|---|
| `ci.yml` | every pull request; push to `main` (non-doc paths) | fmt, core + ui on Linux; the app gate on Windows |
| `auto-merge.yml` | after `CI` completes | merges a green `claude/*` PR into `main`; leaves a draft or a PR that changes `.github/workflows/` to Kade |
| `actionlint.yml` | pull requests that change `.github/workflows/` or `.github/actionlint.yaml`; on demand | actionlint with shellcheck on every workflow; the findings it ignores are in `.github/actionlint.yaml` |
| `audit.yml` | Cargo file changes, Mondays, on demand | `cargo audit` |
| `mutants.yml` | its own file changes, Mondays, on demand (`file` input) | `cargo mutants` on one core file; green when the tool ran, the missed list is in the summary and the `mutants-out` artifact |
| `bridge-smoke.yml` | `workflow_dispatch`, and `workflow_call` from `release.yml` | the real-app pipelines |
| `release-candidate.yml` | push to `rc` | CI gates, a plain exe, a GitHub prerelease |
| `release.yml` | a pushed `vX.Y.Z` tag | gates, smoke (a hard gate), signed exe, `latest.json` |
| `release-feed-check.yml` | release events, daily | the live update feed is readable |
| `test-branch-guard.yml` | pull requests into `main` | a `test/*` branch must never merge |

A pull request does not run the smoke test (Kade's rule, 2026-10-06): a bridge or runbook change is proven on the real app only by a manual dispatch or the next tag.

## 5. Rules

1. **Test first.** The failing test comes before the code, and you see it fail for the right reason. A bug fix starts with a test that reproduces the bug. New pure logic goes in `crates/core`.
2. **A golden diff is read, not accepted.** A snapshot records today's behaviour, not a claim that it is the best answer. Pinned known defects (`[P0]` collision, a leftover "model") change on purpose when fixed.
3. **A new setting** goes into `AppConfig` (`crates/types`), the `app_config_full.json` fixture and `ConfigSignals` with the same name.
4. **A new bridge command** needs: a core `Command` variant and parse tests (red first), one match arm in `bridge/live.rs`, a stand-in model with a bug switch, rows in a pipeline, and the README line.
5. **A new pipeline row** is shown red by a stand-in bug switch before it is trusted.
6. **Never skip, disable or edit a test to get green.** Never report a gate as passed when it could not run; name it in the commit (`NOT VERIFIED: ...`).
7. **The runbook is docs.** No CI job checks it and it has no gate; the pytest run is a courtesy.
8. Mutation results are a to-do list: each missed mutant is a test to write (or a note that the mutant is equivalent).

## 6. What the layers do not catch (known gaps)

| Gap | Effect | Plan |
|---|---|---|
| The stand-in app does not model the real client's limits | the 10 KiB MQTT packet limit was invisible in every dry run (found by the real run, PR #203) | A-7.7: move known real limits into the stand-in |
| A failing smoke step hides every later step | one red row hid five pipelines in the last run | decision D-3, stage S0b: `if: always()`, while a release still needs all steps green |
| Smoke runs only from the release run (by hand on `rc`) or by its own manual dispatch | a bridge change is not proven on the real app before it merges | accepted (Kade); read the result after a manual dispatch |
| The real-app rows `CP-fav-*`, `CR-ruby-*` (the 一人 reading, K16), `TS-dict-*`, popups and download have not been read since they were built | their real-app result is unknown | S0c |
| Mutation testing covers `text.rs` only | the rest of core has no measure of test strength | A-7.4: rotate files |
| The app's 21 unit tests run only on Windows | a Linux session cannot run them | Windows CI |
| No visual regression | a layout break is found by eye | A-4.5 / A-7.3 |
| No fault injection, no soak test | failure paths and slow leaks are untested | A-7.5, A-7.6 |
| Human-only items (game paste, anti-cheat, second monitor, VPN, real GPU, tray and shortcuts) | only Kade can check them | `checklist-triage.md` |
| UI click automation | not wanted (Kade, 2026-10-06) | `ui-preview` stays the UI check |

## 7. Where a new test goes

| You are changing | Put the test in |
|---|---|
| a pure function (decoding, text, history, worker decision, download check) | `crates/core` unit test; if its output is a table of strings, a golden test |
| anything read from outside bytes or text | add a property test beside the existing ones |
| a DTO that crosses the Tauri boundary | `crates/types/tests/wire_format.rs` |
| a settings field | `AppConfig` (`crates/types`) and `app_config_full.json` |
| a pure ui module (`chat_view`, `favorites`, ...) | a unit test in the module (runs on the host) |
| a view's look | the `ui-preview` skill (screenshot) |
| Tauri / Windows glue that cannot be unit-tested | say so in the commit body; add a bridge pipeline row if the real exe can show it |
| behaviour only the real exe shows (restart, persistence, downloads, popups) | a pipeline row, its stand-in bug switch, and the README line |

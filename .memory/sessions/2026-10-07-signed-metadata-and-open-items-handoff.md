# Handoff -- signed metadata done, the open items N-1 to N-6, the CSP candidate (2026-10-07, evening)

For a new session with no context. It continues [`2026-10-07-architecture-roadmap-handoff.md`](2026-10-07-architecture-roadmap-handoff.md) (background: S0-S6, the
decisions D-25 to D-29, how the work is done); read that only if you need the history. Read `CLAUDE.md` first (plan first, test first, one task = one branch =
one PR, wait for the merge before the next, never report a gate as passed when it could not run, a blocked host is asked for), then `MEMORY.md`, then
[`../roadmap/refactor.md`](../roadmap/refactor.md) (the sections "Real-window check and decisions" and "Open items found after S6" are what you act on) and
[`../../docs/refactoring_report.md`](../../docs/refactoring_report.md) (the before / after of the whole refactoring, with its method).
Kade is the user (Korean, a Windows PC with the game). He answers in short lines; "follow your recommendation" means do what you recommended.

## Where things stand

`main` is at the merge of #249 (`c68472d`) or later. Everything from the architecture review that was planned is merged: S0-S6, the signed metadata
(M1-M3: #237, #238, #240, #241, #242), dictionary 1.0.7 (#243), the report (#245), the docs corrections (#246, #247), the cheat-sheet / dictionary test (#249).

- **Signed metadata is live.** `metadata/` on `main` is the source; a pushed tag `metadata-v<N>` runs `metadata.yml`, which signs and publishes it on the generated
  `metadata` branch (`metadata.json`, `metadata.json.sig`, `custom_dict.json`). Published: revision 1 and **revision 2** (dictionary 1.0.7). The next tag is
  **`metadata-v3`**, and it is Kade's to push (`git fetch origin main && git tag metadata-v3 origin/main && git push origin metadata-v3`); the tagged commit must
  already contain `metadata.yml`. A merge never signs. The app (build of `main`) reads only that branch: **it has no gist address**. Installed 0.6.x copies still read
  the old gist, which Kade leaves untouched on purpose.
- **Checked by Kade in `cargo tauri dev` (2026-10-07):** S6a-d (list limit, furigana, scrolling, `ChatRow` menus), the compact-mode click, the first-run wizard with
  the real signed metadata. **Not checked yet:** the wizard's refusal line, the update check (O-3), how the new dictionary terms read in real chat (unit tests only).
- **Open PR #248 (not yours to merge):** the webview CSP, branch `candidate/webview-csp`, **into `rc`**. Kade merges it by hand, the release-candidate workflow
  builds a plain exe (prerelease `v0.6.1-rc.webview-csp`), he runs it on Windows and reports; then the same branch goes to `main` in its own PR (`rc` is never merged
  into `main`). The PR description has his checklist. If he reports a problem, the CSP violation text names the directive.

## What you do next: N-4, N-6, N-1, N-2, N-5, one PR at a time, in this order (Kade approved all, 2026-10-07)

Each is a normal task: branch `claude/<name>` from `origin/main`, plan (short, in the answer), failing test first, gates, memory in the same branch, PR, `subscribe_pr_activity`.
**Auto-merge lands a PR 2 to 3 minutes after it opens**, once CI is green; wait for the merge event, check CI on the merged commit, then start the next from `origin/main`.
The gist is not touched by any of them.

1. **N-4 Short dictionary keys.** `Dictionary` shields every occurrence of a term, with no word boundary (Japanese has none), so a short key hits longer words.
   `crates/core/tests/repo_dictionary.rs` already pins the one-character keys. Extend it: pin the **28 keys of one or two characters** (today: `PT 〆 乱風 光盾 光砕 凸 剛守 募集 千夢 周回 完凸 工場 巨塔 巨竜 巨龍 暗霧 浮島 烈風 狂音 珊瑚 虚飾 迷妄 遺跡 鉄牙 雷刃 霧海 響奏 ＠`), each with a one-line
   reason in the test, so a new short key must be added on purpose. Write the matching behaviour down in `docs/architecture_review.md` A-2. Test only plus docs; no dictionary change.
2. **N-6 Show which dictionary is in use** (Kade's own request: "add the custom-dict version inside the app so I can check"). Today only the dictionary editor's badge shows
   `v<version>` (the version last synced). Plan: store `current_dict_sha256` and the signed `revision` in `AppMetadata` at sync (serde default; they are known: the verified metadata);
   a pure core function says same / modified / unknown by comparing the SHA-256 of the file with the stored one (`save_local_dictionary` does not touch it, so a local edit shows
   as modified); a small command and a DTO in `crates/types` (wire test); in the settings, next to the sync button: "v1.0.7 · signed revision 2 · modified/same". `ui-preview` for the view.
   Show Kade the plan before coding (new feature, wire addition).
3. **N-1 The sync button shows no result.** `sync_dict_action` (`src/components/settings/mod.rs`) returns "최신 상태" or "동기화 실패" and nothing displays it. Show the outcome and the
   backend's reason next to the button (format it in a pure, host-tested ui function). Do it with N-6 in the same view if it stays small; otherwise its own PR.
4. **N-2 `sync_dictionary` ignores its `version` argument.** Remove it from the command, the ui callers (`hydration.rs`, `setup_flow.rs`, `settings/mod.rs`), the bridge command
   (`crates/core/src/bridge.rs`, `src-tauri/src/bridge/live.rs`: check how `parse_command` treats an extra field), the stand-in app `runbook/tests/fake_app.py` and the pipelines that send
   `sync-dictionary`. A wire change: plan first.
5. **N-5 `download_model` still takes its URL and hash from the UI.** Make the backend use the last verified metadata (as `sync_dictionary` does: `LAST_VERIFIED` in
   `src-tauri/src/services/downloader/gist.rs`, or verify itself). **Careful:** the `download-integrity` pipeline (`runbook/runbook/pipelines/download_integrity.py`) tests wrong hash / cut-off /
   404 / no hash / non-https by passing the URL and hash over the bridge; it has to move to a signed mock (`mockfeed.MetadataSigner`, `MockServer(signer=...)`, `--metadata-trust-key`) whose model entry varies.
   Plan first, and tell Kade that pipeline changes.

Also tell Kade when a PR changes what a real window shows, so he can look.

## Waiting on Kade (do not do these for him)

- **Merge #248 into `rc` and test the exe**, then say so. Then you open the PR of the same branch into `main`.
- **Push the tags:** `metadata-v<N>` after a dictionary or model change merges (bump `metadata/metadata.json`'s dictionary `version`, or the app never offers the sync); `v<version>` for a stable
  release. **The signed metadata reaches users only in a new stable release** (0.6.1 reads the gist): bump `[workspace.package] version`, write `release-notes/v<version>.md` (copy `TEMPLATE.md`;
  about 10 plain Korean lines above `## 개발자용 상세`, at most 12), merge, push the tag; `release.yml` builds, smoke-tests, signs, publishes. Never publish a stable release by hand. Suggest it after #248 is tested.
- **The eight class trees** (月影 氷牙 霜天 狼弓 鷹弓 剛身 威咲 森癒) and 威咲's fan name イサキ: for each, which Korean name is right, the cheat sheet's or the dictionary's. They sit in
  `KNOWN_DISAGREEMENTS` in `src/cheatsheet.rs` (the test fails when one is settled and not removed). He settled only 光砕 / 光盾: the cheat sheet. Also `開拓` (always-available dungeon, not added).
- **K16 and the smoke logs:** he reads them in a new session. The artifact `bridge-smoke-logs` of run **37613640483** (`CR-ruby-probe` for K16, the `TS-meta-*` row values) **expires 2026-10-14**;
  after that dispatch `bridge-smoke.yml` on `main` again (`mcp__github__actions_run_trigger`, ref `main`, about 22 minutes). Earlier green runs: 37545082187, 37567018818, 37573382046, 37588905867.
- **O-3 the update check:** no dialog is expected while the app is 0.6.1 = the newest release; only a newer release shown to an older copy proves it.
- **Inputs for work that has not started** (noted in the roadmap, nothing to do until he gives them): S7 a real capture from the app's debug "Raw Capture" (never committed raw: it holds real nicknames; then a
  substitution tool and `capture_replay.rs`); D-29 hy-mt2 (its prompt template, stop tokens, file hash; the Gemma-specific code to revisit is in the report, section 5 item 6; the model goes out through the signed path).

## Things learned that are not in the code

- **GitHub can answer a push with `500 Internal Server Error` for a few minutes** (it did, 16:56-17:05 UTC). It is not your commit: retry with 2/4/8/16 s backoff, then `send_later` a retry; reads keep working. Do not
  write the files through another route.
- **A wrong CSP blanks the window**, and the first proposed policy would have: `connect-src` needs `'self'`, because the app starts by `fetch`-ing its own wasm file. Tauri adds the SHA-256 of every inline
  `<script>` itself (`tauri-codegen`), so the config has no hash. **These files are on the branch `candidate/webview-csp` (#248), not on `main` yet:** `.claude/skills/ui-preview/csp-check.mjs` checks a policy
  against a real `trunk build` (SKILL.md, "Checking a Content-Security-Policy"), and `crates/core/tests/webview_policy.rs` pins the rules. Not testable here: WebView2 and Tauri's IPC.
- **Previewing the first-run wizard** needs `config.js` rewritten (`init_done` false): SKILL.md, "Previewing the first-run wizard".
- **The mock signer.** `mockfeed.MetadataSigner` makes a throwaway key with `npx @tauri-apps/cli@2 signer generate --ci` and signs with `signer sign -f <key> -p "" --app-version <revision> <file>`; a dry run
  uses a marked stand-in scheme that only `runbook/tests/fake_app.py` reads. The real tool's output was checked against the app's own Rust verifier.
- **Pipelines against the stand-in app need `npm ci` in `runbook/bridge`** (otherwise the pytest dry runs skip). `pip install pytest`; `nbformat` is not installed, so the two notebook-wrapper tests cannot run here.
- **`cargo tauri` is not installed** here; use `npx --yes @tauri-apps/cli@2 ...`. `trunk` is (0.21.14); `trunk build` needs a hook-free copy of `Trunk.toml` on Linux (the repo's hook calls `cmd`).
- **Measuring before / after:** `python3 scripts/refactoring_metrics.py <worktree>` (stdlib; a text-scan estimate), two `git worktree add --detach` checkouts, the scratch test described in the report's section 8.
- **Auto-merge is fast;** if Kade wants to review a PR first he must say so before you open it.

## How the work is done here (unchanged)

- Gates: `cargo fmt --check`; `cargo test -p resonance-core -p resonance-llama -p resonance-types` (629 pass on `main`; 637 once #248's `webview_policy` tests arrive); `cargo test -p resonance-stream-ui` (177) and
  `cargo check -p resonance-stream-ui --target wasm32-unknown-unknown`; the app only as a compile check: `cargo check -p resonance-stream --target x86_64-pc-windows-gnu` (plain, `--features test-env --tests`,
  `--release --features test-env`); `bash .github/scripts/memory-check.sh` and `bash .github/scripts/*.test.sh`. The app's own tests run only in Windows CI.
- Environment setup this session needed: `rustup target add wasm32-unknown-unknown x86_64-pc-windows-gnu`, `apt-get update && apt-get install -y gcc-mingw-w64-x86-64`, `cargo install cargo-mutants trunk --locked`,
  `pip install pytest` (and `playwright`, `Pillow` for `ui-preview`), `npm ci` in the repo root and in `runbook/bridge`.
- Mutation testing: `cargo mutants -p resonance-core --file <file> --no-shuffle -j 2 --output <dir>` (about 6 minutes for `text.rs`).
- Golden tests use `insta`; `INSTA_UPDATE=always cargo test -p resonance-core --test golden_text` rewrites snapshots, then read `git diff`.
- `MEMORY.md` is an index of at most 40 lines and 6,144 bytes (CI fails above); it is at about 5.1 KB. Move detail to `.memory/`.
- After a PR merges: check CI on its last commit (`mcp__github__actions_list`, `list_workflow_runs` for `ci.yml` on the branch), confirm the `claude/*` branch is gone, cancel its `send_later` check-in
  (`delete_trigger`), then start the next from `origin/main`.
- A blocked host is asked for, not worked around. The gist hosts are blocked from this session; the app no longer needs them.

## First message for the new session

"Read `.memory/sessions/2026-10-07-signed-metadata-and-open-items-handoff.md`, `MEMORY.md` and `.memory/roadmap/refactor.md`. Tell me what you read, then wait for what I ask (I may report the CSP exe test,
the class-tree answers, or tell you to start N-4)."

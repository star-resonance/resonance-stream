# Handoff -- dictionary 1.0.8, N-1 / N-3 / N-4 / N-6 done, N-2 and N-5 left (2026-10-07, night)

For a new session with no context. It continues [`2026-10-07-signed-metadata-and-open-items-handoff.md`](2026-10-07-signed-metadata-and-open-items-handoff.md)
(background: the signed metadata, how the work is done, the environment setup, the things learned; read it for those). Read `CLAUDE.md` first (plan first, test first,
one task = one branch = one PR, wait for the merge before the next, never report a gate as passed when it could not run), then `MEMORY.md` and
[`../roadmap/refactor.md`](../roadmap/refactor.md) (the section "Open items found after S6").
Kade is the user (Korean, a Windows PC with the game). He answers in short lines; "follow your recommendation" means do what you recommended.

## Where things stand

`main` is at the merge of #254 (`fef5b97`) or later. This session merged: **#251** the webview CSP into `main` (Kade had tested the `rc` exe: "works well"), **#252** dictionary 1.0.8,
**#253** N-4, **#254** N-6. **#255 (N-1) was open, CI running, when this handoff was written** -- check it first (`mcp__github__pull_request_read`, then the `ci.yml` run on its
branch); if it is red, fix it on the same branch, if merged, confirm the `claude/*` branch is gone.

- **#252 dictionary 1.0.8.** Kade: "use the cheat sheet's Japanese names as the source, but not only one character of kanji". So the eight class trees (月影 氷牙 霜天 狼弓 鷹弓 剛身
  威咲 森癒) now read the cheat sheet's Korean (월광 얼음창 얼음빔 늑대활 매활 방패 심판 치유), `イサキ` is a new key for 심판, no one-character kanji key was added, and `KNOWN_DISAGREEMENTS`
  in `src/cheatsheet.rs` shrank to 7 (ティナ 巨塔 墓 始 継 終 開拓). `metadata/metadata.json` says dictionary `1.0.8`. **It reaches users only after Kade pushes `metadata-v3`.**
- **#253 N-4.** `crates/core/tests/repo_dictionary.rs` pins the 25 two-character keys with a reason each (the reasons are my reading of the categories and the cheat sheet; Kade may correct one);
  A-2.7 in `docs/architecture_review.md`. No dictionary change.
- **#254 N-6.** `AppMetadata.current_dict_sha256` + `current_dict_revision` are set at each sync; `resonance_core::signed_metadata::dictionary_state` (same / modified / unknown; two empty
  hashes are unknown); the command `get_dictionary_status` returns `DictionaryStatus { version, revision, state }` (types crate, wire test); the ui's pure `src/dictionary_status.rs::status_line`;
  the settings (data section) show e.g. `v1.0.8 · 서명 리비전 3 · 게시본과 같음` under the sync button (`직접 수정됨`, `확인 불가`, `아직 동기화한 적 없음`). A copy that synced before this has no
  hash and shows 확인 불가 until its next sync.
- **#255 N-1.** The same file adds `sync_outcome` / `version_to_sync`; the sync button's result line is green / red with the backend's reason as sent; a refused publication (`metadata_error`) or a
  failed check stops before `sync_dictionary` is called. Its tests were written together with the code (then checked by breaking the code); the PR says so.

## What you do next, in this order (one PR at a time, as before)

1. **N-2 `sync_dictionary` ignores its `version` argument.** Since M3b it records the verified metadata's version and only logs a warning when the UI's differs (`src-tauri/src/services/downloader/gist.rs`).
   Remove the parameter from: the command; the ui callers (`src/app/hydration.rs`, `src/app/setup_flow.rs`, `src/components/settings/mod.rs` -- #255's `sync_dict_action` still looks the version up through
   `check_all_updates` / `version_to_sync` only to pass it, so after N-2 it needs the check only to see a refused publication or can call the command directly); the bridge command
   (`crates/core/src/bridge.rs` `sync-dictionary` with its `version` field, `src-tauri/src/bridge/live.rs`; **check how `parse_command` treats an extra field** so an older caller does not break); the stand-in app
   `runbook/tests/fake_app.py`; and every runbook pipeline that sends `sync-dictionary`. A wire change: **plan first, tell Kade, wait for his go.** Runbook changes are docs (no gate), but run the dry runs
   (`cd runbook/bridge && npm ci`; `cd runbook && python -m pytest -q`, about 3 minutes).
2. **N-5 `download_model` still takes its URL and hash from the UI.** Make the backend use the last verified metadata (`LAST_VERIFIED` in `gist.rs`, or verify itself). **Careful:** the `download-integrity`
   pipeline (`runbook/runbook/pipelines/download_integrity.py`) tests wrong hash / cut-off / 404 / no hash / non-https by passing the URL and hash over the bridge; it has to move to a signed mock
   (`mockfeed.MetadataSigner`, `MockServer(signer=...)`, `--metadata-trust-key`) whose model entry varies. Plan first, tell Kade that the pipeline changes.

Also tell Kade when a PR changes what a real window shows (N-6 and N-1 do: he has not looked at either in `cargo tauri dev`).

## Waiting on Kade (do not do these for him)

- **Push `metadata-v3`** (dictionary 1.0.8): `git fetch origin main && git tag metadata-v3 origin/main && git push origin metadata-v3`. He said he would do it after the work; nothing after #252 changes the dictionary,
  so the tag can go any time now. A merge never signs.
- **A stable release** (`v0.6.2` or whichever he picks): the signed metadata reaches users only in a new stable release. Bump `[workspace.package] version`, write `release-notes/v<version>.md` (copy `TEMPLATE.md`;
  about 10 plain Korean lines above `## 개발자용 상세`, at most 12), merge, he pushes the tag; `release.yml` builds, smoke-tests, signs, publishes. Never publish by hand. Suggest it once N-2 / N-5 are in or he asks.
  This release is also the only way to prove **O-3** (the update check): start the previous exe (0.6.1) after it and see whether the update dialog shows; if not, read the system log's update line.
- **Look at N-6 / N-1 in a real window** (`cargo tauri dev`, Administrator): the line under the sync button in the data settings, after a sync and after editing the dictionary; and whether the new class-tree names read well in real chat.
- **K16 and the smoke logs:** artifact `bridge-smoke-logs` of run **37613640483** (`CR-ruby-probe` row, the 一人 reading; the `TS-meta-*` row values) **expires 2026-10-14**. He should download it
  (Actions -> the run -> Artifacts), find the row, compare with the reference reading from his other Claude session and paste both; after the date, dispatch `bridge-smoke.yml` on `main` again
  (`mcp__github__actions_run_trigger`, ref `main`, about 22 minutes).
- **S7 (capture corpus)** needs a real capture from the app's debug "Raw Capture" (never committed raw: it holds real nicknames; then a substitution tool and `capture_replay.rs`). **hy-mt2 (D-29)** needs its
  prompt template, stop tokens, the GGUF URL and file SHA-256; the model goes out through the signed path (`metadata/metadata.json`, then his tag). Nothing to do until he gives them.

## Things learned this session that are not in the code

- **The designated branch is `claude/tender-faraday-hdnlzh`; reuse the name for every task:** after a merge, `git fetch origin main && git checkout -B claude/tender-faraday-hdnlzh origin/main`. Auto-merge deletes the
  remote branch, so the stop hook then says "1 unpushed commit" -- it is the merge commit already on `main`; do not push it (that would recreate a branch with merged history).
- **`.claude/skills/ui-preview/shot.mjs` is stale:** the settings button is no longer visible, it sits in the "..." menu. Working recipe: `page.locator('button').nth(16).click()` (the "..." button; the
  toolbar order may change), then `page.locator('button[aria-label=Settings]').click()`, then click through `.modal-box nav li button` until the section you want shows. Mock a command with
  `page.evaluate(() => { window.__mock.cmd = () => value })` (a function that `throw`s a string makes `invoke` reject with it). Worth fixing in the skill on a docs-only PR.
- **A container is fresh each session:** redo the setup in the earlier handoff (`rustup target add wasm32-unknown-unknown x86_64-pc-windows-gnu`, `apt-get install gcc-mingw-w64-x86-64`, `npm ci`, playwright,
  `preview.sh <scratchpad>/ui-preview` which serves on 127.0.0.1:8765).
- Test counts on `main` after #254: core + llama + types **641**, ui **180** (185 with #255). `MEMORY.md` is near its 6 KB limit: keep *Now* short.
- A new short dictionary key must be added to `TWO_CHARACTER_KEYS` (or the one-character list) in `crates/core/tests/repo_dictionary.rs` with a reason, or CI fails.

## First message for the new session

"Read `.memory/sessions/2026-10-07-open-items-n2-n5-handoff.md`, `MEMORY.md` and `.memory/roadmap/refactor.md`. Tell me what you read, then wait for what I ask (I may say N-2 go, report the real-window check
of N-6 / N-1, push `metadata-v3`, or give the capture / hy-mt2 inputs)."

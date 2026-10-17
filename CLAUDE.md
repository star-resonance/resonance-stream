# Agent Operating Rules — resonance-stream

Resonance Stream is a Windows desktop app: it sniffs Blue Protocol: Star Resonance
chat packets (raw socket, no client hooking), translates Japanese chat to Korean
through a local llama.cpp server, and shows it in a Tauri overlay.

One Cargo workspace in five crates, three parts, plus the runbook (treated as docs).
**Which part you touch decides which gate applies** -- the most important thing on this page.

Current state: the organization Project "Resonance" (star-resonance), read as `STATUS.md` on the
`status` branch, index @MEMORY.md

| tree | part | builds on | gate |
|---|---|---|---|
| `crates/core/` `crates/llama/` `crates/types/` | **core** — packet → chat pipeline, protocol decoding, translation text processing; llama-server HTTP client; DTOs shared by app and ui | any OS | `just core-check` |
| `src/` | **ui** — Leptos 0.8 CSR frontend (wasm); pure modules unit-tested on the host | any OS | `just ui-check` |
| `src-tauri/` | **app** — Tauri 2 backend: sockets, translator server, downloader, windows, tray | **Windows only** | `just app-check` (Windows) · `just app-cross-check` (Linux, compile only) |
| `runbook/` | **runbook** — Jupyter notebooks + Python helpers that Kade runs by hand on Windows (`runbook/README.md`) | any OS (dry runs) | **none — treated as docs** (see `.claude/rules/runbook.md`) |

`just check` (`pip install rust-just`) runs `fmt-check`, then every gate this OS can run. On Linux
the app gate is a **compile-only** cross-check against `x86_64-pc-windows-gnu` (rustup target +
`gcc-mingw-w64-x86-64`), so the app's own tests run only on Windows. CI (`ci.yml`) runs core + ui
on Linux and the full app gate on `windows-latest`, on every push and PR.

**New pure logic goes in `crates/core`** (or the ui's pure modules), where it is tested on every OS;
`just coverage` shows what the tests do not reach. Anything that
crosses the Tauri boundary is defined once, in `crates/types` (serde, serde_with — it
compiles to wasm) -- `AppConfig`, the settings file's type, too (`crates/types/src/app_config.rs`;
the app owns the file and the real defaults, the ui re-exports the same type).

## Tech Stack

- **Frontend:** Leptos 0.8 (CSR) → wasm via Trunk; Tailwind 4 + daisyUI (Trunk pre-build hook, `cmd /c`).
  **Backend:** Tauri 2 (`unstable`, tray, global-shortcut, fs, shell, opener).
- **Capture and translation:** raw-socket capture (Administrator) and a local llama.cpp server; detail in `.claude/rules/app.md`.
- **Updates and metadata:** signed stable-release feed and signed metadata; detail in `.claude/rules/app.md`.
- **Packaging:** `package.bat` → `cargo tauri build` → NSIS in `dist/`; version: `[workspace.package]` in `Cargo.toml`.

## Repository Layout (top level; per file: `.claude/rules/{core,ui,app}.md`)

```
.claude/              rules/ (path-scoped), skills/: release, ui-preview, workflow-control
.memory/              working memory; see .memory/README.md
.github/ justfile     CI (gates per OS; release, rc, metadata workflows); the gates as commands
crates/               core, llama, types — pure logic, tested on any OS
src/                  ui crate (resonance-stream-ui)
src-tauri/            app crate (resonance-stream, lib resonance_stream_lib)
metadata/ release-notes/  signed metadata source; release notes per version
docs/                 architecture review, decisions log, security model, testing
runbook/              notebooks Kade runs on Windows -- docs, see .claude/rules/runbook.md
style/ public/        CSS source; static assets
```

## Guardrails

- **Protocol.** `ChatMessage`/`SystemMessage` serialize as camelCase across the Tauri boundary:
  renaming a field is a protocol change, not a refactor.
- **Gist and release URLs are public**; none of them is a secret.
- **Trusted hosts:** `Lindera.dev` (the furigana dictionary's build script downloads from it).

## Definition of Done

1. **Behaviour check where a gate cannot see it.** UI changes: `.claude/rules/ui.md`.
2. **Record state on the issue** (a `cmd:` label: `cmd:status-now`, `cmd:verify-needs-windows`, `-macos`, `-android`, `-iphone`, `cmd:verify-not-verified`; a decision for Kade: assign the issue to him; a follow-up gets an issue); `MEMORY.md` only for what has none.

## Version Control

- `.github/workflows/auto-merge.yml` (not GitHub auto-merge) merges a green PR and deletes its
  `claude/*` branch.
- Local sessions follow the same flow: a `claude/<task>` branch and a PR, opened with Kade's
  `gh` login (without it, print the commands for Kade).
- Only `claude/*` PRs into `main` auto-merge; a PR into `rc` (or anywhere else), a draft, or a PR
  that changes `.github/workflows/` waits for a person.
  `workflow_run` workflows are read from `main`: an `auto-merge.yml` change acts after its own merge.
  Workflow merges use `GITHUB_TOKEN` (no `push` run on `main`); the PR's own run is the gate.
- Dependabot (`.github/dependabot.yml`) opens `dependabot/*` PRs weekly; they never auto-merge:
  Kade merges them.
- **Test branches, release candidates, stable releases:** the `release` skill; the rules for release
  files (never publish by hand, release notes) are in `.claude/rules/release.md`.
- **Never commit:** build output (`target/`, `dist/`, `style/output.css`, `*.exe`);
  `node_modules/`, IDE folders; an unformatted tree.

## Overrides of global rules

- **Test first** does not apply to `runbook/`: it is docs, run by hand on Windows, with no CI job
  and no gate (`.claude/rules/runbook.md`).

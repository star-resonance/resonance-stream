---
name: ui-preview
description: See a ui (src/) change in a real browser on Linux -- build the Leptos
  app to wasm, serve it with a mocked Tauri bridge, drive and screenshot it with
  Playwright. Use on every task that changes what the UI looks like or does,
  before committing; it is how a Linux session checks a view it cannot run in
  `cargo tauri dev`.
---

# ui-preview

`just ui-check` proves the ui compiles and its pure modules pass. It does not show
what a view looks like. The Tauri app (`cargo tauri dev`) needs Windows. This skill
closes the gap: the real ui, the real Tailwind/daisyUI CSS, in headless Chromium,
with `window.__TAURI__` faked.

## Steps

1. **Serve.** `.claude/skills/ui-preview/preview.sh <scratchpad>/ui-preview`
   (first run ~2 min: installs the wasm target, `wasm-bindgen-cli` pinned to
   `Cargo.lock`, `npm ci` for Tailwind; then a rebuild is ~20 s). It prints the
   folder it serves on `http://127.0.0.1:8765`. Re-run it after every code change.
2. **Drive.** Copy `shot.mjs` to the scratchpad, replace its example steps (it
   opens settings and visits each category) with the flow the task touches, run
   `node shot.mjs <out_dir> [width] [height]`. The default 900x640 is about the
   app's window; also try a small size if the change is about layout.
3. **Look.** Read the PNGs. Check layout at the sizes that matter, both themes if
   colours changed, and the printed invoke log: every backend call the flow
   makes, with its arguments (Maps flattened, as Tauri's IPC does).
4. **Fix, rebuild, repeat.** Say in the commit body what was screenshotted, and
   that `cargo tauri dev` on Windows was still not run.

## How it works

- `preview.sh` generates the mock `config.json` from the ui's own `AppConfig`
  (a throwaway test appended to `src/ui_types.rs`, restored from a backup even on
  failure), so new config fields never make the mock stale. `init_done: true`
  skips the setup wizard -- edit the test in the script for a different start
  state.
- `mock.js` answers `invoke` from `window.__mock` (unknown commands return
  `null`) and records every call in `window.__calls`. When a view needs data
  (a history, a status), add a case there, or override it per run with
  `page.addInitScript` / `page.evaluate(() => window.__mock.cmd = ...)`.
  Backend events (`listen`) never fire; to show a state that arrives by event,
  set it through a command the view calls on mount instead.

## Previewing the first-run wizard

`init_done: true` in the generated `config.js` skips it. Do not edit `preview.sh` for one run: in the
Playwright script intercept the file and flip the flag --

```js
await page.route('**/config.js', async (route) => {
  const res = await route.fetch();
  await route.fulfill({ response: res, body: (await res.text()).replace(/init_done"?\s*:\s*true/, (m) => m.replace('true', 'false')) });
});
```

-- then, after `page.goto`, answer the backend per case with `page.evaluate` (a mock function that
`throw`s a string makes `invoke` reject with that string). To reach the last step: tick the agreement
checkbox, click "동의하고 시작하기", switch the translation toggle on (`input.toggle`) and click "다음";
"다운로드 시작" then calls `check_all_updates`, `download_model`, `download_ai_server` and
`sync_dictionary` in that order.

## Checking a Content-Security-Policy

A wrong policy blanks the real window and cannot be run on Linux, so check it in Chromium first
(`csp-check.mjs`). It needs the *real* Trunk output (the preview's own page is not it: Trunk adds an
inline module script that starts the wasm).

1. `cargo install trunk --locked` once. Run `preview.sh` once (it makes `style/output.css`).
2. Build with a copy of `Trunk.toml` that has no `pre_build` hook (the repo's hook calls Windows `cmd`),
   absolute `target` and `dist`, and the `style/output.css` asset: `trunk build --config <that file>`.
3. Put the policy in a file. Tauri adds `'sha256-<hash of the inline script>'` to `script-src` itself, so the
   config has none; for the check, add it: the hash is `base64(sha256(text between <script type="module">
   and </script>))` of `dist/index.html`.
4. `node csp-check.mjs <dist> <preview web dir> <policy file> <label>` prints the violations and any other
   page problem (a favicon 404 is expected). It must be 0 violations and the main view present.
5. Run controls as well, so a silent pass cannot fool you: the same policy without the hash, without
   `'wasm-unsafe-eval'`, with `style-src 'self'` -- each must show its own violation.

Found this way (2026-10-07): the first proposed policy had `connect-src ipc: http://ipc.localhost` and no
`'self'`; the page's start-up `fetch` of its wasm file is a `connect-src` request, so the window stayed
blank. The policy is pinned in `crates/core/tests/webview_policy.rs`. What this cannot show: WebView2 on
Windows and Tauri's IPC (`ipc:`, `http://ipc.localhost`) -- the test-env exe of a candidate does.

## Limits

- Not the Tauri window: no transparency, no window commands, no tray, no global
  shortcuts. Window geometry commands only log their arguments.
- A flow that depends on a backend event cannot be reached without extending
  the mock.
- `confirm()` dialogs block in headless Chromium unless the script handles
  `page.on('dialog', d => d.accept())`.

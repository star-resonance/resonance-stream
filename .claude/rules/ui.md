---
paths:
  - "src/**"
---

## Conventions (ui crate)

- **State lives in `AppSignals` (context), not in component locals**, when more than one
  component touches it. Components read it with `use_context::<AppSignals>()` and reach a
  signal through its group: `signals.config` (settings), `.service`, `.setup`, `.updates`,
  `.chat`, `.ui`. A new signal goes in the group it describes.
- **A component under `<Show>` is re-created each time it shows.** State that must
  survive closing a modal (typed input, an `Action`'s last result) is created by the
  parent outside the `<Show>` and passed down as props — see `components/settings/`.
- Helpers that need many signals take `signals: AppSignals` and destructure only the
  fields they use (`let ChatSignals { a, set_b, .. } = signals.chat;`).
- **A setting that lives in `config.json` is a field of `AppConfig` (`crates/types`) and a
  signal of `signals.config` (`ConfigSignals`, named like the field).** Adding one means adding
  it to `AppConfig` (with its default and a value in `app_config_full.json`) and to
  `ConfigSignals` -- `to_config` / `apply` list every field, so forgetting is a compile
  error. Its load-time quirks (a saved value that is
  clamped or replaced) live in `apply`, not in `hydration.rs`.

## Behaviour check for UI changes

UI changes are screenshotted in a browser with the `ui-preview` skill
(`.claude/skills/ui-preview/`, runs on Linux), and need a manual run (`cargo tauri dev`,
Windows, as Administrator); say in the commit body which of the two was done.

## Layout (src/)

```
src/                  ui crate (resonance-stream-ui)
  app/                  App shell; actions.rs (save_config, clear_history),
                          hydration.rs (start-up load), setup_flow.rs (first-run wizard)
  store.rs              AppSignals (app-wide signals, AppSignals::new) + AppActions
  config_signals.rs     ConfigSignals: the signals that mirror AppConfig; to_config() / apply()
  status_signals.rs     ServiceSignals / SetupSignals / UpdateSignals: backend status, wizard, update dialogs
  view_signals.rs       ChatSignals / UiSignals: chat list, system log, scroll + unread, open dialogs
  chat_view.rs          chat list: per-tab views + limits (ChatStore), filter, paging -- pure, host-tested
  components/           views; settings/ is one file per settings section
  hooks/                backend event, config and tray wiring
  ui_types.rs           ui-only types + re-export of resonance-types (AppConfig included)
```

## Frontend build

- **Frontend:** Leptos 0.8 (CSR) → wasm via **Trunk**; Tailwind 4 + daisyUI through
  `npx @tailwindcss/cli` (Trunk pre-build hook, `cmd /c` — Windows shell).

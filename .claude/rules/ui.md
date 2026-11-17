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
- **The backend is called through its argument struct, never by name:**
  `tauri_bridge::call(&resonance_types::ipc::<Struct> { .. })` returns the command's typed reply.
  A new command gets its struct in the `commands!` list of `crates/types/src/ipc.rs` in the same
  change as its `generate_handler!` entry (and the count that test asserts), or the types test
  `the_backend_registers_exactly_the_commands_listed_here` fails. Events are named through `ipc::event`.

## Chat composer (R7)

- Shown only while `ingame_send` is on (a `<Show>` after `<ChatContainer />`): turning it off drops the draft and error.
- The draft goes out exactly as typed (no prefix, D-35 DS-0001); the backend prepares it (`SendChat { channel, text }`).
  `Sent` and `NotConfirmed` clear the draft; `TooFrequent`, `Cooldown` and `Refused` (the core's `Other` too) keep it.
  `Err` (a pre-send failure) keeps the draft and shows the backend's sentence unchanged; the input is read-only while sending.
- `NotConfirmed` is a neutral notice ("sent, no confirmation"), not an error. A `Cooldown` shows a countdown and blocks Send
  for that client and channel only, never another pair; the pure model (`CooldownBlock`, the status rules) is in `composer_view.rs`.
- Enter sends only when plain: not Shift, not `is_composing`, not key code 229 (IME commit). A 길드/파티/월드/로컬 tab preselects the channel.
- The counter counts UTF-16 units of the prepared text, `n/140` (an emoji is 2).
- While the DLL downloads it shows `게임 내 전송 모듈을 내려받는 중... N%` from `download-progress`, matched by label prefix (`module_line`).
- Logic that can be a pure function goes in `composer_view.rs` with a test; the component only wires signals and events.

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
  composer_view.rs      chat composer logic (counter, can_send, Enter, channel preselect, verdict status, cooldown block) -- pure, host-tested
  components/           views; settings/ is one file per settings section; chat_composer.rs is the composer's wiring
  hooks/                backend event, config and tray wiring
  ui_types.rs           ui-only types + re-export of resonance-types (AppConfig included)
```

## Frontend build

- **Frontend:** Leptos 0.8 (CSR) → wasm via **Trunk**; Tailwind 4 + daisyUI through
  `npx @tailwindcss/cli` (Trunk pre-build hook, `cmd /c` — Windows shell).

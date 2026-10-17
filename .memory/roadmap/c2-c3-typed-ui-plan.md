# C2 / C3 — typed channels, states and tabs; grouped `AppSignals`

**Status: DONE 2026-10-01 (C2a-d, C3a-d merged as PRs #56-#65).** Approved by Kade (order C2a, C2b, C2c, C2d, C3; Beginner stays a
separate feature; the four C3 groups as below). One PR at a time, each merged before the
next starts (CLAUDE.md, *Several tasks in one session*). Both are behaviour-preserving
refactors: no logic, no UI, no wire or file format change (CLAUDE.md, *Refactors do not change
behaviour*). One step = one `claude/*` branch = one PR, test first, each step merges green
before the next starts.

## Why

Three closed sets travel as bare strings, so a typo compiles and the same table is written
several times: channel names (`"WORLD"`, ...: ~130 places), service states (`"Active"`, ...:
~50), tab labels (`"전체"`, ...: ~55). `AppSignals` has 70 signal pairs read by 23 files, and its
40-odd config fields are listed again, field by field, in `actions.rs` (save) and
`hydration.rs` (load). Also W6 in `core-review-2026-09-30.md`: two channel-code tables disagree.

## What must not change (pinned by a test in each step)

| thing | form on the wire / disk | rule |
|---|---|---|
| `ChatMessage.channel` | `"WORLD" "LOCAL" "PARTY" "GUILD"` (camelCase struct, upper-case value) | enum serialises to the same strings; **any other string reads as WORLD** (old `chat_logs/*.jsonl`, `dataset_*.jsonl` and the unmapped code 9 stay loadable) |
| `SnifferStatePayload.state`, `TranslatorStatePayload.state` | `"Off" "Starting" "Pending" "Active" "Error" "Restarting"`; translator also `"Loading Model" "Catching Up"`, `"Ready"` | same strings; unknown reads as `Off` |
| `config.json` | `active_tab` = Korean label, `tab_limits` keys = channel name or `"전체"`/`"커스텀"`, `archive_ignored_channels`, `custom_tab_filters` = channel names | stays strings on disk; the ui/app convert at the edge (`Tab::from_label`/`label`, `Channel::from_str`) |
| the `AppConfig` type | one type in `crates/types` since 2026-10-07 (was two) | unchanged |

## Steps

**C2a — `Channel` in `resonance-types`.** *(done: `claude/c2a-channel-enum`; `ChannelLimits` is keyed by `Channel` now, so limits for names that are no channel are ignored instead of kept; the dead `"SYSTEM"` arm of `chat_row` went; `Tab::Channel` and the tab keys are still strings -- C2c.)* `enum Channel { World, Local, Party, Guild }` with
`as_str`, `from_code(u64)` (the one code table: 2 Local, 3 Party, 4 Guild, else World -- the
parser's `channel_name` and the second table W6 named collapse into it), `FromStr`/serde as above.
`ChatMessage.channel: Channel`. Touches (from grep): types 1, core
`parser.rs` 10, `history.rs` 21 (mostly tests), `pipeline.rs` 1, app `config/app_config.rs` 9,
ui `chat_view.rs` 66, `nav_bar.rs` 11, `chat_row.rs` 4, `store.rs` 5, and the llama-crate
mock (`support/game_server.rs`, `capture.rs`) that builds/reads channel strings. Gates: core, ui, app cross-check.
Test first: serde round trip of every variant + "unknown string -> World" + an old-format
`ChatMessage` JSON still parses.

**C2b — `ServiceState` for sniffer and translator.** *(done: `claude/c2b-service-states`; `SnifferState` Off/Starting/Binding/Pending/Active/Error and `TranslatorState` Off/Starting/Loading Model/Catching Up/Restarting/Active/Error -- the plan's list missed `Binding`, and `Ready` was never a state; unknown reads as Off; the title-bar's `contains("warning")` click test was dead and is gone.)* One enum per service (they differ:
sniffer has `Restarting`; translator has `Loading Model`, `Catching Up`, `Ready`), with the
exact wire strings via `#[serde(rename)]`. `SnifferStatePayload/TranslatorStatePayload.state`,
`AppSignals.sniffer_state/translator_state`, `ServiceStates`. Touches: types 11 hits,
`title_bar.rs` 20, `sniffer/mod.rs` 8, `translator/mod.rs` 4, `server_manager.rs` 2, `use_events.rs`,
`hydration.rs` (`translator_ready`), `network_troubleshooter.rs`, `setup_flow.rs`, core
`workers.rs` (1). The `match ... .as_str()` blocks in `title_bar.rs` become exhaustive matches,
which is the point. Gates: core, ui, app cross-check. Test first: wire strings pinned per variant.

**C2c — `Tab` uses `Channel`.** *(done: `claude/c2c-typed-tabs`; the persisted names `ALL_TAB`/`CUSTOM_TAB`/`SYSTEM_TAB` and `Channel::label`/`from_label` live in `resonance-types` (the app's config defaults need them too); `Channel::ALL` is in the nav/menu order World, Guild, Party, Local; `Tab` got `nav`, `label`, `key`, `icon`, `colors`, `switch_from`, `clear_unread`, so `nav_bar` lost its string tables, the shortcut's 5-arm match and the click handler's match.)* `Tab::Channel(&'static str)` -> `Tab::Channel(Channel)`;
`from_label` and the tab labels (`nav_bar.rs` 28 hits, `chat_view.rs` 16, `chat_container.rs` 3)
move to one `Tab::label`/`from_label` pair; the persisted keys (`"전체"`, `"커스텀"`, channel names)
keep their strings. Ui only (+ core/app hits that only name the keys). Gate: ui (+ cross-check).
The existing `chat_view` tests are the safety net; add `label`/`from_label` round trip.

**C3 — group `AppSignals`.** *(split in PRs: C3a `claude/c3a-config-signals` = `ConfigSignals` (the 33 signals that mirror `AppConfig`, named like its fields: `is_pinned` -> `always_on_top`, `custom_filters` -> `custom_tab_filters`, `opacity` -> `overlay_opacity`) with `to_config()` / `apply()`; `actions.rs` 120 -> 52 lines, `hydration.rs` 302 -> 224, `store.rs` 405 -> 230. **C3b** `claude/c3b-service-signals`: `status_signals.rs` -- `ServiceSignals` (translator/sniffer state and error, `is_sniffer_active`, `model_ready`, `restart_required`, `dict_update_available`), `SetupSignals` (wizard step, status text, downloading, progress), `UpdateSignals` (the app/model update dialogs); the dead `name_cache` signal (used nowhere) removed. **C3c** `claude/c3c-chat-ui-signals`: `view_signals.rs` -- `ChatSignals` (chat, system log, unread, scroll flags, filters, search, `current_time`) and `UiSignals` (settings/dictionary/troubleshooter/favorites modals, `active_menu_id`, `click_through`); the odd setter `set_system_at_bottom` is `set_is_system_at_bottom` now. `AppSignals` is six groups; `store.rs` is 405 -> 110 lines. C3d `claude/c3d-troubleshooter-status`: the troubleshooter's scan status is a private `ScanStatus` enum (its `_ => <div>` fallback was dead). **C2 and C3 are complete.**)* Split by concern, mechanically, into sub-structs held by
`AppSignals`: `ConfigSignals` (everything that mirrors `AppConfig`), `ServiceSignals`
(sniffer/translator state, model/server readiness, restart-required, update info),
`ChatSignals` (store, unread, system log), `UiSignals` (modals, wizard step, pins).
Then `ConfigSignals::to_config()` / `apply(config)` replace the two 40-line field lists in
`actions.rs` and `hydration.rs` -- adding a config field becomes one place in the ui, and a
missed field is a compile error, not a setting that silently does not save. 23 files use
`AppSignals` (17 via `use_context`); the helpers that `let AppSignals { a, set_b, .. } =
signals;` destructure keep working by taking the sub-struct they need. CLAUDE.md's ui
convention is updated with the step. Gate: ui; test first for `to_config`/`apply` round trip
(every field set to a non-default value survives).

**C2d — the other stringly-typed settings** *(split: `claude/c2d1-settings-enums` = `ComputeMode`, `Tier`, `Theme` via a `string_enum!` macro in resonance-types (case-insensitive, unknown -> default); core `workers::gpu_layers` replaces `server_manager`'s string match; the dead `"extreme"` tier class is gone. Part 2, `claude/c2d2-log-level-modifier`: `LogLevel` (the config filter, ordered) and `SystemLogLevel` (message severity, moved from the app into resonance-types with the lowercase wire names; `SystemMessage.level` uses it; the ui's log filter is `severity() >= log_level`). Part 3, `claude/c2d3-tab-switch-modifier`: `TabSwitchModifier` (Ctrl/Alt/Shift/`NoModifier` stored as "None"; default Ctrl) with the accelerator logic in `shortcut_keys.rs` and the app's `shortcut.rs`. **C2 is complete after this.**)* (ideas added on Kade's request, same
recipe: enum, same strings on disk, unknown value -> the default). `ComputeMode` (`"cpu"`,
`"gpu"`, ...: `workers.rs` `TranslatorSettings`, `setup_wizard.rs`, `translation.rs`,
`server_manager.rs`, `store.rs`), `Tier` (`"low" "middle" "high"`: same files),
`Theme` (`"dark" "light"`: `appearance.rs`, `app/mod.rs`), `LogLevel` (`"trace"` .. `"error"`:
`chat_container.rs` 13 hits, `hydration.rs`, `data_dev.rs`, `events.rs`, `commands.rs`;
the ui's log-level filter becomes an ordered comparison instead of a string match),
`TabSwitchModifier` (`"Ctrl" "Alt" "Shift"`: `shortcut_keys.rs`, `shortcut.rs`, `appearance.rs`).
Split into two PRs if it grows: settings enums (`ComputeMode`, `Tier`, `Theme`) and
`LogLevel` + `TabSwitchModifier`. Also folded into C2c: one `Channel::ALL` list feeds the
nav tabs and the custom-tab checkboxes (today each writes the names out), and `ALL_TAB` /
`CUSTOM_TAB` become associated constants of `Tab`.

Found on the way, ui-local and small (do after C3, or fold into it): `network_troubleshooter.rs` keeps its scan status as strings (`"idle" "scanning" "success" "fail"`) -- a private enum.

Not refactors, so **not** in this task (own tasks afterwards, in this order of value):
plain `0x0003` `{3: channel, 5: chat}` history frames (a line is dropped today, see
`core-review-2026-09-30.md`); `Channel::Beginner` (feature); W4 dedup capacity, W6 parser
gaps (`class_id`, `SenderInfo.is_blocked`), W7 IPv6 watchdog, W8 `pick_local_port` race, W9
sniffer busy loop.

## Order and size

C2a -> C2b -> C2c -> C2d -> C3. Each is about one to three sessions of work; C3 is the largest by
files touched and the least risky by logic. After C2a a follow-up *feature* (not part of this
refactor) adds `Channel::Beginner` (code 9, seen in the 2026-10-01 capture): a variant, a
tab and its label -- separate because today code 9 shows as WORLD and adding the variant
changes what the user sees.

## Verification

Linux: `just core-check`, `just ui-check`, `just app-cross-check` per step. The visible
surfaces are the ui (`cargo tauri dev`, Windows, as Administrator): tabs and their unread
badges, title-bar badges for both services through Off -> Starting -> Active -> Error and the
sniffer restart, settings save and reload (every setting survives a restart), chat history
reload (old logs load, channels intact). Kade runs these; anything not run is named `NOT
VERIFIED` in the step's commit body.

## Questions for Kade (answered 2026-10-01: order as given; Beginner separate; groups OK)

1. Order OK (C2a, C2b, C2c, then C3)? Or C3 first (it has no wire risk)?
2. Beginner channel as its own follow-up feature (recommended), or fold `Channel::Beginner`
   into C2a and accept that beginner lines get their own tab/label at the same time?
3. Sub-struct names/grouping for C3: fine as above, or a different split?

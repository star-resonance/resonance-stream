# Roadmap -- known issues since 2026-09-28 (written 2026-10-03)

Tick `[x]` when the item is done **and** verified the way its "Done when" says. A check that
could not run (Windows-only, needs Kade) stays unticked -- CLAUDE.md: never report a gate as
passed that did not run. `K1`..`K25` are this file's ids (not GitHub issue numbers). Each code
item is its own `claude/*` branch and PR, test first. Sources: the `.memory/` notes,
[`../active-issues/unverified-on-windows.md`](../active-issues/unverified-on-windows.md),
[`../sessions/2026-10-02-0.6.0-windows-verification-handoff.md`](../sessions/2026-10-02-0.6.0-windows-verification-handoff.md),
the git log since 09-28, and one user report (K1). The older GitHub issues (#1, #5, #12, #13)
are deliberately not part of this list.

**What 0.6.0 already contains.** Route-based interface pick, per-exe firewall rule, furigana
(+ on'yomi fallback), the wizard scroll fix and the rest of `rc` were merged to `main`
(#87-#91) before the `v0.6.0` tag (#93). So the next release is **fix-only**: its changes are
K1/K3/K20, and it is also the "release newer than 0.6.0" that K2 needs.

---

## Release 0.6.1 -- updates you can trust (P0)

- [x] **K1. Auto-update stuck at 0%** (GitHub #96; friend's report 2026-10-03) -- done and
  verified on Windows by Kade (2026-10-03): `claude/update-download-stall`, merged as #99.
  Stall + connect timeouts for every download, cancel for the app update, an indeterminate bar
  until the first percent, a manual-download button. A guild mate's update succeeded before
  the friend reported, so the friend's cause was probably their network; still unconfirmed.
  **Reach:** only copies on 0.6.1 or later get the fix; a stuck 0.6.0 copy downloads the exe by
  hand once -- the link is in the 0.6.1 notes and the README (`claude/release-0-6-1`).
- [x] **K3. Keep `latest` a signed release** (`release.yml`) -- verified 2026-10-05: v0.6.1's tag run read the feed back, and Kade ran `release-feed-check.yml` by hand (run 37282112213, green) plus the `feed` notebook (feed fields, Korean notes, K3b all pass).
  - What breaks it is a release made *outside* the workflow (the workflow itself always uploads
    `latest.json`, the exe and the `.sig`, and verifies the signature first): a hand-made
    release, a candidate that is not a prerelease, deleting the newest stable one. GitHub then
    makes it "latest", the feed 404s and the app silently finds no update. Nothing can
    *prevent* that, so K3 is a detector.
  - Done in `claude/release-latest-guard` (2026-10-03): `release_feed_problem`
    (`release-lib.sh`) + `check-release-feed.sh` (tested with gh and curl faked, in
    `release-lib.test.sh`, 46 checks); `release.yml` reads the live feed back after publishing;
    new `release-feed-check.yml` (daily 03:17 UTC, on release published / edited / deleted by a
    person, manual); CLAUDE.md *Stable releases* says never to publish by hand. Alert = a red
    Actions run (GitHub's email), no auto-issue; the daily job checks structure only (the
    signature is verified by `release.yml` before publishing).
  - Dry run against the real v0.6.0 release: healthy; a wrong expected tag fails.
  - Stays unticked until the first real runs: `release.yml`'s new last step (at the 0.6.1
    release) and `release-feed-check.yml` once (Actions > Release feed > Run workflow).
- [x] **K3b. Key custody (Kade, offline)** -- Kade answered `pass` in the `feed` notebook (2026-10-05). -- keep `primary` and `backup` key files and their
  passwords in two places each. Losing both strands every 0.6.0 copy (a new key can only be
  introduced by a release signed with an old one). Nothing to commit.
- [x] **K20. Stable release notes** -- replaced by the two-layer notes (CLAUDE.md *Stable
  releases*, `claude/release-notes-layers`). Why 0.6.0's were long: the commit list starts at the
  nearest earlier *stable* tag, which for `v0.6.0` was `v0.5.0` -- 136 commits, 137 lines in the
  in-app dialog, which had no height limit. (An earlier note here blamed "no previous stable
  tag"; that was a shallow clone hiding the old tags.) Now the app shows only the Korean user
  summary, the commit list is collapsed on the release page, and the dialog box scrolls.
- [x] **Cut 0.6.1** -- published; the live feed names 0.6.1 (read by the `feed` notebook, 2026-10-05). (Kade, 2026-10-03: "push 0.6.1 first") -- prepared in `claude/release-0-6-1`:
  `release-notes/v0.6.1.md` (checked with `release_notes_problem`), `[workspace.package] version`
  0.6.1 (+ `Cargo.lock`), the README link for stuck 0.6.0 copies. **Left for Kade, after that PR
  merges: push tag `v0.6.1` on `main`** (publishes to every user). The gist `app` entry is **not**
  touched. Ticks when the tag run is green *and* `release.yml`'s new last step (feed read-back)
  passed -- that also closes K3.
  What 0.6.1 contains besides K1/K3/K20 -- the UI fixes it waited for, all on `main`: row menu
  outside click (#104), cheat sheet + favorites as popup windows (#105-#107), favorites table
  (#108), nav bar order / one-width buttons / translation picker (#110-#112), cheat sheet data:
  nine classes with trees, Korean first with fan names, dungeons by season, tree lines
  (#113-#116). Not run on Windows yet: #108, #110-#116 (Kade ran the popups, #105-#107).

### Windows session W1 (Kade at the keyboard, `cargo tauri dev` / installed exe as Administrator)

Run right after 0.6.1 is published, with 0.6.0 installed. One session covers K2, K4, K6, K7, K25.
Handoff: [`../sessions/2026-10-04-w1-notebooks-handoff.md`](../sessions/2026-10-04-w1-notebooks-handoff.md); plan for
test-run parameters (data dir etc., not started): [`test-run-parameters.md`](test-run-parameters.md).
Each job has a notebook in `runbook/notebooks/` on `main` (until 2026-10-05: its own `test/w1-*` branch, now an archive; the folder was `test-w1/`): `git checkout main && git pull`, then `runbook/README.md`.

- [ ] **K2. Signed update path, end to end** (handoff checks A1-A5). **2026-10-05, `updater-mock` notebook, a local `test-env` build against a mock feed: 45 pass / 0 fail** (new version announced, signed with the backup key, downloaded, restart into a new process on the swapped exe, `.old` kept, `update_temp.exe` gone; tampered file, bad signature, cut and silent downloads refused with no half file; 404 / older / same / not-JSON feeds are no update; the real `config.json` untouched). That covers the update path of the *current* code; what is still open is the hop from a real **0.6.0** install (below).
  - [ ] A1 (the mock run proves the swap for a current build; the real 0.6.0 -> 0.6.1 hop failed once with the old notebook, undiagnosed, and 0.6.0 copies are stuck by K1 anyway -- decide with Kade whether it still matters) installed 0.6.0 offers 0.6.1 -> bar -> "다운로드 완료" only after the check -> restart
    lands on 0.6.1; `<exe>.old` stays, `update_temp.exe` is gone.
  - [x] A2 (mock M4, 2026-10-05) flip one byte of `update_temp.exe` before pressing 재시작 -> error step, old version
    keeps running, no `.old` created.
  - [x] A3 (mock M6 cut, M7 silent, M5 bad signature, 2026-10-05) cut the network mid-download -> error step with a reason, 다시 시도 works, no half
    `update_temp.exe`.
  - [x] A4 (mock M12, 2026-10-05) offline start -> no dialog, "Check failed" in the system log.
  - [x] A5 backup key: sign a file with `backup.key`, run `examples/verify_update` -> `ok`. **Verified on Kade's PC
    (2026-10-04, `test/w1-updater` notebook):** key id `0099CF719FD83012` (the backup), accepted for 0.6.0,
    refused for 0.6.9.
- [ ] **K25. The rest of `unverified-on-windows.md`** -- 2026-10-05: Kade ran `checklists.ipynb` (K7, Esc, K25) and reported `pass 62 / fail 0 / skip 0`, but pasted only that line, not the table, so it is not known which sections he ran. **Ask for the table** (or his list of sections), then delete exactly those bullets; nothing is deleted yet., top to bottom: main UI redesign, rc UI
  fixes, favorite tabs, cheat sheet, chat row menus + add to dictionary, settings sidebar +
  window grow, beginner tab, W8/W9, log dedup, study-mode hover + first-line latency. Delete
  each bullet there as it is checked; a failure gets a test first, then its own branch.

---

## Release 0.6.2 -- capture just works (P1)

Order: verify first (K4, K6, K7 in W1), then build on the answers.

- [ ] **K4. Route-based interface pick on Windows** -- **2026-10-05, `interface` notebook:** (1) pass: `(default route)` Ethernet 192.168.50.220, chat captured. (2) **failed twice, now a separate issue:** NordVPN's `NordLynx` adapter was trusted as the route (fixed: `NordLynx`, `Wintun` added to the keywords, #141), and then with the physical adapter picked chat is **still not captured** while the VPN is on -- a full tunnel hides the game's chat from both adapters ([#142](https://github.com/enjay27/resonance-stream/issues/142)). So the "virtual adapters are not trusted" rule is right but not enough. (3) offline: not run. -- (1) two live adapters: the system tab says
  `(default route)` and that is the adapter the game uses; (2) full-tunnel VPN on: the physical
  adapter is picked and capture still works (**the open question** -- if packets are seen on the
  VPN adapter, the virtual-adapter rule in `pick_interface` is wrong and gets a test + fix);
  (3) offline: no crash, falls back to the list.
- [x] **K6. Per-exe firewall rule on Windows** -- (2026-10-06 Kade: stays manual; an automatic run makes a rule only with `--add-firewall-rule`) verified 2026-10-05 by Kade (`firewall` notebook, 8 pass / 0 fail: dev + release exe, wizard once, chat captured, old shared rule gone, unique names). Original steps: -- the four steps in `unverified-on-windows.md`
  (dev exe -> wizard -> captured; installed exe -> wizard once -> captured; back to dev: no
  wizard; `netsh ... show rule` lists two rules; the old shared rule is gone).
- [ ] **K7. Favorites paste into the game** -- the game or its anti-cheat may ignore `SendInput`.
  If it does: keep the text on the clipboard and say so in the UI (decide with Kade). If the
  game pastes the *old* text: raise `CLIPBOARD_RESTORE_DELAY` (500 ms, `shortcut.rs`).
- [ ] **K5. Adapter-pick follow-ups** (roadmap A questions b, c; depends on K4)
  - [ ] (see also #142: with a full-tunnel VPN no adapter works, so a re-pick cannot help there; the UI should say why) re-pick the adapter after the "no traffic" watchdog trips (pure decision in
    `resonance_core::sniffer_net`, test first);
  - [ ] the troubleshooter (`network_troubleshooter.rs`) tries the routed adapter first.
  - Decision for Kade, optional: create the firewall rule automatically when it is missing (the
    app already runs as Administrator). Not built.

- [x] **K26. Favorites tabs by stable id** (Kade, 2026-10-03: stable ids, not position keys --
  tab order may become movable later; no backup file; a blank tab name becomes "탭 N") --
  plan: [`favorites-stable-ids.md`](favorites-stable-ids.md). Duplicate tab names allowed.
  Built (2026-10-03, `candidate/favorites-stable-ids`, #118 into `rc`) and **verified on Windows by
  Kade (2026-10-04)** with the candidate exe `v0.6.1-rc.favorites-stable-ids`: "works well". He did
  not itemise the six checks, so a failure in one of them (e.g. the shortcut paste, K7) still goes
  through K7 / K25. The same branch goes to `main` in its own PR, then ships in 0.6.2. Not in it:
  renaming / moving a tab.

---

## Release 0.7.0 -- decisions and gaps (P2)

Small and independent first; the ones that wait on Kade last.

- [ ] **K18. Settings open when the app closes leaves the window enlarged** (2026-10-06: `window-restore` K18-quit / K18-close pass 9/9 on a hosted Windows runner, a real exe at 1024x720 -- see [`test-automation.md`](test-automation.md); still unticked until Kade has seen it on his PC; built 2026-10-05, `claude/restore-window-size-on-exit`: unticked until Kade checks it on Windows -- by hand: open Settings, quit from the tray / close the window, start again: the window must come back at its old size; **automatic since 2026-10-05: `python -m runbook.run window-restore`**, the bridge pipeline does both ways out, no clicks) -- the grown size is
  what `tauri-plugin-window-state` saves. Restore before exit. App glue; Windows check.
- [x] **K13. Hydration drops a translation event** (done 2026-10-05, `claude/hydration-translation-race`: `ChatStore::hold_translation` / `take_held_translations`, 3 tests; the race itself was never observed on Windows) that lands during the `get_chat_history`
  fetch for a row that existed before listening (`ChatStore::merge_history`, UI pure module,
  test first).
- [ ] **K19. Archive and tab quirks** -- a message archived untranslated and caught up later is in
  `dataset_raw.jsonl` twice (decide: dedup on write or leave); old beginner lines stay WORLD;
  favorite shortcuts are global across tabs (backend sees one flat list). Decide each, or close.
- [ ] **K17. Build cost of the study-mode dictionary** -- measure the exe / installer growth and
  Windows CI time (release exe is 59 MB); cache the Lindera dictionary in CI, since the build
  needs Lindera.dev reachable.
- [ ] **K14. Compact-mode hover** -- never looked at; the row may re-wrap on hover. Base it on
  current `main` and screenshot with `ui-preview`.
- [x] **K16. Furigana misses** -- done 2026-10-10 (`claude/k16-counter-readings`). The built exe read
  一人 as いち・じん (`CR-ruby-probe`, smoke run 37613640483), and so did a core test: the dictionary is
  embedded, the same on every OS. The cause was the single-kanji on'yomi rule (響奏), not the dictionary.
  Now a run that starts with a numeral keeps the dictionary's readings (三人 さんにん) and 一人 / 二人 are
  ひとり / ふたり; 一人前 stays いちにんまえ. Readings are Claude's; Kade's reference list was not used.
- [ ] **K15. Cheat-sheet data** -- the nine classes with their trees and the season dungeons are
  in (#113-#116, as Kade gave them). Left: check the names against the official JP/KO sites, then
  clear `SOURCE_NOTE`; add fan names as Kade learns them (`Entry.ja`, after the official one);
  add the next season as a new group right after 상시 (the previous one folds by itself).
- [ ] **K12. History from other channels** -- shown as WORLD and queued for translation (~30
  lines per refresh) and may flood the translator. Decide which history to keep (Kade).
- [ ] **K11. W8 gap** -- llama-server binds its port itself, so pick-then-bind cannot be closed;
  a broken model retries 3 times before the error shows. Leave unless seen in the field.
- [ ] **K10. `-t 4` threads** -- decide by measuring FPS with the game running (Kade).
- [ ] **K9. `class_id` is always 0** (sender tag 24) -- needs a capture where the value can be
  matched to a class (Kade; `raw_capture`).
- [ ] **K8. A4: double `<bos>`** -- (2026-10-06: the `translator-stub` pipeline now reports the literal `<bos>` count of every prompt the app sends; it is not judged.) waits on Kade's answer: did training tokenize with BOS
  added (keep the prompt) or not (drop `<bos>` from `translation_prompt`)? Question is in
  [`review-2026-09-30-round2.md`](review-2026-09-30-round2.md).

---

## Housekeeping (P3, any time)

- [ ] **K21. `rc` was merged into `main` wholesale (#91)**, against CLAUDE.md. Decide: record it
  as accepted, or re-cut `rc` from `main` so the rule holds again.
- [ ] **K22. Trim `MEMORY.md`** -- far over its ~40-line rule; move detail into `.memory/`.
- [ ] **K23. Troubleshooting docs** -- the Sep 29 deletion left the README pointing at old
  GitHub issues; write new `TROUBLE_SHOOTING*.md` (Kade) and fix the `index.html` `<title>`
  ("Tauri + Leptos App"; check Tauri does not use it for the window / taskbar first).
- [ ] **K24. Dropped by design, revisit only if asked:** gist host allow-list, reqwest 0.12. (2026-10-06: `audit.yml`, #172, reports reqwest 0.11 findings -- so it is now asked; **Kade has not decided**; needs the app gate on Windows. See [`test-automation.md`](test-automation.md).)

---

## Order at a glance

1. K1 -> K3 -> K20 -> **release 0.6.1**  (K3b alongside, offline) -- prepared, waits for the tag
2. **W1 session:** K2, K4, K6, K7, K25
3. K5, K26 (+ any fix W1 finds) -> **release 0.6.2**
4. K18, K13, K19, K17, K14, K16 -> K15, K12, K11, K10, K9, K8 (Kade's) -> **release 0.7.0**
5. K21-K24 whenever

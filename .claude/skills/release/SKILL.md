---
name: release
description: Ship a resonance-stream build outside the normal claude/* flow - a throw-away test/* branch for Kade's Windows PC, a release candidate through rc (candidate/<feature> PR into rc, prerelease exe), or a signed stable release (version bump, release notes, v<version> tag). Use before cutting any of these branches or tags, or when Kade asks for an rc, a test build or a release.
---

# Release

The rules for release files (never publish by hand, release notes format) are in `.claude/rules/release.md`.

### Test branches (`test/*`)

The runbook lives on `main`, in `runbook/` (see `.claude/rules/runbook.md`); the old
`test/w1-*` branches are an archive and get no updates. A `test/<job>` branch is now only
for a **throw-away** experiment on Kade's Windows PC that does not belong in `main`.
**It is never merged into `main` and never opens a release**: cut it from `main`, push it,
and Kade checks it out. No PR is needed; if one is opened into `main`,
`.github/workflows/test-branch-guard.yml` fails it (helper `branch-guard.sh`, tested in CI).
A finding from a run becomes a normal task on a `claude/*` branch, test first when it is
app logic; the `test/*` branch itself stays out of `main`'s history.

### Release candidates (`rc`)

A change that needs a run on Kade's Windows PC before `main` goes through `rc`:

1. Work on `candidate/<feature>` (from `main`), gate green, push. Open a PR **into `rc`**
   -- Kade or another maintainer merges it by hand (auto-merge never touches it).
2. The merge starts `.github/workflows/release-candidate.yml`: CI's gates, then a
   Windows build of the plain exe (`tauri build --no-bundle`, no installer). **No real-app
   smoke test runs here**, and none runs on a pull request either: it is slow, and it starts
   only from a pushed release tag (`release.yml`; `rc-lib.test.sh` pins that). Then a
   GitHub **prerelease** `v<version>-rc.<feature>` (`.2`, `.3` ... for a repeat
   build of the same branch) with the exe, `SHA256SUMS.txt`, and Korean notes: how to
   run it, the merged PR's description, the commits not yet on `main`, and their
   `NOT VERIFIED` lines. Only the newest 5 candidates (and their tags) are kept.
   Tag / notes logic: `.github/scripts/rc-lib.sh`, tested by `rc-lib.test.sh` (CI).
3. After Kade's test, the **same branch** goes to `main` in its own PR. `rc` is never
   merged into `main`; it is kept current by merging `main` into it.

Prereleases never become "Latest", and the app's update check reads the gist, not
GitHub releases, so users never see a candidate. The candidate exe shares the
installed app's data folder (same identifier): config, model, chat logs.

### Stable releases (signed plain exe)

An app update is installed only if one of the keys built into the app
(`TRUSTED_UPDATE_KEYS`, `crates/core/src/update_signature.rs`) signed it for the
announced version -- so a stable release is built and signed by
`.github/workflows/release.yml`, never by hand: bump `[workspace.package] version`, write
`release-notes/v<version>.md` (copy `release-notes/TEMPLATE.md`), merge to `main`, then
push the tag `v<version>` on it. The workflow gates, runs the same
real-app smoke test -- the only place it runs -- on a separate test-flag build of that commit
(the shipped exe has no bridge; a red row stops the publish), builds
the plain exe, signs it with the `TAURI_SIGNING_PRIVATE_KEY` secret, checks the
signature the way the app will (`examples/verify_update.rs`) and publishes the exe,
`<exe>.sig` and `latest.json` (the update feed). The private keys are never
committed; the backup key stays offline. Tag / feed helpers:
`.github/scripts/release-lib.sh` (tests in CI).


---
paths:
  - ".github/workflows/candidate*.yml"
  - ".github/workflows/promote.yml"
  - "scripts/release*.py"
  - "scripts/test_release.py"
  - "release-notes/**"
  - ".github/scripts/release-lib*.sh"
  - ".github/scripts/check-release-feed.sh"
---

# Release rules

Releases follow the global `release-flow` skill (D-33). Here: the version is `Cargo.toml`
`[workspace.package]`; the notes are `release-notes/v<version>.md` (copy `TEMPLATE.md`); the helpers
are `scripts/release.py` (tests: `python3 -m unittest discover -s scripts -p 'test_*.py'`) and
`.github/scripts/release-lib.sh`.

**The signed exe is built and signed once, in Candidate, which alone holds the signing secrets**
(`TAURI_SIGNING_PRIVATE_KEY`, `..._PASSWORD`). It carries the final version and no test flags; the
`-test.exe` on the prerelease (`test-env`, unsigned, for Kade's debugging) is never promoted. Promote
downloads the files, checks `SHA256SUMS` and the signature against the app's public keys, reuses the
exe and its `.sig`, and writes only the stable `latest.json`: the app installs an update only if a key
in `TRUSTED_UPDATE_KEYS` (`crates/core/src/update_signature.rs`) signed those bytes for that version, and
the feed's URL is not part of that. The private keys are never committed; the backup key stays offline.
The prerelease's `latest.json` is the candidate-channel entry: try it with `--feed-url` on a test-env build.

**Never publish a stable release by hand.** Every installed app reads
`releases/latest/download/latest.json`; a hand-made release (no `latest.json`, another exe) or a
candidate that is not a prerelease becomes "latest" and updates silently stop. `promote.yml` reads the
live feed back, and `release-feed-check.yml` watches it (daily, and when a person publishes, edits or
deletes a release); both run `.github/scripts/check-release-feed.sh`. A red run means users get no
update: delete the hand-made release or mark it prerelease, then run Promote again.

**Release notes: simple for users, detailed for maintainers.** `release-notes/v<version>.md` has two
layers. Above `## 개발자용 상세` is the **user summary**: about 10 plain Korean lines (at most 12), no
commit subjects, PR numbers, file or function names, no English. It is what the update dialog shows
(`latest.json`'s `notes`) and the top of the release page. Below is the **maintainer detail**, shown
collapsed on the release page with the commit list since the previous stable tag. Candidate and Promote
refuse a missing file, an empty or over-long summary, a line without Korean or a `<<작성>>` placeholder
(`release_notes_problem`). The update dialog's notes box scrolls, so a long note cannot hide its buttons.
Prerelease notes are for the tester and keep their own format.

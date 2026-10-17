---
paths:
  - ".github/workflows/release*.yml"
  - "release-notes/**"
  - ".github/scripts/release-lib*.sh"
  - ".github/scripts/check-release-feed.sh"
---

# Release rules

The procedure (test branches, release candidates, stable releases) is the `release` skill (`.claude/skills/release/SKILL.md`).

**Never publish a stable release by hand.** Every installed app reads
`releases/latest/download/latest.json`; a hand-made release (no `latest.json`, another
exe) or a candidate that is not a prerelease becomes "latest" and updates silently stop
-- the app just finds no update. `release.yml` reads the live feed back after publishing,
and `release-feed-check.yml` watches it (daily, and when a person publishes, edits or
deletes a release); both run `.github/scripts/check-release-feed.sh`. A red run means
users get no update: delete the hand-made release or mark it prerelease, then run the
workflow again.

**Release notes: simple for users, detailed for maintainers.** `release-notes/v<version>.md`
has two layers. Above the line `## 개발자용 상세` is the **user summary**: a few plain
lines (about 10, at most 12) in Korean, in everyday words -- no commit subjects, PR
numbers, file or function names, no English. It is what the app's update dialog shows
(`latest.json`'s `notes`) and the top of the release page. Below that line is the
**maintainer detail** (technical, any length, English is fine); the release page puts it,
with the commit list since the previous stable tag, in one collapsed block, and the app
never shows it. `release.yml` refuses to start without the file, or when the summary is
empty, over 12 lines, has a line without Korean, or still has the `<<작성>>` placeholder
(`release_notes_problem`, tested in `release-lib.test.sh`). The update dialog's notes box
also scrolls past a fixed height, so a long note can never push its buttons off screen.
Candidate (`rc`) notes are for the tester and keep their own format.


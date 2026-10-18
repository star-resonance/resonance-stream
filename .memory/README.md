# .memory — which file takes what

**State is the star-resonance Project "Resonance"** (D-32): read
`git fetch origin status && git show origin/status:STATUS.md`, write with a `cmd:` label on the issue or a
Project set run (`CLAUDE.md`, Definition of Done). There is no `MEMORY.md`. This folder is the detail
store, one folder per kind of note; the history is in git and `sessions/`.

| read | when |
|---|---|
| `CLAUDE.md`, this file | rules, gates, layout; which memory file takes what |
| `docs/architecture_review.md` | structure, findings, recommendations by feature |
| `docs/`: `decisions.md`, `security_model.md`, `testing.md`, `refactoring_report.md` | decisions log; trust; tests; before/after |
| `roadmap/` (open items: `roadmap/refactor.md`, "Open items found after S6"), `active-issues/` | what is next and what proves it done; before trusting a doc, a build or a behaviour |
| `sessions/` | why a decision was made, wrong turns included; the old index is `2026-10-06-memory-index-archive.md` |

| folder | takes | rule |
|---|---|---|
| `active-issues/` | things **not to trust yet**: code or docs written but never run on Windows, docs that contradict the code, known bugs | one file per topic; **keep each under ~150 lines** — close items by deleting them, the history is in `sessions/` and git |
| `roadmap/` | what is next, with the check that proves it done; facts with a number | one file per workstream |
| `sessions/` | dated write-ups `YYYY-MM-DD-<topic>.md`, **including wrong turns** | append-only; flat folder; **no new handoff files** (below) |

**No handoff files (from 2026-10-08).** A session ends by updating the issues (a `cmd:` label, a date, an assignee, a new issue); wrong turns go in the PR's *Wrong turns*,
and the `session-handoff` skill gives Kade the starter prompt for the next session. The handoffs already in
`sessions/` stay as history.

Write the note in the same commit as the change it describes.

# .memory — which file takes what

`MEMORY.md` (repo root) is the index. It holds only what would be *false* the moment
it goes stale: what is next, and what is written but not yet verified. **At most 40 lines,
6 KB and 200 characters a line; CI fails above** (`bash .github/scripts/memory-check.sh`). Everything
else lives here, one folder per kind of note. When an item in `MEMORY.md` is done, delete
it; the history is in git and `sessions/`.

| folder | takes | rule |
|---|---|---|
| `active-issues/` | things **not to trust yet**: code or docs written but never run on Windows, docs that contradict the code, known bugs | one file per topic; **keep each under ~150 lines** — close items by deleting them, the history is in `sessions/` and git |
| `roadmap/` | what is next, with the check that proves it done; facts with a number | one file per workstream |
| `sessions/` | dated write-ups `YYYY-MM-DD-<topic>.md`, **including wrong turns** | append-only; flat folder; **no new handoff files** (below) |

**No handoff files (from 2026-10-08).** A session ends by updating the issues' labels (*Now* only for what has no issue); wrong turns go in the PR's *Wrong turns*,
and the `session-handoff` skill gives Kade the starter prompt for the next session. The handoffs already in
`sessions/` stay as history.

Write the note in the same commit as the change it describes.

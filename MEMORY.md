# Active State — resonance-stream

**Index, not the record.** At most 40 lines, 6 KB and 200 characters a line, checked by CI (`memory-check.sh`, D-2).
Only what would be *false* once stale lives here; the rest is in `.memory/`.

## Now — 2026-10-08

**State lives in the star-resonance Project "Resonance"** (since 2026-10-09; this file keeps only what has no issue). A session reads
`git fetch origin status && git show origin/status:STATUS.md` (a snapshot 3 times a day; run `project-snapshot.yml` for a fresh one).
Write with a `cmd:` label on the issue (`cmd:status-now`, `cmd:verify-needs-windows`, `cmd:verify-not-verified`); a decision for Kade is an assignee.

- **Where:** stages S0–S6 and signed metadata M1–M3 merged; open items: [`roadmap/refactor.md`](.memory/roadmap/refactor.md), "Open items found after S6".
- **Next:** S8 (the edges between the parts, `roadmap/refactor.md`): S8a to S8d done; S8e (sniffer dispatch in core behind a `Sink`) needs a plan.
- **Trial:** the Project instead of *Now* until about 2026-10-22; then drop this file's *Now* (and maybe `MEMORY.md`) in its own PR.

**Decided** (detail in `docs/decisions.md`): D-28 signed metadata (`metadata-v1`, `-v2` published), D-29 model → hy-mt2 (K8 moot).
**Stages and risks left:** `roadmap/refactor.md` (S3–S7, W-items).

## Where the detail is

| read | when |
|---|---|
| [`CLAUDE.md`](CLAUDE.md), [`.memory/README.md`](.memory/README.md) | rules, gates, layout; which memory file takes what |
| [`docs/architecture_review.md`](docs/architecture_review.md) | structure, findings, recommendations by feature |
| `docs/`: `decisions.md`, `security_model.md`, `testing.md`, `refactoring_report.md` | decisions log; trust; tests; before/after |
| [`.memory/roadmap/`](.memory/roadmap/), [`.memory/active-issues/`](.memory/active-issues/) | what is next and what proves it done; before trusting a doc, a build or a behaviour |
| [`.memory/sessions/`](.memory/sessions/) | why a decision was made, wrong turns included; the index before the shrink is `2026-10-06-memory-index-archive.md` |

## Rules
- At most 40 lines, 6 KB, 200 characters a line; CI fails above. *Now* carries only what has no issue; detail goes to `.memory/`.
- No handoff files: end a session by updating the issues' labels (`session-handoff` skill). Close an issue when it is done; git and `sessions/` keep the history.

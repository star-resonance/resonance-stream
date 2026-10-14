---
name: workflow-control
description: Plan-first protocol for any change in this repo — plan, approval, execute,
  verify, commit. Use at the start of every task that edits code, manifests, CI or docs.
---

# workflow-control

Four steps, in order. Do not skip ahead.

## 1 · Plan (no edits)

- Locate the code with graft (the `code-graph` account skill), not by reading whole files:
  `g ask "<task>" --source`, `g skeleton <file>`.
- For every symbol you will move, rename, split or change the signature of, run
  `g callers <sym> --depth all`. List the files it reaches — that is the impact.
- Say which part(s) are touched (ui / app, per `CLAUDE.md`) and therefore which gate
  applies, and whether this session can run it (Linux cannot build `src-tauri/`).
- Present: what changes, what does not, the impact list, the gate, the commit(s).

## 2 · Approval

Wait for explicit confirmation. Approval of a plan covers the steps in it, nothing more.
A new step discovered mid-way that widens scope goes back to the developer.

## 3 · Execute

- One task at a time. Refactors are behaviour-preserving: move first, change later,
  never both in one commit.
- After each move, re-run `g callers` on the moved symbol and confirm every caller
  compiles against the new path.
- At most **2** self-corrections on a failing gate; then stop and report.

## 4 · Verify and commit

- Run the gate(s). Report exactly what ran and what could not.
- Update `MEMORY.md` *Now* and the relevant `.memory/` file.
- Work on a `claude/<task>` branch; commit as often as useful, code and memory together,
  per `CLAUDE.md` *Version Control*.
  Unrun gates go in the last commit body as `NOT VERIFIED: ...`. Green gate → push the
  branch and open a PR; CI auto-merges it, unless it is a draft or changes
  `.github/workflows/` (Kade merges those).

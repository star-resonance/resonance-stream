---
paths:
  - "runbook/**"
---

# The runbook

- **The runbook is docs.** `runbook/` -- the Jupyter notebooks Kade runs by hand on his
  Windows PC, their helpers, dry-run tests and fixtures -- changes no app logic, so it is
  treated like a doc: **no CI job checks it** (do not add one), **no gate** for it in the
  table, no test-first requirement, no `NOT VERIFIED` line for it. A PR that only touches
  `runbook/` (and memory) is an ordinary `claude/*` PR; CI still runs on it, because
  auto-merge needs a green run, but nothing in CI tests the runbook. A courtesy, not a gate:
  after changing a pipeline or helper run its own test file (`cd runbook && python -m pytest -q
  tests/test_pipeline_<name>.py`); the full suite takes about 35 minutes (#282).
  The runbook's real proof is Kade's run on Windows, and the report he pastes back says
  what is broken. Fix a notebook in the same PR as the code change it follows.

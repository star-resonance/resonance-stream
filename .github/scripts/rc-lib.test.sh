#!/usr/bin/env bash
# Tests for rc-lib.sh. Run: bash .github/scripts/rc-lib.test.sh
# shellcheck source-path=SCRIPTDIR
set -uo pipefail
cd "$(dirname "$0")" || exit 1
source ./rc-lib.sh

fails=0
eq() { # eq <name> <expected> <actual>
  if [ "$2" == "$3" ]; then echo "ok   $1"; else echo "FAIL $1"; echo "  expected: $2"; echo "  actual:   $3"; fails=$((fails + 1)); fi
}

# --- rc_feature: the merged branch name, without its prefix, tag-safe ---
eq "prefix dropped"            "main-ui-a-cb"   "$(rc_feature candidate/main-ui-a-cb)"
eq "no prefix kept"            "hotfix"         "$(rc_feature hotfix)"
eq "only the first prefix"     "ui-tabs"        "$(rc_feature feat/ui/tabs)"
eq "lowercase, unsafe -> -"    "fix-ui-v2"      "$(rc_feature 'claude/Fix_UI v2')"
eq "runs of - squeezed, trimmed" "a-b"          "$(rc_feature 'x/--a..b--')"
eq "nothing left -> build"     "build"          "$(rc_feature 'candidate/___')"
eq "empty -> build"            "build"          "$(rc_feature '')"

# --- rc_tag: the next free tag, a counter when the feature was built before ---
eq "first build"   "v0.6.0-rc.main-ui"   "$(printf '' | rc_tag 0.6.0 main-ui)"
eq "second build"  "v0.6.0-rc.main-ui.2" "$(printf 'v0.6.0-rc.main-ui\n' | rc_tag 0.6.0 main-ui)"
eq "third build"   "v0.6.0-rc.main-ui.3" "$(printf 'v0.6.0-rc.main-ui.2\nv0.6.0-rc.main-ui\n' | rc_tag 0.6.0 main-ui)"
eq "other features do not count" "v0.6.0-rc.main" "$(printf 'v0.6.0-rc.main-ui\nv0.6.0-rc.main-ui.2\n' | rc_tag 0.6.0 main)"
eq "a gap is reused"  "v0.6.0-rc.x.2" "$(printf 'v0.6.0-rc.x\nv0.6.0-rc.x.3\n' | rc_tag 0.6.0 x)"
eq "another version is free" "v0.7.0-rc.x" "$(printf 'v0.6.0-rc.x\n' | rc_tag 0.7.0 x)"

# --- rc_prune: tags of candidate releases beyond the newest N (input: tag<TAB>created_at) ---
list=$'v0.6.0-rc.a\t2026-10-01T01:00:00Z\nv0.6.0-rc.f\t2026-10-01T06:00:00Z\nv0.6.0-rc.b\t2026-10-01T02:00:00Z\nv0.6.0-rc.e\t2026-10-01T05:00:00Z\nv0.6.0-rc.c\t2026-10-01T03:00:00Z\nv0.6.0-rc.d\t2026-10-01T04:00:00Z\nv0.6.0-rc.g\t2026-10-01T07:00:00Z'
eq "keeps the newest 5" $'v0.6.0-rc.b\nv0.6.0-rc.a' "$(rc_prune 5 <<<"$list")"
eq "nothing to prune"   ""            "$(head -3 <<<"$list" | rc_prune 5)"
eq "stable releases never pruned" "" "$(printf 'v0.5.0\t2026-01-01T00:00:00Z\n' | rc_prune 0)"
# The workflow runs with errexit + pipefail: no candidates at all is not an error.
eq "no candidates under pipefail" "ok" "$(bash -c 'set -eo pipefail; source ./rc-lib.sh; printf "" | rc_prune 5; echo ok')"
eq "no unverified under pipefail" "ok" "$(bash -c 'set -eo pipefail; source ./rc-lib.sh; printf "x\n" | rc_unverified; echo ok')"

# --- rc_unverified: "NOT VERIFIED" lines of commit messages, once each ---
log=$'Subject\n\nbody\nNOT VERIFIED: app gate -- no Windows\n\nother\n\nSecond\nNOT VERIFIED: cargo tauri dev on Windows\nNOT VERIFIED: app gate -- no Windows'
eq "lines, deduplicated" $'- app gate -- no Windows\n- cargo tauri dev on Windows' "$(rc_unverified <<<"$log")"
eq "none"                ""  "$(rc_unverified <<<$'Subject\n\nall good')"
wrapped=$'Subject\n\nNOT VERIFIED: cargo tauri dev on Windows (transparency, drag regions,\n  compact hover bar in the real window) -- Kade runs it.\n\nCo-Authored-By: X <x@y>'
eq "wrapped note joined" "- cargo tauri dev on Windows (transparency, drag regions, compact hover bar in the real window) -- Kade runs it." "$(rc_unverified <<<"$wrapped")"
trailer=$'NOT VERIFIED: app gate\nCo-Authored-By: X <x@y>\nClaude-Session: https://z'
eq "trailer ends a note" "- app gate" "$(rc_unverified <<<"$trailer")"
two=$'NOT VERIFIED: one\nNOT VERIFIED: two\nwraps'
eq "next note ends a note" $'- one\n- two wraps' "$(rc_unverified <<<"$two")"

# --- rc_version: [workspace.package] version, not a dependency's ---
toml=$'[package]\nversion = "9.9.9"\n\n[workspace.package]\nversion = "0.6.0"\n\n[workspace.dependencies]\nserde = { version = "1" }'
eq "workspace version" "0.6.0" "$(rc_version <<<"$toml")"
eq "the repo's own"    "$(sed -n 's/^version = "\(.*\)"/\1/p' ../../Cargo.toml | head -1)" "$(rc_version < ../../Cargo.toml)"

# --- rc_changes: the list of what this merge brings into rc, never an empty section ---
# shellcheck disable=SC2016 # backticks are markdown, not substitutions
eq "commits listed"   $'- a (`1`)\n- b (`2`)' "$(printf -- '- a (`1`)\n- b (`2`)\n' | rc_changes)"
eq "nothing -> (없음)" "- (없음 -- 이전 후보와 같은 내용)" "$(printf '' | rc_changes)"
eq "blank lines only -> (없음)" "- (없음 -- 이전 후보와 같은 내용)" "$(printf '\n\n' | rc_changes)"

# --- rc_rank: which merge the dispatch dropdown picked (1 = the latest) ---
eq "rank from a choice"       "1" "$(rc_rank '1 — 가장 최근 머지')"
eq "rank 3"                   "3" "$(rc_rank '3 — 2개 전 머지')"
eq "rank 5"                   "5" "$(rc_rank '5 — 4개 전 머지')"
eq "empty -> the latest"      "1" "$(rc_rank '')"
eq "push event (no input)"    "1" "$(rc_rank)"
eq "nonsense -> the latest"   "1" "$(rc_rank 'latest please')"
eq "beyond the list -> latest" "1" "$(rc_rank '9 — x')"
eq "zero -> the latest"       "1" "$(rc_rank '0')"

# --- rc_nth_merge: the Nth line of a newest-first commit list; an error past its end ---
shas=$'aaa\nbbb\nccc'
eq "1st is the newest"  "aaa" "$(rc_nth_merge 1 <<<"$shas")"
eq "2nd"                "bbb" "$(rc_nth_merge 2 <<<"$shas")"
eq "last"               "ccc" "$(rc_nth_merge 3 <<<"$shas")"
eq "past the end fails" "failed" "$(rc_nth_merge 4 <<<"$shas" 2>/dev/null || echo failed)"
eq "empty history fails" "failed" "$(printf '' | rc_nth_merge 1 2>/dev/null || echo failed)"
eq "past the end says so" "rc에 4번째 머지가 없습니다 (3개뿐)" "$(rc_nth_merge 4 <<<"$shas" 2>&1 >/dev/null || true)"

# --- test flags (src-tauri `test-env` feature): candidates are built with them, stable releases never ---
workflows=../workflows
eq "candidate builds with test-env" "1" "$(grep -c 'tauri-apps/cli@2 build .*--features test-env' "$workflows/release-candidate.yml")"
eq "stable release never mentions test-env" "0" "$(grep -c 'test-env' "$workflows/release.yml")"
# The real-app smoke test is slow and starts only from the stable release run (release.yml, by hand on rc): no pull request, no push to a branch, no candidate.
eq "candidate does not run the smoke test" "0" "$(cat "$workflows/release-candidate.yml" ../scripts/rc-lib.sh | grep -c 'bridge-smoke\.yml\|needs\.smoke\|rc_smoke_note')"
eq "smoke test has no pull_request trigger" "0" "$(grep -c '^  pull_request' "$workflows/bridge-smoke.yml")"
eq "smoke test has no push trigger" "0" "$(grep -c '^  push' "$workflows/bridge-smoke.yml")"
eq "stable release calls the smoke test once" "1" "$(grep -c 'uses: \./\.github/workflows/bridge-smoke\.yml' "$workflows/release.yml")"
eq "stable release waits for the smoke test" "1" "$(grep -c 'needs: \[check, build, smoke\]' "$workflows/release.yml")"
# Decision D-3: the pipeline steps keep running after one fails (each has its own data folder), but the job still fails, so a release is blocked.
smoke_steps() { # smoke_steps: "<name> <guarded|bare>" for every "over the bridge" step of bridge-smoke.yml
  awk '/^      - name: /{ if (n) print n, (g ? "guarded" : "bare"); n=""; g=0 }
       /^      - name: .*over the bridge/{ n=$0; sub(/^      - name: /, "", n); gsub(/ /, "_", n) }
       n && /^        if: \$\{\{ !cancelled\(\) \}\}$/{ g=1 }
       END{ if (n) print n, (g ? "guarded" : "bare") }' "$workflows/bridge-smoke.yml"
}
eq "smoke test has eight pipeline steps" "8" "$(smoke_steps | wc -l | tr -d ' ')"
eq "every pipeline step runs even after another failed" "0" "$(smoke_steps | grep -c ' bare$')"
eq "smoke test never swallows a failure" "0" "$(grep -c '^ *continue-on-error' "$workflows/bridge-smoke.yml")"
eq "candidate notes name the test flags" "1" "$(grep -c "printf '5\. .*--data-dir" "$workflows/release-candidate.yml")"

echo
if [ "$fails" -eq 0 ]; then echo "all passed"; else echo "$fails failed"; exit 1; fi

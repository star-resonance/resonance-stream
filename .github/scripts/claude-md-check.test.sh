#!/usr/bin/env bash
# Tests for claude-md-check.sh. Run: bash .github/scripts/claude-md-check.test.sh
# shellcheck source-path=SCRIPTDIR
set -uo pipefail
cd "$(dirname "$0")" || exit 1
source ./claude-md-check.sh

fails=0
eq() { # eq <name> <expected> <actual>
  if [ "$2" == "$3" ]; then echo "ok   $1"; else echo "FAIL $1"; echo "  expected: $2"; echo "  actual:   $3"; fails=$((fails + 1)); fi
}
has() { # has <name> <needle> <haystack>
  if [[ "$3" == *"$2"* ]]; then echo "ok   $1"; else echo "FAIL $1"; echo "  wanted to find: $2"; echo "  in:             $3"; fails=$((fails + 1)); fi
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

body() { # body <n> <file>: append n lines of text
  local i
  for ((i = 0; i < $1; i++)); do echo "line $i" >>"$2"; done
}
claude_md() { : >"$tmp/CLAUDE.md"; body "$1" "$tmp/CLAUDE.md"; echo "$tmp/CLAUDE.md"; }
rule() { # rule <total lines> [frontmatter: paths|nopaths|none]: a rules file of exactly that many lines
  local n=$1 kind=${2:-paths} f="$tmp/rule.md"
  case $kind in
    paths)   printf -- '---\npaths:\n  - "src/**"\n---\n' >"$f"; body $((n - 4)) "$f" ;;
    nopaths) printf -- '---\ndescription: x\n---\n' >"$f"; body $((n - 3)) "$f" ;;
    none)    : >"$f"; body "$n" "$f" ;;
  esac
  echo "$f"
}
cproblem() { if claude_md_problem "$1" >/dev/null; then echo yes; else echo no; fi; }
rproblem() { if rule_file_problem "$1" >/dev/null; then echo yes; else echo no; fi; }

# --- CLAUDE.md: at most 100 lines (refactor plan, Kade 2026-10-08) ---
eq "the limits are 100 and 80 lines"   "100 80" "$CLAUDE_MD_MAX_LINES $RULE_MAX_LINES"
eq "100 lines are fine"                no  "$(cproblem "$(claude_md 100)")"
eq "101 lines are too many"            yes "$(cproblem "$(claude_md 101)")"
eq "unterminated last line counts"     yes "$(f=$(claude_md 100); printf 'no newline' >>"$f"; cproblem "$f")"
eq "a missing CLAUDE.md is a problem"  yes "$(cproblem "$tmp/nope.md")"
msg=$(claude_md_problem "$(claude_md 130)")
has "CLAUDE.md: says the count"        "130 lines" "$msg"
has "CLAUDE.md: says the cap"          "100"       "$msg"
has "CLAUDE.md: says where to move it" ".claude/rules/" "$msg"

# --- .claude/rules/*.md: at most 80 lines, and paths: in the frontmatter ---
eq "a rule of 80 lines with paths: is fine" no  "$(rproblem "$(rule 80)")"
eq "a rule of 81 lines is too long"         yes "$(rproblem "$(rule 81)")"
eq "a rule without frontmatter is a problem" yes "$(rproblem "$(rule 10 none)")"
eq "frontmatter without paths: is a problem" yes "$(rproblem "$(rule 10 nopaths)")"
eq "paths: after the frontmatter does not count" yes "$(f=$(rule 10 nopaths); echo 'paths:' >>"$f"; rproblem "$f")"
eq "an unclosed frontmatter is a problem"   yes "$(printf -- '---\npaths:\n  - "x"\n' >"$tmp/u.md"; rproblem "$tmp/u.md")"
eq "CRLF line endings still parse"          no  "$(f=$(rule 10); sed -i 's/$/\r/' "$f"; rproblem "$f")"
msg=$(rule_file_problem "$(rule 90)")
has "long rule: says the count"   "90 lines" "$msg"
has "long rule: says the cap"     "80"       "$msg"
msg=$(rule_file_problem "$(rule 10 nopaths)")
has "no paths: says paths:"       "paths:"   "$msg"

# --- run as a command over a repository: CLAUDE.md plus every rules file ---
repo() { # repo <CLAUDE.md lines> <rule kind>: a fake repository root
  local r=$tmp/repo
  rm -rf "$r"; mkdir -p "$r/.claude/rules"
  body "$1" "$r/CLAUDE.md"
  cp "$(rule 10 "$2")" "$r/.claude/rules/a.md"
  cp "$(rule 10)" "$r/.claude/rules/b.md"
  echo "$r"
}
eq "run as a command: ok on a good repo"     0 "$(bash ./claude-md-check.sh "$(repo 50 paths)" >/dev/null; echo $?)"
eq "run as a command: long CLAUDE.md fails"  1 "$(bash ./claude-md-check.sh "$(repo 150 paths)" >/dev/null 2>&1; echo $?)"
eq "run as a command: a bad rule fails"      1 "$(bash ./claude-md-check.sh "$(repo 50 nopaths)" >/dev/null 2>&1; echo $?)"
out=$(bash ./claude-md-check.sh "$(repo 150 nopaths)" 2>&1)
has "run as a command: names CLAUDE.md" "CLAUDE.md" "$out"
has "run as a command: names the rule"  "a.md"      "$out"
eq "run as a command: no rules folder is fine" 0 "$(r=$(repo 50 paths); rm -rf "$r/.claude"; bash ./claude-md-check.sh "$r" >/dev/null; echo $?)"

[ "$fails" -eq 0 ] && echo "all passed" || { echo "$fails failed"; exit 1; }

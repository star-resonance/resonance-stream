#!/usr/bin/env bash
# Tests for memory-check.sh. Run: bash .github/scripts/memory-check.test.sh
# shellcheck source-path=SCRIPTDIR
set -uo pipefail
cd "$(dirname "$0")" || exit 1
source ./memory-check.sh

fails=0
eq() { # eq <name> <expected> <actual>
  if [ "$2" == "$3" ]; then echo "ok   $1"; else echo "FAIL $1"; echo "  expected: $2"; echo "  actual:   $3"; fails=$((fails + 1)); fi
}
has() { # has <name> <needle> <haystack>
  if [[ "$3" == *"$2"* ]]; then echo "ok   $1"; else echo "FAIL $1"; echo "  wanted to find: $2"; echo "  in:             $3"; fails=$((fails + 1)); fi
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

lines() { # lines <n> <bytes per line, newline included>: a file of n lines
  local n=$1 width=$2 i
  : >"$tmp/f"
  for ((i = 0; i < n; i++)); do printf '%*s\n' $((width - 1)) '' | tr ' ' x >>"$tmp/f"; done
  echo "$tmp/f"
}
problem() { if memory_index_problem "$1" >/dev/null; then echo yes; else echo no; fi; }

# --- limits: 40 lines and 6 KB (6144 bytes) ---
eq "the limits are 40 lines and 6144 bytes" "40 6144" "$MEMORY_MAX_LINES $MEMORY_MAX_BYTES"
eq "an empty file is fine"          no  "$(: >"$tmp/e"; problem "$tmp/e")"
eq "40 short lines are fine"        no  "$(problem "$(lines 40 20)")"
eq "41 lines are too many"          yes "$(problem "$(lines 41 20)")"
eq "exactly 6144 bytes is fine"     no  "$(problem "$(lines 32 192)")"
eq "6145 bytes are too many"        yes "$(f=$(lines 32 192); printf 'x' >>"$f"; problem "$f")"
eq "few lines but over 6 KB"        yes "$(problem "$(lines 20 400)")"
eq "a missing file is a problem"    yes "$(problem "$tmp/does-not-exist")"

# --- the reason names what is wrong, with the numbers and where to put the rest ---
msg=$(memory_index_problem "$(lines 41 20)")
has "too many lines: says lines"    "41 lines"   "$msg"
has "too many lines: says the cap"  "40"         "$msg"
has "points to .memory/"            ".memory/"   "$msg"
msg=$(memory_index_problem "$(lines 20 400)")
has "too many bytes: says bytes"    "8000 bytes" "$msg"
has "too many bytes: says the cap"  "6144"       "$msg"
msg=$(memory_index_problem "$tmp/does-not-exist")
has "missing file is named"         "does-not-exist" "$msg"

# --- a last line without a trailing newline still counts as a line ---
eq "unterminated last line counts"  yes "$(f=$(lines 40 20); printf 'no newline' >>"$f"; problem "$f")"

# --- no line over 200 characters (characters, not bytes: Korean and arrows are multibyte) ---
line_of() { # line_of <n chars> <char>: a 3-line file whose middle line has n copies of char
  local n=$1 c=$2 i out=""
  for ((i = 0; i < n; i++)); do out+=$c; done
  printf 'first\n%s\nlast\n' "$out" >"$tmp/l"
  echo "$tmp/l"
}
eq "the line limit is 200 characters" 200 "$MEMORY_MAX_LINE_CHARS"
eq "a 200-character line is fine"     no  "$(problem "$(line_of 200 x)")"
eq "a 201-character line is too long" yes "$(problem "$(line_of 201 x)")"
eq "200 Korean characters are fine (600 bytes)" no "$(problem "$(line_of 200 가)")"
eq "201 Korean characters are too long" yes "$(problem "$(line_of 201 가)")"
eq "a CRLF line ending is not a character" no "$(f=$(line_of 200 x); sed -i 's/$/\r/' "$f"; problem "$f")"
msg=$(memory_index_problem "$(line_of 250 x)")
has "too long: names the line"        "line 2"     "$msg"
has "too long: says its length"       "250"        "$msg"
has "too long: says the cap"          "200"        "$msg"

# --- the real index passes when the script is run as a command ---
eq "run as a command: ok on a small file" 0 "$(bash ./memory-check.sh "$(lines 3 20)" >/dev/null; echo $?)"
eq "run as a command: fails on a big file" 1 "$(bash ./memory-check.sh "$(lines 99 20)" >/dev/null 2>&1; echo $?)"

[ "$fails" -eq 0 ] && echo "all passed" || { echo "$fails failed"; exit 1; }

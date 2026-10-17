# Keeps CLAUDE.md short and its path-scoped rules loadable. Tested by claude-md-check.test.sh;
# run by the "CLAUDE.md and rules size" step of ci.yml.
#
# CLAUDE.md is loaded into every session, so it holds only what applies to every task here; rules for
# one part of the repository live in .claude/rules/<part>.md and load only when matching files are
# touched, which needs a `paths:` list in the file's frontmatter. Refactor plan (enjay27/claude-skills
# docs/refactor-plan.md, Kade 2026-10-08): CLAUDE.md at most 100 lines, each rules file at most 80.

CLAUDE_MD_MAX_LINES=100
RULE_MAX_LINES=80

_line_count() { awk 'END { print NR }' "$1"; } # counts an unterminated last line too (wc -l would not)

# Each *_problem function succeeds, and says why on stdout, when the file breaks a rule (or cannot be
# read); it fails quietly when the file is fine.
claude_md_problem() {
  local file=$1 lines
  if [ ! -r "$file" ]; then
    echo "$file cannot be read"
    return 0
  fi
  lines=$(_line_count "$file")
  if [ "$lines" -gt "$CLAUDE_MD_MAX_LINES" ]; then
    echo "$file has $lines lines; it may have at most $CLAUDE_MD_MAX_LINES." \
      "Move rules for one part of the repository to .claude/rules/<part>.md (with paths:), procedures to a skill."
    return 0
  fi
  return 1
}

rule_file_problem() {
  local file=$1 lines
  if [ ! -r "$file" ]; then
    echo "$file cannot be read"
    return 0
  fi
  lines=$(_line_count "$file")
  if [ "$lines" -gt "$RULE_MAX_LINES" ]; then
    echo "$file has $lines lines; a rules file may have at most $RULE_MAX_LINES. Split it, or move a procedure to a skill."
    return 0
  fi
  # Frontmatter: the first line is ---, a top-level paths: key, then a closing ---.
  # (In awk, exit jumps to END, so END alone decides.)
  if ! awk '{ sub(/\r$/, "") }
            NR == 1 { if ($0 != "---") exit; next }
            $0 == "---" { closed = 1; exit }
            /^paths:/ { found = 1 }
            END { exit (closed && found) ? 0 : 1 }' "$file"; then
    echo "$file has no paths: in its frontmatter, so it would load in every session." \
      "Start it with ---, a paths: list of globs, and ---."
    return 0
  fi
  return 1
}

# Run as a command: bash .github/scripts/claude-md-check.sh [repository root]   (default: this repository)
if [ "${BASH_SOURCE[0]}" == "$0" ]; then
  root=${1:-$(dirname "$0")/../..}
  bad=0
  if reason=$(claude_md_problem "$root/CLAUDE.md"); then echo "$reason" >&2; bad=1; fi
  for rule in "$root"/.claude/rules/*.md; do
    [ -e "$rule" ] || continue
    if reason=$(rule_file_problem "$rule"); then echo "$reason" >&2; bad=1; fi
  done
  [ "$bad" -eq 0 ] || exit 1
  echo "ok   $root/CLAUDE.md is within $CLAUDE_MD_MAX_LINES lines; every rules file is within $RULE_MAX_LINES lines and has paths:"
fi

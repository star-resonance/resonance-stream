# Keeps MEMORY.md an index. Tested by memory-check.test.sh; run by the "MEMORY.md size" step of ci.yml.
#
# MEMORY.md is the first file a new session reads. It holds only what would be false the moment it goes
# stale (what is next, what is written but not verified); everything else lives under .memory/.
# Decision D-2 (docs/decisions.md): at most 40 lines and 6 KB. No line over 200 characters (refactor plan R3,
# Kade 2026-10-08), so a single line cannot hold a whole paragraph. Characters, not bytes: perl -CSD reads UTF-8.

MEMORY_MAX_LINES=40
MEMORY_MAX_BYTES=6144
MEMORY_MAX_LINE_CHARS=200

# Succeeds, and says why on stdout, when the file breaks the limits (or cannot be read);
# fails quietly when it is within them.
memory_index_problem() {
  local file=$1 lines bytes
  if [ ! -r "$file" ]; then
    echo "$file cannot be read"
    return 0
  fi
  # awk counts an unterminated last line too (wc -l would not).
  lines=$(awk 'END { print NR }' "$file")
  bytes=$(wc -c <"$file" | tr -d '[:space:]')
  if [ "$lines" -gt "$MEMORY_MAX_LINES" ] || [ "$bytes" -gt "$MEMORY_MAX_BYTES" ]; then
    echo "$file has $lines lines and $bytes bytes; the index may have at most $MEMORY_MAX_LINES lines and $MEMORY_MAX_BYTES bytes (6 KB)." \
      "Move the detail to .memory/ (sessions/, roadmap/, active-issues/) and keep one or two lines per item here."
    return 0
  fi
  local long
  long=$(MAX="$MEMORY_MAX_LINE_CHARS" perl -CSD -ne 's/\r?\n\z//; if (length > $ENV{MAX}) { print "$. " . length; exit }' "$file")
  if [ -n "$long" ]; then
    echo "$file line ${long% *} has ${long#* } characters; a line may have at most $MEMORY_MAX_LINE_CHARS." \
      "Keep one short line per item here and move the detail to .memory/."
    return 0
  fi
  return 1
}

# Run as a command: bash .github/scripts/memory-check.sh [file]   (default: MEMORY.md at the repository root)
if [ "${BASH_SOURCE[0]}" == "$0" ]; then
  file=${1:-$(dirname "$0")/../../MEMORY.md}
  if reason=$(memory_index_problem "$file"); then
    echo "$reason" >&2
    exit 1
  fi
  echo "ok   $file is within the index limits ($MEMORY_MAX_LINES lines, $MEMORY_MAX_BYTES bytes, $MEMORY_MAX_LINE_CHARS characters a line)"
fi

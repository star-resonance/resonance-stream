# Helpers for .github/workflows/release-candidate.yml. Tested by rc-lib.test.sh.
# Sourced by bash; every function reads its list input from stdin.

# The feature part of a candidate tag: the merged branch's name without its
# first prefix (candidate/, claude/, feat/ ...), lower case, only [a-z0-9-].
rc_feature() {
  local name=${1#*/}
  name=$(printf '%s' "$name" | tr '[:upper:]' '[:lower:]' | sed -E 's/[^a-z0-9-]+/-/g; s/-+/-/g; s/^-//; s/-$//')
  printf '%s\n' "${name:-build}"
}

# The next free tag v<version>-rc.<feature>[.<n>], given the existing tags.
rc_tag() {
  local version=$1 feature=$2 base tags n
  base="v${version}-rc.${feature}"
  tags=$(cat)
  if ! grep -qxF "$base" <<<"$tags"; then
    printf '%s\n' "$base"
    return
  fi
  n=2
  while grep -qxF "$base.$n" <<<"$tags"; do n=$((n + 1)); done
  printf '%s\n' "$base.$n"
}

# Candidate tags to delete so only the newest <keep> remain.
# Input: "<tag><TAB><created_at ISO-8601>" lines; non-candidate tags are ignored.
rc_prune() {
  local keep=$1
  { grep -E $'^v[^\t]*-rc\\.[^\t]*\t' || true; } | sort -t $'\t' -k2,2r | tail -n +"$((keep + 1))" | cut -f1
}

# The app version: [workspace.package] version of the root Cargo.toml (stdin).
rc_version() {
  sed -n '/^\[workspace\.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' | head -1
}

# How many merges the dispatch dropdown can reach back (its options are 1..5).
RC_PICK_MAX=5

# Which merge into rc a manual run builds: the number the dropdown choice
# starts with ("3 — 2개 전 머지" -> 3). 1 is the latest, and what an empty
# choice, a push, or anything unreadable means.
rc_rank() {
  local n=${1:-}; n=${n%%[!0-9]*}
  if [ -n "$n" ] && [ "$n" -ge 1 ] && [ "$n" -le "$RC_PICK_MAX" ]; then
    printf '%s\n' "$n"
  else
    printf '%s\n' 1
  fi
}

# The <rank>th line of a newest-first commit list (stdin); an error, said in
# Korean on stderr, when the history is shorter than that.
rc_nth_merge() {
  local rank=$1 list count
  list=$(cat)
  count=$(grep -c . <<<"$list" || true)
  if [ "$rank" -gt "$count" ]; then
    echo "rc에 ${rank}번째 머지가 없습니다 (${count}개뿐)" >&2
    return 1
  fi
  sed -n "${rank}p" <<<"$list"
}

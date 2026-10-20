# shellcheck shell=bash
# Helpers for .github/workflows/candidate.yml and promote.yml. Tested by release-lib.test.sh.
# Sourced by bash.

# The version of a stable release tag (v<major>.<minor>.<patch>); anything else
# -- a candidate tag (v0.6.0-rc.x), a typo -- is refused.
release_version() {
  if [[ $1 =~ ^v([0-9]+\.[0-9]+\.[0-9]+)$ ]]; then
    printf '%s\n' "${BASH_REMATCH[1]}"
  else
    printf '%s는 정식 릴리스 태그(vX.Y.Z)가 아닙니다\n' "$1" >&2
    return 1
  fi
}

# May <version> (Cargo.toml's) become a new release? The repository's tags are on
# stdin, one per line; only stable tags (vX.Y.Z) count. Prints the reason and
# returns 1 when the version is not X.Y.Z, is already tagged, or is not above the
# last release; prints nothing and returns 0 when it is new.
release_new_version_problem() {
  local version=$1 tag last
  if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    printf '버전 %s는 X.Y.Z 형식이 아닙니다\n' "$version"
    return 1
  fi
  last=$(grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sed 's/^v//' | sort -t. -k1,1n -k2,2n -k3,3n | tail -n 1)
  if [ "$last" = "$version" ]; then
    printf 'v%s는 이미 릴리스되었습니다. Cargo.toml의 버전을 올리세요\n' "$version"
    return 1
  fi
  if [ -n "$last" ] && [ "$(printf '%s\n%s\n' "$last" "$version" | sort -t. -k1,1n -k2,2n -k3,3n | tail -n 1)" != "$version" ]; then
    printf '%s는 마지막 릴리스 v%s보다 높지 않습니다\n' "$version" "$last"
    return 1
  fi
}

# The exe's file name on the release. The app fetches its signature from the
# same URL plus ".sig", so the two assets are always <name> and <name>.sig.
release_asset() {
  printf 'Resonance-Stream-%s.exe\n' "$1"
}

# Where the app downloads the exe from: <repo> is "owner/name".
release_download_url() {
  printf 'https://github.com/%s/releases/download/%s/%s\n' "$1" "$2" "$(release_asset "$2")"
}

# latest.json, the app's update feed: version, notes (stdin), pub_date, the exe's
# url and the base64 signature (as `tauri signer sign` writes it). The five
# field names are the updater's "dynamic" manifest format, so the official
# plugin could read this file too.
release_manifest() {
  local version=$1 pub_date=$2 url=$3 signature=$4
  jq -n --arg version "$version" --arg pub_date "$pub_date" --arg url "$url" \
    --arg signature "${signature//[$'\r\n']/}" --rawfile notes /dev/stdin \
    '{version: $version, notes: $notes, pub_date: $pub_date, url: $url, signature: $signature}'
}

# Would the app update from release <tag>? The release's asset names (one per
# line) and the live latest.json (stdin) are checked the way the app reads
# them: the three files exist, the feed is for this version, points at this
# release's exe and carries a signature. Prints the reason and returns 1 when
# something is off; prints nothing and returns 0 when it is fine. <slug> is
# the repository, "owner/name".
release_feed_problem() {
  local tag=$1 assets=$2 slug=$3 feed version exe want got
  feed=$(cat)
  if ! version=$(release_version "$tag" 2>/dev/null); then
    printf '%s는 정식 릴리스 태그(vX.Y.Z)가 아닙니다\n' "$tag"
    return 1
  fi
  exe=$(release_asset "$tag")
  for want in latest.json "$exe" "$exe.sig"; do
    if ! grep -Fxq -- "$want" <<<"$assets"; then
      printf '릴리스 %s에 %s 파일이 없습니다\n' "$tag" "$want"
      return 1
    fi
  done
  if ! jq -e 'type == "object"' >/dev/null 2>&1 <<<"$feed"; then
    printf 'latest.json이 JSON 객체가 아닙니다 (앞부분: %s)\n' "$(head -c 80 <<<"$feed" | tr '\n' ' ')"
    return 1
  fi
  got=$(jq -r '(.version // "") | tostring | ltrimstr("v")' <<<"$feed")
  if [ "$got" != "$version" ]; then
    printf 'latest.json의 version이 %s입니다 (릴리스 %s는 %s)\n' "$got" "$tag" "$version"
    return 1
  fi
  got=$(jq -r '(.url // "") | tostring' <<<"$feed")
  want=$(release_download_url "$slug" "$tag")
  if [ "$got" != "$want" ]; then
    printf 'latest.json의 url이 이 릴리스의 exe가 아닙니다: %s (기대: %s)\n' "$got" "$want"
    return 1
  fi
  if [ -z "$(jq -r '(.signature // "") | tostring' <<<"$feed")" ]; then
    printf 'latest.json의 signature가 비어 있습니다\n'
    return 1
  fi
}

# --- Release notes: a short Korean summary for the people who use the app, and
# detail for maintainers. A notes file (release-notes/vX.Y.Z.md) is the summary,
# then optionally a line starting "## 개발자용 상세" and everything below it.
# The summary goes to latest.json (the app's update dialog) and the top of the
# release page; the detail goes in a collapsed block of the release page.
# CRLF (a file saved on Windows) is read like LF.

# The summary: everything before the "## 개발자용 상세" line (stdin -> stdout),
# without leading or trailing blank lines.
release_notes_user_part() {
  tr -d '\r' | awk '
    /^## 개발자용 상세/ { exit }
    /^[[:space:]]*$/ { if (started) blanks++; next }
    { started = 1; for (; blanks > 0; blanks--) print ""; print }'
}

# The detail: the "## 개발자용 상세" line and everything below it; nothing when
# there is no such line.
release_notes_dev_part() {
  tr -d '\r' | awk 'found || /^## 개발자용 상세/ { found = 1; print }'
}

# Is the summary fit to show users? <max> is the most lines it may have (stdin:
# the whole notes file). Prints the reason and returns 1 when it is empty, too
# long, or has a line without Korean text (a pasted English commit list).
release_notes_problem() {
  local max=$1 user count line
  user=$(release_notes_user_part)
  if [ -z "${user//[[:space:]]/}" ]; then
    printf '사용자용 요약이 비어 있습니다 ("## 개발자용 상세" 위에 몇 줄 적어 주세요)\n'
    return 1
  fi
  if grep -q '<<' <<<"$user"; then
    printf '사용자용 요약에 <<...>> 자리 표시가 남아 있습니다\n'
    return 1
  fi
  count=$(grep -c '[^[:space:]]' <<<"$user")
  if [ "$count" -gt "$max" ]; then
    printf '사용자용 요약이 %s줄입니다 (최대 %s줄). 자세한 내용은 "## 개발자용 상세" 아래로 옮기세요\n' "$count" "$max"
    return 1
  fi
  while IFS= read -r line; do
    [ -z "${line//[[:space:]]/}" ] && continue
    if ! LC_ALL=C.UTF-8 grep -qP '[\x{AC00}-\x{D7A3}]' <<<"$line"; then
      printf '사용자용 요약에 한국어가 아닌 줄이 있습니다: %s\n' "$line"
      return 1
    fi
  done <<<"$user"
}

# The release page's text: the summary, then one collapsed block for
# maintainers with the detail part and the commit list. <commits-file> is the
# commit list (markdown); the notes file is on stdin.
release_page_body() {
  local commits=$1 notes
  notes=$(tr -d '\r')
  release_notes_user_part <<<"$notes"
  printf '\n<details>\n<summary>개발자용 상세</summary>\n\n'
  release_notes_dev_part <<<"$notes" | tail -n +2
  printf '\n### 커밋 목록\n\n'
  cat "$commits"
  printf '\n</details>\n'
}

# The nearest earlier stable release tag (vX.Y.Z only: no -rc, -beta, -test
# suffix), where the commit list of <commit> starts; empty when there is none.
# A commit that carries a stable tag itself looks from its parent; an untagged
# one (the commit Candidate or Promote is about to tag) looks from itself. Run in the repository, with its tags fetched.
release_previous_stable_tag() {
  local from=$1
  if git tag --points-at "$1" --list 'v*' | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$'; then
    from="$1^"
  fi
  git describe --tags --abbrev=0 --match 'v[0-9]*.[0-9]*.[0-9]*' --exclude '*-*' "$from" 2>/dev/null || true
}

# "NOT VERIFIED: ..." notes of commit messages as a list, each once. A note
# wrapped over several lines (commit bodies wrap at ~72 columns) is joined; it
# ends at a blank line, the next note or a trailer ("Co-Authored-By: ...").
rc_unverified() {
  awk '
    function flush() { if (note != "" && !seen[note]++) print "- " note; note = ""; on = 0 }
    /^[[:space:]]*NOT VERIFIED:?/ {
      flush(); line = $0
      sub(/^[[:space:]]*NOT VERIFIED:?[[:space:]]*/, "", line)
      note = line; on = 1; next
    }
    on && (/^[[:space:]]*$/ || /^[A-Za-z-]+: /) { flush(); next }
    on { line = $0; gsub(/^[[:space:]]+|[[:space:]]+$/, "", line); note = note " " line }
    END { flush() }
  '
}

# What a merge brings into rc (stdin: one "- subject (`sha`)" line per commit);
# a merge that brings nothing new says so instead of leaving the section empty.
rc_changes() {
  local lines
  lines=$(grep -v '^[[:space:]]*$' || true)
  if [ -n "$lines" ]; then
    printf '%s\n' "$lines"
  else
    printf '%s\n' '- (없음 -- 이전 후보와 같은 내용)'
  fi
}

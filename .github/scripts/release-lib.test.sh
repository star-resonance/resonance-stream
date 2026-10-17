#!/usr/bin/env bash
# Tests for release-lib.sh. Run: bash .github/scripts/release-lib.test.sh
# shellcheck source-path=SCRIPTDIR
set -uo pipefail
cd "$(dirname "$0")" || exit 1
source ./release-lib.sh

fails=0
eq() { # eq <name> <expected> <actual>
  if [ "$2" == "$3" ]; then echo "ok   $1"; else echo "FAIL $1"; echo "  expected: $2"; echo "  actual:   $3"; fails=$((fails + 1)); fi
}

# --- release_version: a stable tag v<major>.<minor>.<patch> gives its version ---
eq "stable tag"            "0.6.1"  "$(release_version v0.6.1)"
eq "multi-digit"           "12.30.4" "$(release_version v12.30.4)"
eq "candidate tag refused" "failed" "$(release_version v0.6.0-rc.main-ui 2>/dev/null || echo failed)"
eq "no v refused"          "failed" "$(release_version 0.6.1 2>/dev/null || echo failed)"
eq "two parts refused"     "failed" "$(release_version v0.6 2>/dev/null || echo failed)"
eq "suffix refused"        "failed" "$(release_version v0.6.1.2 2>/dev/null || echo failed)"
eq "empty refused"         "failed" "$(release_version '' 2>/dev/null || echo failed)"
eq "refusal says why"      "v0.6.0-rc.x는 정식 릴리스 태그(vX.Y.Z)가 아닙니다" "$(release_version v0.6.0-rc.x 2>&1 >/dev/null || true)"

# --- release_asset: the exe's file name on the release ---
eq "asset name" "Resonance-Stream-v0.6.1.exe" "$(release_asset v0.6.1)"

# --- release_download_url: where the app will fetch the exe (and <url>.sig) ---
eq "download url" "https://github.com/o/r/releases/download/v0.6.1/Resonance-Stream-v0.6.1.exe" "$(release_download_url o/r v0.6.1)"

# --- release_manifest: latest.json, the app's update feed (notes on stdin) ---
json=$(printf 'Line one\n"quoted" \\ back\n' | release_manifest 0.6.1 2026-10-02T09:00:00Z https://x/y.exe 'c2lnbmF0dXJl')
eq "version"   "0.6.1"                 "$(jq -r .version <<<"$json")"
eq "pub_date"  "2026-10-02T09:00:00Z"  "$(jq -r .pub_date <<<"$json")"
eq "url"       "https://x/y.exe"       "$(jq -r .url <<<"$json")"
eq "signature" "c2lnbmF0dXJl"          "$(jq -r .signature <<<"$json")"
eq "notes survive quotes and newlines" $'Line one\n"quoted" \\ back' "$(jq -r .notes <<<"$json")"
eq "no notes -> empty string" "" "$(printf '' | release_manifest 0.6.1 2026-10-02T09:00:00Z https://x/y.exe sig | jq -r .notes)"
eq "only the five fields" "notes pub_date signature url version" "$(jq -r 'keys | join(" ")' <<<"$json")"
# The signature file is one line; a trailing newline must not end up in the feed.
eq "signature trimmed" "abc" "$(printf '' | release_manifest 0.6.1 2026-10-02T09:00:00Z https://x/y.exe $'abc\n' | jq -r .signature)"

# --- release_feed_problem: would the app update from this release? (feed on stdin) ---
assets=$'latest.json\nResonance-Stream-v0.6.1.exe\nResonance-Stream-v0.6.1.exe.sig\nSHA256SUMS.txt'
good_feed() { printf '' | release_manifest 0.6.1 2026-10-02T09:00:00Z "$(release_download_url o/r v0.6.1)" c2ln; }
problem() { # problem <tag> <assets> <feed>  -> "rc|reason"
  local out rc
  out=$(release_feed_problem "$1" "$2" o/r <<<"$3" 2>&1); rc=$?
  printf '%s|%s' "$rc" "$out"
}
has() { # has <name> <text> <needle>
  if [[ $2 == *"$3"* ]]; then echo "ok   $1"; else echo "FAIL $1"; echo "  wanted '$3' in: $2"; fails=$((fails + 1)); fi
}

eq "a healthy release has no problem" "0|" "$(problem v0.6.1 "$assets" "$(good_feed)")"

res=$(problem v0.6.1 "${assets/latest.json/}" "$(good_feed)")
eq  "missing latest.json: refused" "1" "${res%%|*}"
has "missing latest.json: named"   "$res" "latest.json"
res=$(problem v0.6.1 "${assets/Resonance-Stream-v0.6.1.exe.sig/}" "$(good_feed)")
eq  "missing .sig: refused" "1" "${res%%|*}"
has "missing .sig: named"   "$res" "Resonance-Stream-v0.6.1.exe.sig"
res=$(problem v0.6.1 $'latest.json\nResonance-Stream-v0.6.1.exe.sig' "$(good_feed)")
eq  "missing exe: refused" "1" "${res%%|*}"
has "missing exe: named"   "$res" "Resonance-Stream-v0.6.1.exe"

old_feed=$(printf '' | release_manifest 0.6.0 2026-10-02T09:00:00Z "$(release_download_url o/r v0.6.1)" c2ln)
res=$(problem v0.6.1 "$assets" "$old_feed")
eq  "feed for another version: refused" "1" "${res%%|*}"
has "feed for another version: both versions named" "$res" "0.6.0"

wrong_url=$(printf '' | release_manifest 0.6.1 2026-10-02T09:00:00Z https://example.com/app.exe c2ln)
res=$(problem v0.6.1 "$assets" "$wrong_url")
eq  "feed url elsewhere: refused" "1" "${res%%|*}"
has "feed url elsewhere: named"   "$res" "url"

no_sig=$(printf '' | release_manifest 0.6.1 2026-10-02T09:00:00Z "$(release_download_url o/r v0.6.1)" '')
res=$(problem v0.6.1 "$assets" "$no_sig")
eq  "empty signature: refused" "1" "${res%%|*}"
has "empty signature: named"   "$res" "signature"

res=$(problem v0.6.1 "$assets" "<html>Not Found</html>")
eq  "feed that is not JSON: refused" "1" "${res%%|*}"
has "feed that is not JSON: named"   "$res" "JSON"
eq  "empty feed: refused" "1" "$(problem v0.6.1 "$assets" "" | cut -d'|' -f1)"
eq  "a candidate tag is never a stable release" "1" "$(problem v0.6.1-rc.x "$assets" "$(good_feed)" | cut -d'|' -f1)"

# --- check-release-feed.sh: the live check, with gh and curl faked ---
fake=$(mktemp -d)
cat > "$fake/gh" <<'FAKE'
#!/usr/bin/env bash
[ -n "${FAKE_GH_FAIL:-}" ] && { echo "HTTP 404" >&2; exit 1; }
printf '%s\n' "$FAKE_RELEASE_JSON"
FAKE
cat > "$fake/curl" <<'FAKE'
#!/usr/bin/env bash
# FAKE_CURL_FAILS=n: the first n calls fail (a feed that is not up yet).
n=$(cat "$FAKE_COUNT" 2>/dev/null || echo 0)
echo $((n + 1)) > "$FAKE_COUNT"
if [ -n "${FAKE_CURL_FAIL:-}" ] || [ "$n" -lt "${FAKE_CURL_FAILS:-0}" ]; then exit 22; fi
printf '%s\n' "$FAKE_FEED"
FAKE
chmod +x "$fake/gh" "$fake/curl"
release_json() { # release_json <tag> -> what GET releases/latest returns
  jq -n --arg tag "$1" --arg exe "$(release_asset "$1")" \
    '{tag_name: $tag, assets: [{name: "latest.json"}, {name: $exe}, {name: ($exe + ".sig")}]}'
}
live() { # live <repo> [tag] [ENV=value ...] -> "rc|output" of check-release-feed.sh
  local out rc
  rm -f "$fake/count"
  out=$(env PATH="$fake:$PATH" FAKE_COUNT="$fake/count" FEED_ATTEMPTS=3 FEED_PAUSE=0 \
    FAKE_RELEASE_JSON="$(release_json v0.6.1)" FAKE_FEED="$(good_feed)" "${@:3}" \
    bash ./check-release-feed.sh "$1" ${2:+"$2"} 2>&1); rc=$?
  printf '%s|%s' "$rc" "$out"
}

res=$(live o/r)
eq "live: a healthy latest release passes" "0" "${res%%|*}"
eq "live: ... with the expected tag" "0" "$(live o/r v0.6.1 | cut -d'|' -f1)"
res=$(live o/r v0.7.0)
eq  "live: latest is not the release just published: fails" "1" "${res%%|*}"
has "live: ... naming both tags" "$res" "v0.6.1"
res=$(live o/r "" FAKE_GH_FAIL=1)
eq  "live: no stable release at all: fails" "1" "${res%%|*}"
has "live: ... is an error annotation" "$res" "::error title="
has "live: ... says why" "$res" "정식 릴리스(latest)가 없습니다"
res=$(live o/r "" FAKE_CURL_FAIL=1)
eq  "live: feed that cannot be fetched (the app's 404): fails" "1" "${res%%|*}"
has "live: ... names the feed" "$res" "latest.json"
eq  "live: a feed that is not up yet is waited for" "0" "$(live o/r v0.6.1 FAKE_CURL_FAILS=2 | cut -d'|' -f1)"
eq  "live: ... but not forever" "1" "$(live o/r v0.6.1 FAKE_CURL_FAILS=3 | cut -d'|' -f1)"
rm -rf "$fake"

# --- release notes: a short Korean summary for users, detail for maintainers ---
notes_file=$'이번 업데이트에서 바뀐 점\n- 업데이트를 받다가 멈추던 문제를 고쳤어요.\n- 업데이트 창에 취소 버튼이 생겼어요.\n\n## 개발자용 상세\n- StallWatch: no data for 30 s ends the download\n- fetch::guarded'
user_expected=$'이번 업데이트에서 바뀐 점\n- 업데이트를 받다가 멈추던 문제를 고쳤어요.\n- 업데이트 창에 취소 버튼이 생겼어요.'
nproblem() { # nproblem <max> <notes> -> "rc|reason"
  local out rc
  out=$(release_notes_problem "$1" <<<"$2" 2>&1); rc=$?
  printf '%s|%s' "$rc" "$out"
}

eq "user part: what comes before the marker" "$user_expected" "$(release_notes_user_part <<<"$notes_file")"
eq "user part: trailing blank lines dropped" "한 줄" "$(printf '한 줄\n\n\n## 개발자용 상세\nx\n' | release_notes_user_part)"
eq "user part: leading blank lines dropped" "한 줄" "$(printf '\n\n한 줄\n' | release_notes_user_part)"
eq "user part: no marker -> the whole file" $'가\n나' "$(printf '가\n나\n' | release_notes_user_part)"
eq "user part: CRLF (a file saved on Windows)" $'가\n나' "$(printf '가\r\n나\r\n\r\n## 개발자용 상세\r\nx\r\n' | release_notes_user_part)"
eq "dev part: from the marker on" $'## 개발자용 상세\n- StallWatch: no data for 30 s ends the download\n- fetch::guarded' "$(release_notes_dev_part <<<"$notes_file")"
eq "dev part: none without the marker" "" "$(printf '가\n나\n' | release_notes_dev_part)"

eq "notes: a short Korean summary passes" "0|" "$(nproblem 12 "$notes_file")"
eq "notes: detail may be English and long" "0|" "$(nproblem 12 "$(printf '가\n## 개발자용 상세\n'; seq 1 50 | sed 's/^/- detail /')")"
eq "notes: CRLF passes" "0|" "$(nproblem 12 "$(printf '가\r\n나\r\n')")"
twelve=$(seq 1 12 | sed 's/^/- 고친 점 /')
thirteen=$(seq 1 13 | sed 's/^/- 고친 점 /')
eq  "notes: exactly the limit passes" "0|" "$(nproblem 12 "$twelve")"
res=$(nproblem 12 "$thirteen")
eq  "notes: one over the limit fails" "1" "${res%%|*}"
has "notes: ... and names the limit" "$res" "12"
res=$(nproblem 12 $'## 개발자용 상세\n- 내용')
eq  "notes: no user part fails" "1" "${res%%|*}"
res=$(nproblem 12 $'- 업데이트가 멈추던 문제를 고쳤어요.\n- The update no longer hangs at 0%')
eq  "notes: an English line fails" "1" "${res%%|*}"
has "notes: ... and shows that line" "$res" "The update no longer hangs"
eq  "notes: an empty file fails" "1" "$(nproblem 12 "" | cut -d'|' -f1)"

commits=$(mktemp)
printf -- '- 첫 커밋\n- second commit\n' > "$commits"
body=$(release_page_body "$commits" <<<"$notes_file")
has "page: the user part comes first" "${body%%<details>*}" "업데이트 창에 취소 버튼이 생겼어요."
eq  "page: ... and the detail is not outside the fold" "no" "$([[ ${body%%<details>*} == *StallWatch* ]] && echo yes || echo no)"
has "page: the fold is titled" "$body" "<summary>개발자용 상세</summary>"
has "page: the dev part is inside the fold" "${body#*<summary>}" "StallWatch: no data for 30 s ends the download"
eq  "page: ... without its marker heading" "no" "$([[ $body == *'## 개발자용 상세'* ]] && echo yes || echo no)"
has "page: the commit list is inside the fold" "${body#*<summary>}" "- second commit"
eq  "page: the fold is closed at the end" "</details>" "$(tail -n 1 <<<"$body")"
eq  "page: exactly one fold" "1" "$(grep -c '<details>' <<<"$body")"
rm -f "$commits"

# An unedited template must not be published.
res=$(nproblem 12 $'- 고친 점을 쉬운 말로 <<작성>>\n')
eq  "notes: a left-over placeholder fails" "1" "${res%%|*}"
has "notes: ... named" "$res" "<<"

# --- release_new_version_problem: may Cargo.toml's version become a release? (tags on stdin) ---
tags=$'v0.5.0\nv0.6.0-rc.updater\nv0.6.1\nv0.6.2\nv0.6.10-rc.main\nmetadata-v3'
vproblem() { # vproblem <version> <tags> -> "rc|reason"
  local out rc
  out=$(release_new_version_problem "$1" <<<"$2" 2>&1); rc=$?
  printf '%s|%s' "$rc" "$out"
}
eq  "new version: a patch above the last release passes" "0|" "$(vproblem 0.6.3 "$tags")"
eq  "new version: a minor above passes" "0|" "$(vproblem 0.7.0 "$tags")"
eq  "new version: compared as numbers, not text" "0|" "$(vproblem 0.6.10 "$tags")"
eq  "new version: candidate tags do not count as releases" "0|" "$(vproblem 0.6.3 $'v0.6.2\nv0.6.3-rc.main')"
eq  "new version: the first release passes" "0|" "$(vproblem 0.1.0 "")"
res=$(vproblem 0.6.2 "$tags")
eq  "new version: already tagged: refused" "1" "${res%%|*}"
has "new version: ... names the tag" "$res" "v0.6.2"
res=$(vproblem 0.6.1 "$tags")
eq  "new version: below the last release: refused" "1" "${res%%|*}"
has "new version: ... names the last release" "$res" "v0.6.2"
res=$(vproblem 0.5.9 $'v0.6.2\nv0.5.0')
eq  "new version: below, with tags out of order: refused" "1" "${res%%|*}"
res=$(vproblem 0.6 "$tags")
eq  "new version: not X.Y.Z: refused" "1" "${res%%|*}"
has "new version: ... shows it" "$res" "0.6"

# --- release_previous_stable_tag: where the commit list starts ---
repo=$(mktemp -d)
(
  cd "$repo" && git init -q . && git config user.email t@t && git config user.name t
  git commit -q --allow-empty -m a && git tag v0.5.0
  git commit -q --allow-empty -m b && git tag v0.5.1-beta && git tag v0.6.0-rc.x
  git commit -q --allow-empty -m c && git tag v0.6.0
  git commit -q --allow-empty -m d && git tag v0.6.1
)
eq "previous stable: skips candidates and betas" "v0.5.0" "$(cd "$repo" && release_previous_stable_tag v0.6.0)"
eq "previous stable: the one right before" "v0.6.0" "$(cd "$repo" && release_previous_stable_tag v0.6.1)"
eq "previous stable: none before the first" "" "$(cd "$repo" && release_previous_stable_tag v0.5.0)"
rm -rf "$repo"

echo
if [ "$fails" -eq 0 ]; then echo "all passed"; else echo "$fails failed"; exit 1; fi

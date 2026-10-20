#!/usr/bin/env bash
# Is the app's update feed healthy? Reads what the app reads -- the newest
# stable release ("latest") and its latest.json -- and fails, loudly, when the
# app would silently find "no update": no stable release, a latest release
# without the feed or the signed exe, a feed for another version.
#
#   check-release-feed.sh <owner/name> [expected-tag]
#
# With <expected-tag> (the release workflow, right after publishing) "latest"
# must be that very release. The feed may take a moment to appear on GitHub's
# side, so it is tried FEED_ATTEMPTS times (default 5), FEED_PAUSE seconds
# apart (default 6). Needs gh (GH_TOKEN), curl and jq. Tested by
# release-lib.test.sh with gh and curl faked.
set -euo pipefail
# shellcheck source=release-lib.sh source-path=SCRIPTDIR
source "$(dirname "${BASH_SOURCE[0]}")/release-lib.sh"

repo=${1:?usage: check-release-feed.sh <owner/name> [expected-tag]}
expected=${2:-}
attempts=${FEED_ATTEMPTS:-5}
pause=${FEED_PAUSE:-6}

# Prints the problem and returns 1, or returns 0.
check_once() {
  local release tag assets feed
  release=$(gh api "repos/$repo/releases/latest") || {
    printf '%s에 정식 릴리스(latest)가 없습니다 -- 앱은 업데이트를 찾지 못합니다\n' "$repo"
    return 1
  }
  tag=$(jq -r .tag_name <<<"$release")
  if [ -n "$expected" ] && [ "$tag" != "$expected" ]; then
    printf 'latest는 %s입니다 (방금 만든 릴리스는 %s)\n' "$tag" "$expected"
    return 1
  fi
  assets=$(jq -r '.assets[].name' <<<"$release")
  feed=$(curl -fsSL "https://github.com/$repo/releases/latest/download/latest.json") || {
    printf '앱이 읽는 latest.json을 받을 수 없습니다 (https://github.com/%s/releases/latest/download/latest.json, 릴리스 %s)\n' "$repo" "$tag"
    return 1
  }
  release_feed_problem "$tag" "$assets" "$repo" <<<"$feed"
}

problem=""
for ((attempt = 1; attempt <= attempts; attempt++)); do
  if problem=$(check_once); then
    echo "ok: the update feed of $repo is healthy"
    exit 0
  fi
  [ "$attempt" -lt "$attempts" ] && sleep "$pause"
done

echo "::error title=앱 업데이트 피드 이상::$problem"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    printf '### 앱 업데이트 피드 이상\n\n%s\n\n' "$problem"
    printf '%s\n' "앱은 \`releases/latest/download/latest.json\`을 읽습니다. 정식 릴리스는 \`promote.yml\`(태그 vX.Y.Z)로만 만드세요. 손으로 만든 릴리스는 지우거나 prerelease로 바꾸고, 필요하면 최신 정식 릴리스의 \`latest.json\`을 다시 올리세요."
  } >> "$GITHUB_STEP_SUMMARY"
fi
exit 1

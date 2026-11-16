#!/usr/bin/env bash
# Candidate's packaging check for the in-game DLL (docs/ingame-design.md section 6, the SHA-256 pin).
#   check-ingame-package.sh <built dll> <sha256> <exe> <target dir>
# The DLL is built first and hashed; the app build pins that hash into the exe. The app downloads
# its own version's DLL, so nothing bundles it. This proves, after the app build and before signing, that:
#   1. <built dll> came from the ingame profile (target/x86_64-pc-windows-msvc/ingame/),
#   2. it still hashes to <sha256> (nothing touched it after it was hashed),
#   3. no resonance_ingame.dll sits under <target dir>/release/ingame/ or beside the exe in <target dir>/release/
#      (loose copies only: Candidate builds with --no-bundle; tauri.conf.json carries no DLL resource, pinned by release-lib.test.sh),
#   4. the exe embeds <sha256> (lowercase hex, an ordinary string literal),
#   5. no other resonance_ingame.dll under <target dir> differs (a release-profile abort DLL, a rebuild).
# One ::error:: line per failure; exit 1 on any, 2 on bad arguments.
set -euo pipefail

if [ "$#" -ne 4 ]; then
  echo "usage: $0 <built dll> <sha256> <exe> <target dir>" >&2
  exit 2
fi
dll=$1 sha=$2 exe=$3 target=$4
if ! [[ "$sha" =~ ^[0-9a-f]{64}$ ]]; then
  echo "the sha256 must be 64 lowercase hex digits: $sha" >&2
  exit 2
fi

bad=0
fail() { echo "::error::$1"; bad=1; }

case "$dll" in
  */target/x86_64-pc-windows-msvc/ingame/resonance_ingame.dll) ;;
  *) fail "$dll is not from the ingame profile (expected .../target/x86_64-pc-windows-msvc/ingame/resonance_ingame.dll)" ;;
esac

if [ ! -f "$dll" ]; then
  fail "the built DLL $dll is missing"
elif [ "$(sha256sum "$dll" | cut -d' ' -f1)" != "$sha" ]; then
  fail "the built DLL $dll changed after it was hashed (expected $sha)"
fi

for bundled in "$target/release/ingame/resonance_ingame.dll" "$target/release/resonance_ingame.dll"; do
  if [ -e "$bundled" ]; then
    fail "the DLL is bundled beside the exe ($bundled): the app downloads it, nothing may carry it beside the exe"
  fi
done

if [ ! -f "$exe" ]; then
  fail "the exe $exe is missing"
elif ! grep -a -F -q -- "$sha" "$exe"; then
  fail "the exe $exe does not embed the DLL's SHA-256 $sha"
fi

while IFS= read -r other; do
  if [ "$(sha256sum "$other" | cut -d' ' -f1)" != "$sha" ]; then
    fail "another resonance_ingame.dll differs from the pinned one: $other"
  fi
done < <(find "$target" -name resonance_ingame.dll -type f 2>/dev/null)

if [ "$bad" -eq 0 ]; then echo "the in-game DLL is the pinned one ($sha)"; fi
exit "$bad"

#!/usr/bin/env bash
# Tests for check-ingame-package.sh. Run: bash .github/scripts/check-ingame-package.test.sh
set -uo pipefail
cd "$(dirname "$0")" || exit 1
script="$PWD/check-ingame-package.sh"

fails=0
eq() { # eq <name> <expected> <actual>
  if [ "$2" == "$3" ]; then echo "ok   $1"; else echo "FAIL $1"; echo "  expected: $2"; echo "  actual:   $3"; fails=$((fails + 1)); fi
}
has() { # has <name> <substring> <text>
  case "$3" in *"$2"*) echo "ok   $1" ;; *) echo "FAIL $1"; echo "  missing: $2"; echo "  in:      $3"; fails=$((fails + 1)) ;; esac
}

built=target/x86_64-pc-windows-msvc/ingame/resonance_ingame.dll
bundled=target/release/ingame/resonance_ingame.dll   # must NOT exist
beside=target/release/resonance_ingame.dll            # must NOT exist

# fixture <dir>: a consistent tree; sets $sha. The "exe" is random bytes with the DLL's hash spliced in.
fixture() {
  local t=$1
  mkdir -p "$t/target/x86_64-pc-windows-msvc/ingame" "$t/target/release"
  head -c 2048 /dev/urandom > "$t/$built"
  sha=$(sha256sum "$t/$built" | cut -d' ' -f1)
  { head -c 512 /dev/urandom; printf '%s' "$sha"; head -c 512 /dev/urandom; } > "$t/target/release/app.exe"
}
# run <dir> [<dll> [<sha>]]: prints "<exit>|<output>"
run() {
  local t=$1 out rc
  out=$(bash "$script" "${2:-$t/$built}" "${3:-$sha}" "$t/target/release/app.exe" "$t/target" 2>&1); rc=$?
  echo "$rc|$out"
}
code() { echo "${1%%|*}"; }
msg() { echo "${1#*|}"; }

t=$(mktemp -d)
fixture "$t/a"; r=$(run "$t/a"); eq "all consistent" 0 "$(code "$r")"

fixture "$t/b"; mkdir -p "$t/b/target/release/ingame"; cp "$t/b/$built" "$t/b/$bundled"
r=$(run "$t/b"); eq "identical bundled copy: exit" 1 "$(code "$r")"; has "identical bundled copy: message" "bundled beside the exe" "$(msg "$r")"

fixture "$t/b2"; mkdir -p "$t/b2/target/release/ingame"; head -c 100 /dev/urandom > "$t/b2/$bundled"
r=$(run "$t/b2"); eq "different bundled copy: exit" 1 "$(code "$r")"

fixture "$t/b3"; cp "$t/b3/$built" "$t/b3/$beside"
r=$(run "$t/b3"); eq "DLL beside the exe: exit" 1 "$(code "$r")"; has "DLL beside the exe: message" "bundled beside the exe" "$(msg "$r")"

fixture "$t/c"; echo changed >> "$t/c/$built"
r=$(run "$t/c"); eq "built DLL rewritten: exit" 1 "$(code "$r")"; has "built DLL rewritten: message" "changed after it was hashed" "$(msg "$r")"

fixture "$t/d"; head -c 2048 /dev/urandom > "$t/d/target/release/app.exe"
r=$(run "$t/d"); eq "exe without the hash: exit" 1 "$(code "$r")"; has "exe without the hash: message" "does not embed" "$(msg "$r")"

fixture "$t/e"; { head -c 512 /dev/urandom; printf '%s' "${sha^^}"; head -c 512 /dev/urandom; } > "$t/e/target/release/app.exe"
r=$(run "$t/e"); eq "upper-case hash in the exe is not the pin" 1 "$(code "$r")"

fixture "$t/f"; mkdir -p "$t/f/target/x86_64-pc-windows-msvc/release"; head -c 100 /dev/urandom > "$t/f/target/x86_64-pc-windows-msvc/release/resonance_ingame.dll"
r=$(run "$t/f"); eq "stray release-profile DLL: exit" 1 "$(code "$r")"; has "stray release-profile DLL: message" "another resonance_ingame.dll" "$(msg "$r")"

fixture "$t/g"; mkdir -p "$t/g/target/x86_64-pc-windows-msvc/release"; cp "$t/g/$built" "$t/g/target/x86_64-pc-windows-msvc/release/resonance_ingame.dll"
r=$(run "$t/g" "$t/g/target/x86_64-pc-windows-msvc/release/resonance_ingame.dll")
eq "DLL path outside the ingame profile: exit" 1 "$(code "$r")"; has "DLL path outside the ingame profile: message" "ingame profile" "$(msg "$r")"

fixture "$t/i"
r=$(run "$t/i" "" "${sha:0:63}"); eq "63-digit sha: exit 2" 2 "$(code "$r")"
r=$(run "$t/i" "" "${sha^^}"); eq "upper-case sha: exit 2" 2 "$(code "$r")"
r=$(bash "$script" "$t/i/$built" "$sha" "$t/i/target/release/app.exe" "$t/i/$bundled" "$t/i/target" 2>&1); eq "the old five-argument call: exit 2" 2 "$?"
rm -rf "$t"

[ "$fails" -eq 0 ] && echo "all passed" || { echo "$fails failed"; exit 1; }

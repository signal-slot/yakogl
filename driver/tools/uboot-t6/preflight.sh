#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# preflight.sh - host-side check before a T6 U-Boot session.
#
#   bash driver/tools/uboot-t6/preflight.sh <image-dir>
#
# Touches no board and no network; prints no environment value. One line per
# check, "ok" or "FAIL"; exit 1 if any FAIL, 2 on a usage error.
#
#   a  T6_UART and T6_HAND are set (names only).
#   b  python3 imports every module prompt-go.py and t6-uart-rfc2217.py
#      import (the RFC 2217 client is that script's own, so pyserial is
#      needed only if one of them imports `serial`).
#   c  each regular file in <image-dir> other than SHA256SUMS and COMMIT is
#      an image with a line in <image-dir>/SHA256SUMS (sha256sum format)
#      whose hash matches, and the build commit in <image-dir>/COMMIT (one
#      sha) is an ancestor of origin/main (git merge-base --is-ancestor on
#      the local ref; nothing is fetched).
#   d  records/ is gitignored.
#   e  if console-check.sh is executable, the command to run it is printed;
#      it is not run.
set -u
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO=${PREFLIGHT_REPO:-$(cd "$HERE/../../.." && pwd)}
if [ $# != 1 ] || [ ! -d "$1" ]; then
	echo "usage: preflight.sh <image-dir>" >&2
	exit 2
fi
DIR=$1
bad=0
ok() { echo "ok   $1"; }
fail() { echo "FAIL $1"; bad=1; }

# a
miss=
for v in T6_UART T6_HAND; do [ -n "${!v:-}" ] || miss="$miss $v"; done
if [ -z "$miss" ]; then ok "a: T6_UART and T6_HAND are set"; else fail "a: not set:$miss"; fi

# b
SRCS=("$HERE/prompt-go.py" "$HERE/../kexec-zephyr/t6-uart-rfc2217.py")
mods=$(cat "${SRCS[@]}" 2>/dev/null |
	sed -n 's/^[[:space:]]*import \([a-zA-Z0-9_]*\).*/\1/p; s/^[[:space:]]*from \([a-zA-Z0-9_]*\) import.*/\1/p' | sort -u)
if ! command -v python3 >/dev/null; then
	fail "b: python3 not found"
elif [ -z "$mods" ]; then
	fail "b: found no imports in prompt-go.py / t6-uart-rfc2217.py"
else
	nomod=
	for m in $mods; do
		python3 -I -c "import $m" >/dev/null 2>&1 || nomod="$nomod $m"
	done
	if [ -z "$nomod" ]; then ok "b: python3 imports the modules prompt-go.py and t6-uart-rfc2217.py need"
	else fail "b: python3 cannot import:$nomod"; fi
fi

# c
sums=$DIR/SHA256SUMS
commit=
[ -f "$DIR/COMMIT" ] && commit=$(tr -d ' \t\r\n' <"$DIR/COMMIT")
imgs=()
while IFS= read -r f; do imgs+=("$f"); done < <(find "$DIR" -maxdepth 1 -type f ! -name SHA256SUMS ! -name COMMIT -printf '%f\n' | sort)
[ ${#imgs[@]} != 0 ] || fail "c: no image in the image directory"
for n in "${imgs[@]}"; do
	if [ ! -s "$DIR/$n" ]; then fail "c: $n is empty"; continue; fi
	want=$(awk -v n="$n" '{f=$2; sub(/^\*/,"",f); sub(/^\.\//,"",f); if (f==n) print $1}' "$sums" 2>/dev/null | head -n1)
	if [ -z "$want" ]; then fail "c: $n has no sha256 in SHA256SUMS"; continue; fi
	have=$(sha256sum "$DIR/$n" | cut -d' ' -f1)
	if [ "$want" = "$have" ]; then ok "c: $n exists, sha256 matches the list"
	else fail "c: $n sha256 differs from SHA256SUMS"; fi
done
if [ -z "$commit" ]; then
	fail "c: no build commit (COMMIT file) in the image directory"
elif ! [[ $commit =~ ^[0-9a-f]{7,40}$ ]]; then
	fail "c: COMMIT is not a hex sha"
elif ! git -C "$REPO" rev-parse -q --verify origin/main >/dev/null 2>&1; then
	fail "c: no origin/main ref here to compare the build commit with"
elif git -C "$REPO" merge-base --is-ancestor "$commit" origin/main 2>/dev/null; then
	ok "c: build commit ${commit:0:8} is reachable from origin/main"
else
	fail "c: build commit ${commit:0:8} is not reachable from origin/main"
fi

# d
if git -C "$REPO" check-ignore -q records/nanopc-t6/x/y; then ok "d: records/ is gitignored"
else fail "d: records/ is not gitignored"; fi

# e
cc=$HERE/console-check.sh
if [ -x "$cc" ]; then ok "e: next, run: bash $cc   (not run here)"
else ok "e: console-check.sh not present or not executable; nothing to run"; fi

if [ $bad = 0 ]; then echo "preflight: all ok"; else echo "preflight: FAIL"; fi
exit $bad

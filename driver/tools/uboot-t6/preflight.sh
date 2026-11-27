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
#   c  <image-dir> holds the images, in one of two layouts:
#      single  each regular file in <image-dir> other than SHA256SUMS and
#              COMMIT is an image with a line in <image-dir>/SHA256SUMS
#              (sha256sum format) whose hash matches.
#      rows    a row image directory (tools/cts/t6-deqp-images.sh's <img-dir>,
#              read by tools/cts/t6-uboot-rows.sh): a first-level directory
#              counts as a label when it holds a zephyr directory, its image
#              is <LABEL>/zephyr/zephyr.bin, checked against a SHA256SUMS at
#              the top (its line names <LABEL>/zephyr/zephyr.bin) or under
#              the label (its line names zephyr/zephyr.bin). A label whose
#              zephyr/ holds no zephyr.bin is named; a top-level file that is
#              not one of the <LABEL>.built files (or a <LABEL>.log beside its .built),
#              SHA256SUMS or COMMIT is
#              named. Both layouts in one directory fail.
#      Either layout needs one <image-dir>/COMMIT (one sha) that is an
#      ancestor of origin/main (git merge-base --is-ancestor on the local
#      ref; nothing is fetched). tools/cts/t6-deqp-images.sh writes both
#      SHA256SUMS and COMMIT; a selective build keeps the other labels'
#      SHA256SUMS lines (sums-merge) and refuses a different COMMIT with
#      exit 2. Its build logs <LABEL>.log sit at the top beside the
#      <LABEL>.built files; one with no matching .built is named.
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
# sumline <sums file> <file as written in it>: the hash of that file, or nothing
sumline() { awk -v n="$2" '{f=$2; sub(/^\*/,"",f); sub(/^\.\//,"",f); if (f==n) print $1}' "$1" 2>/dev/null | head -n1; }
# layout: a first-level directory holding a zephyr directory makes this a row
# image directory; the labels are those directories
labels=()
while IFS= read -r p; do labels+=("${p%/zephyr}"); done < <(find "$DIR" -mindepth 2 -maxdepth 2 -type d -name zephyr -printf '%P\n' 2>/dev/null | sort)
if [ ${#labels[@]} != 0 ]; then
	# rows layout: <LABEL>/zephyr/zephyr.bin per label. Top-level files are
	# the builder's metadata (the <LABEL>.built files, SHA256SUMS, COMMIT);
	# anything else at the top means the directory is not one layout clean.
	# A <LABEL>.log (the build log) is accepted beside its <LABEL>.built.
	strays=()
	while IFS= read -r f; do strays+=("$f"); done < <(find "$DIR" -mindepth 1 -maxdepth 1 -type f ! -name SHA256SUMS ! -name COMMIT ! -name '*.built' -printf '%f\n' | sort | while IFS= read -r f; do
		case $f in *.log) [ -f "$DIR/${f%.log}.built" ] && continue ;; esac
		echo "$f"
	done)
	if [ ${#strays[@]} != 0 ]; then
		fail "c: top-level file of a row image directory, not a <LABEL>.built, SHA256SUMS or COMMIT (a <LABEL>.log needs its .built): ${strays[*]}"
	fi
	for l in "${labels[@]}"; do
		rel=$l/zephyr/zephyr.bin
		if [ ! -e "$DIR/$rel" ]; then fail "c: $rel is missing"; continue; fi
		if [ ! -s "$DIR/$rel" ]; then fail "c: $rel is empty"; continue; fi
		want=$(sumline "$sums" "$rel"); used=$sums
		if [ -z "$want" ] && [ -f "$DIR/$l/SHA256SUMS" ]; then
			want=$(sumline "$DIR/$l/SHA256SUMS" "zephyr/zephyr.bin"); used=$DIR/$l/SHA256SUMS
		fi
		if [ -z "$want" ]; then
			if [ ! -f "$sums" ] && [ ! -f "$DIR/$l/SHA256SUMS" ]; then
				fail "c: $rel has no sha256 in SHA256SUMS: no SHA256SUMS at the top or in $l/"
			else
				fail "c: $rel has no sha256 in SHA256SUMS"
			fi
			continue
		fi
		have=$(sha256sum "$DIR/$rel" | cut -d' ' -f1)
		if [ "$want" = "$have" ]; then ok "c: $rel exists, sha256 matches the list"
		else fail "c: $rel sha256 differs from ${used#"$DIR"/}"; fi
	done
else
	# single-image layout: regular files at the top of <image-dir>
	imgs=()
	while IFS= read -r f; do imgs+=("$f"); done < <(find "$DIR" -maxdepth 1 -type f ! -name SHA256SUMS ! -name COMMIT -printf '%f\n' | sort)
	[ ${#imgs[@]} != 0 ] || fail "c: no image in the image directory"
	for n in "${imgs[@]}"; do
		if [ ! -s "$DIR/$n" ]; then fail "c: $n is empty"; continue; fi
		want=$(sumline "$sums" "$n")
		if [ -z "$want" ]; then fail "c: $n has no sha256 in SHA256SUMS"; continue; fi
		have=$(sha256sum "$DIR/$n" | cut -d' ' -f1)
		if [ "$want" = "$have" ]; then ok "c: $n exists, sha256 matches the list"
		else fail "c: $n sha256 differs from SHA256SUMS"; fi
	done
fi
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

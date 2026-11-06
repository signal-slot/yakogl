#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# pi4-gles3-preflight.sh - read-only host check before a Raspberry Pi 4 dEQP-GLES3 board session: the env, the
# stage 1 package, the table, and that every image of the selected rows is built, rc=0, from the given commit.
# One 'ok'/'FAIL' line per check; exit 0 only if all pass, 1 if a check failed, 2 on a usage error.
#   tools/cts/pi4-gles3-preflight.sh [--commit <sha, 8+ hex>] [--table <file>] [--img-dir <dir>]
#                                    [--rows <a>-<b> | <i,j,...>]
# Defaults: commit = git rev-parse origin/main, table = tools/cts/pi4-gles3-tree.txt, img-dir = $TREE_IMAGE_DIR,
# rows = all. Rows are counted from 0 in table order, the spec grammar of TREE_ROWS (3,5-9).
# The image layout is pi4-deqp-images.sh's: <img-dir>/<LABEL>.built = '<LABEL> rc=<rc> sha256=<hash|none>
# build=<string>'. The build string is `git describe --always --dirty` of the repository at cmake time
# (V3D_BUILD): with no tags the abbreviated commit hash, '<hash>-dirty' for a modified tree, 'unknown' or 'none'
# when git failed. Only a clean hash that is a prefix of (or extends) --commit counts; a '-dirty', 'unknown' or
# 'none' build is a FAIL, because it does not say which commit the image is.
# Writes nothing, uses no network or board; BOARD_UART is only reported set/unset, its value is never printed.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
commit='' table=$HERE/pi4-gles3-tree.txt img=${TREE_IMAGE_DIR:-} rows=''
usage() { echo "usage: $0 [--commit <sha>] [--table <file>] [--img-dir <dir>] [--rows <a>-<b> | <i,j,...>]" >&2; exit 2; }
while [ $# -gt 0 ]; do
	case $1 in
		--commit) [ $# -ge 2 ] || usage; commit=$2; shift 2 ;;
		--table) [ $# -ge 2 ] || usage; table=$2; shift 2 ;;
		--img-dir) [ $# -ge 2 ] || usage; img=$2; shift 2 ;;
		--rows) [ $# -ge 2 ] || usage; rows=$2; shift 2 ;;
		*) usage ;;
	esac
done
if [ -n "$rows" ]; then
	IFS=, read -ra parts <<< "$rows"
	for p in "${parts[@]}"; do
		case $p in
			*[!0-9-]* | '' | -* | *- | *-*-*) echo "bad --rows '$rows'" >&2; usage ;;
		esac
		a=${p%-*}; b=${p#*-}
		if [ "$a" -gt "$b" ]; then echo "bad --rows '$rows': $p runs backwards" >&2; usage; fi
	done
fi
fails=0
ok() { echo "  ok   $1"; }
bad() { echo "  FAIL $1"; fails=$((fails + 1)); }

# (a) DEQP_WORK, ZEPHYR_WS
for v in DEQP_WORK ZEPHYR_WS; do
	eval "val=\${$v:-}"
	if [ -z "$val" ]; then bad "$v is not set"
	elif [ -d "$val" ]; then ok "$v is set, directory exists"
	else bad "$v is set, directory does not exist"; fi
done
# (b) DEQP_MODULE (pi4-deqp-images.sh defaults to gles2: the GLES3 tree needs it spelled out)
if [ "${DEQP_MODULE:-}" = gles3 ]; then ok "DEQP_MODULE is gles3"
else bad "DEQP_MODULE is '${DEQP_MODULE:-}' (unset), not gles3"; fi
# (c) BOARD_UART: set or unset only
if [ -n "${BOARD_UART:-}" ]; then ok "BOARD_UART is set"; else bad "BOARD_UART is unset"; fi
# (d) stage 1 (cts-build-zephyr.sh MODULE=gles3 BOARD=rpi_4b, WORK=$DEQP_WORK)
pkg=build/modules/gles3/libdeqp-gles3-package.a
if [ -n "${DEQP_WORK:-}" ] && [ -d "$DEQP_WORK/cts" ] && [ -f "$DEQP_WORK/$pkg" ]; then
	ok "stage 1 package exists (<DEQP_WORK>/$pkg, <DEQP_WORK>/cts)"
else bad "stage 1 package missing (<DEQP_WORK>/$pkg and <DEQP_WORK>/cts)"; fi
# (e) the table
if [ ! -f "$table" ]; then bad "table file not found"
elif python3 "$HERE/check-gles3-tree.py" "$table" > /dev/null 2>&1; then ok "table passes check-gles3-tree.py (42810 matched once)"
else bad "table fails check-gles3-tree.py"; fi

# the commit
if [ -z "$commit" ]; then commit=$(git -C "$ROOT" rev-parse --verify -q origin/main 2> /dev/null || true); fi
case $commit in
	'') bad "no commit: origin/main does not resolve (give --commit)" ;;
	*[!0-9a-fA-F]*) bad "commit is not hex"; commit='' ;;
	*) if [ ${#commit} -lt 8 ]; then bad "commit is shorter than 8 hex digits"; commit=''; fi ;;
esac
commit=$(printf %s "$commit" | tr 'A-F' 'a-f')

# the labels, in table order, as pi4-deqp-images.sh and board.sh tree name them
labels=()
if [ -f "$table" ]; then
	while IFS= read -r l; do labels+=("$l"); done < <(awk '
		/^# image [A-Za-z0-9]+:/ { l = $3; sub(/:$/, "", l); next }
		/^[[:space:]]*(#|$)/ { next }
		{ n++; print (l != "") ? l : "img" n; l = "" }' "$table")
fi
n=${#labels[@]}
sel=()
if [ "$n" -gt 0 ]; then
	rowsok=1
	for ((i = 0; i < n; i++)); do
		if [ -z "$rows" ]; then sel+=("$i"); continue; fi
		IFS=, read -ra parts <<< "$rows"
		for p in "${parts[@]}"; do
			a=${p%-*}; b=${p#*-}
			if [ "$i" -ge "$a" ] && [ "$i" -le "$b" ]; then sel+=("$i"); break; fi
		done
	done
	if [ -n "$rows" ]; then
		IFS=, read -ra parts <<< "$rows"
		for p in "${parts[@]}"; do
			b=${p#*-}
			if [ "$b" -ge "$n" ]; then bad "rows $rows reach past the table end (rows 0-$((n - 1)))"; rowsok=0; break; fi
		done
	fi
	if [ $rowsok -eq 1 ]; then ok "rows ${rows:-all}: ${#sel[@]} image(s) of $n"; fi
fi

# (f), (g) every selected image
if [ -z "$img" ]; then bad "no image directory (--img-dir or TREE_IMAGE_DIR)"
elif [ ! -d "$img" ]; then bad "image directory does not exist"
elif [ ${#sel[@]} -gt 0 ]; then
	nb=0 nc=0 miss=() builderr=() other=() gap=()
	for i in "${sel[@]}"; do
		l=${labels[$i]}
		f=$img/$l.built
		if [ ! -f "$f" ]; then miss+=("$l"); continue; fi
		line=$(head -1 "$f")
		rc=$(printf %s "$line" | sed -n 's/.* rc=\([^ ]*\).*/\1/p')
		sha=$(printf %s "$line" | sed -n 's/.* sha256=\([^ ]*\).*/\1/p')
		bld=$(printf %s "$line" | sed -n 's/.* build=\(.*\)$/\1/p')
		if [ "$rc" != 0 ] || [ -z "$sha" ] || [ "$sha" = none ]; then builderr+=("$l(rc=${rc:-?} sha256=${sha:-?})"); continue; fi
		nb=$((nb + 1))
		h=''
		case $bld in
			*-dirty) other+=("$l(build $bld: modified tree)"); continue ;;
			'' | none | unknown) gap+=("$l(build '${bld:-}')"); continue ;;
			*-g[0-9a-f]*) h=${bld##*-g} ;;
			*[!0-9a-f]*) other+=("$l(build '$bld' is not a commit)"); continue ;;
			*) h=$bld ;;
		esac
		if [ -z "$commit" ]; then continue; fi
		if [ ${#h} -lt 7 ]; then other+=("$l(build '$bld' too short)"); continue; fi
		case $commit in
			"$h"*) nc=$((nc + 1)) ;;
			*) case $h in "$commit"*) nc=$((nc + 1)) ;; *) other+=("$l(build $h)") ;; esac ;;
		esac
	done
	if [ ${#miss[@]} -gt 0 ]; then bad "image(s) missing <img-dir>/<LABEL>.built: ${miss[*]}"; fi
	if [ ${#builderr[@]} -gt 0 ]; then bad "image(s) not built (rc=0 and a sha256 needed): ${builderr[*]}"; fi
	if [ ${#miss[@]} -eq 0 ] && [ ${#builderr[@]} -eq 0 ]; then ok "all ${#sel[@]} selected image(s) built, rc=0, with a sha256"; fi
	if [ ${#gap[@]} -gt 0 ]; then
		bad "commit not recorded for: ${gap[*]} (the build string is 'unknown'/'none': git describe failed in the build; no other record of the commit)"
	fi
	if [ ${#other[@]} -gt 0 ]; then bad "image(s) not built from commit ${commit:0:8}: ${other[*]}"; fi
	if [ -n "$commit" ] && [ "$nc" -eq "$nb" ] && [ "$nb" -gt 0 ] && [ ${#gap[@]} -eq 0 ] && [ ${#other[@]} -eq 0 ]; then
		ok "all $nc built image(s) are commit ${commit:0:8}"
	fi
fi

if [ $fails -eq 0 ]; then echo "preflight: all ok"; exit 0; fi
echo "preflight: $fails check(s) FAILED"
exit 1

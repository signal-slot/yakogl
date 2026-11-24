#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# am62p-gles3-preflight.sh - read-only host check before an AM62P (Verdin AM62P, PowerVR) dEQP-GLES3 board
# session: the generator and the table, the images of the selected rows, the run bounds and the env.
# One 'ok'/'FAIL' line per check; exit 0 only if all pass, 1 if a check failed, 2 on a usage error.
#   tools/cts/am62p-gles3-preflight.sh [--commit <sha, 8+ hex>] [--table <file>] [--img-dir <dir>]
#                                      [--rows <a>-<b> | <i,j,...>] [--gltest-img <dir>] [--skip-table]
# Defaults: commit = git rev-parse origin/main, table = tools/cts/am62p-gles3-tree.txt,
# img-dir = build/deqp/am62p-gles3-<sha8> (board-stage.sh am62p gles3 names it so; with an arm it is
# ...-n3d0|-n3d1[-idiv0|-idiv1], give that directory), rows = all (the spec grammar of TREE_ROWS, from 0).
# Checks: (a) gen-am62p-gles3-tree.py --check; (b) check-gles3-tree.py on the table (every mustpass case
# matched once); (c) every selected row's image is built rc=0 from --commit; (d) RUN_BOUND >= 900 and
# RETURN_BOUND = 240 in the environment (board.sh's own defaults, 780 s and 180 s for a dEQP image, are not
# enough for the 3D rows); (e) ZEPHYR_WS, DEQP_WORK or GLES3_WORK, PVR_FW_BLOB, BOARD_UART set or unset (a value
# is never printed); (f) the DEQP_PVR_NATIVE_3D arm, read from the image directory name.
# The image layout is am62p-deqp-images.sh's: a row with a '# image <LABEL>:' line in the table has
# <img-dir>/<LABEL>.built = '<LABEL> rc=<rc> sha256=<hash|none>' (a 'build=<describe>' field, if present, must be
# a clean hash of the commit); any other row is 'img<N> rc=<rc> bin=<hash|none>' in <img-dir>/build.out (the
# last such line wins). The commit identity of the images is the directory name's <sha8>: a '-dirty' name, a
# name without a hash, or a row directory img<N>-<tag> with another tag is a FAIL.
# Injection (for the selftest): PREFLIGHT_GEN and PREFLIGHT_CHECK name commands that replace (a) and (b)
# (the latter gets the table as its argument); --skip-table leaves (a) and (b) out, printed as 'skip'.
# Writes nothing, uses no network or board; no value of the env or path of the layout is printed.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
commit='' table=$HERE/am62p-gles3-tree.txt img='' rows='' gltest='' skiptable=0
usage() { echo "usage: $0 [--commit <sha>] [--table <file>] [--img-dir <dir>] [--rows <a>-<b> | <i,j,...>] [--gltest-img <dir>] [--skip-table]" >&2; exit 2; }
while [ $# -gt 0 ]; do
	case $1 in
		--commit) [ $# -ge 2 ] || usage; commit=$2; shift 2 ;;
		--table) [ $# -ge 2 ] || usage; table=$2; shift 2 ;;
		--img-dir) [ $# -ge 2 ] || usage; img=$2; shift 2 ;;
		--rows) [ $# -ge 2 ] || usage; rows=$2; shift 2 ;;
		--gltest-img) [ $# -ge 2 ] || usage; gltest=$2; shift 2 ;;
		--skip-table) skiptable=1; shift ;;
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

# the commit
if [ -z "$commit" ]; then commit=$(git -C "$ROOT" rev-parse --verify -q origin/main 2> /dev/null || true); fi
case $commit in
	'') bad "no commit: origin/main does not resolve (give --commit)" ;;
	*[!0-9a-fA-F]*) bad "commit is not hex"; commit='' ;;
	*) if [ ${#commit} -lt 8 ]; then bad "commit is shorter than 8 hex digits"; commit=''; fi ;;
esac
commit=$(printf %s "$commit" | tr 'A-F' 'a-f')
sha8=${commit:0:8}
if [ -z "$img" ] && [ -n "$sha8" ]; then img=$ROOT/build/deqp/am62p-gles3-$sha8; fi

# (a), (b) the generator and the table
if [ $skiptable -eq 1 ]; then
	echo "  skip generator and table checks (--skip-table)"
else
	if [ -n "${PREFLIGHT_GEN:-}" ]; then gcmd=("$PREFLIGHT_GEN"); else gcmd=(python3 "$HERE/gen-am62p-gles3-tree.py" --check); fi
	if "${gcmd[@]}" > /dev/null 2>&1; then ok "gen-am62p-gles3-tree.py --check: the table is the generator's output"
	else bad "gen-am62p-gles3-tree.py --check fails (the table is not the generator's output)"; fi
	if [ ! -f "$table" ]; then bad "table file not found"
	else
		if [ -n "${PREFLIGHT_CHECK:-}" ]; then ccmd=("$PREFLIGHT_CHECK" "$table"); else ccmd=(python3 "$HERE/check-gles3-tree.py" "$table"); fi
		if "${ccmd[@]}" > /dev/null 2>&1; then ok "table passes check-gles3-tree.py (every mustpass case matched once)"
		else bad "table fails check-gles3-tree.py"; fi
	fi
fi

# the rows, in table order, as am62p-deqp-images.sh names them: N<TAB>label<TAB>labelled
rowlist=()
if [ -f "$table" ]; then
	while IFS= read -r l; do rowlist+=("$l"); done < <(awk '
		/^# image [A-Za-z0-9]+:/ { l = $3; sub(/:$/, "", l); next }
		/^[[:space:]]*(#|$)/ { next }
		{ n++; printf "%d\t%s\t%d\n", n, l, (l != ""); l = "" }' "$table")
fi
n=${#rowlist[@]}
sel=()
if [ "$n" -gt 0 ]; then
	rowsok=1
	if [ -n "$rows" ]; then
		IFS=, read -ra parts <<< "$rows"
		for p in "${parts[@]}"; do
			b=${p#*-}
			if [ "$b" -ge "$n" ]; then bad "rows $rows reach past the table end (rows 0-$((n - 1)))"; rowsok=0; break; fi
		done
	fi
	for ((i = 0; i < n; i++)); do
		if [ -z "$rows" ]; then sel+=("$i"); continue; fi
		for p in "${parts[@]}"; do
			a=${p%-*}; b=${p#*-}
			if [ "$i" -ge "$a" ] && [ "$i" -le "$b" ]; then sel+=("$i"); break; fi
		done
	done
	if [ $rowsok -eq 1 ]; then ok "rows ${rows:-all}: ${#sel[@]} image(s) of $n"; fi
else
	bad "table has no rows"
fi

# (c), (f) the image directory
if [ -z "$img" ]; then bad "no image directory (--img-dir, or a --commit to name it)"
elif [ ! -d "$img" ]; then bad "image directory does not exist"
else
	name=$(basename "$img")
	dtag='' arm=''
	case $name in
		*-dirty*) bad "image directory name says a modified tree ('-dirty')" ;;
		am62p-gles3-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f] | am62p-gles3-[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]-*)
			dtag=${name#am62p-gles3-}; dtag=${dtag:0:8}; arm=${name#am62p-gles3-????????}
			case $arm in
				'' | -n3d[01] | -n3d[01]-idiv[01] | -idiv[01]) ;;
				*) bad "image directory name has an unknown suffix '$arm'"; dtag='' ;;
			esac ;;
		*) bad "image directory name carries no build identity (am62p-gles3-<sha8>[-n3d0|-n3d1][-idiv0|-idiv1])" ;;
	esac
	# (f) the native 3D arm
	case $arm in
		*-n3d1*) ok "arm: DEQP_PVR_NATIVE_3D=1 (native 3D textures)" ;;
		*-n3d0*) ok "arm: DEQP_PVR_NATIVE_3D=0 (slice-grid 3D textures)" ;;
		*) ok "arm: DEQP_PVR_NATIVE_3D unset (the library default)" ;;
	esac
	case $arm in
		*-idiv1*) ok "arm: DEQP_PVR_INSTANCE_DIVISOR=1" ;;
		*-idiv0*) ok "arm: DEQP_PVR_INSTANCE_DIVISOR=0" ;;
	esac
	if [ -n "$dtag" ] && [ -n "$sha8" ]; then
		if [ "$dtag" = "$sha8" ]; then ok "image directory is commit $sha8"
		else bad "image directory is commit $dtag, not $sha8"; fi
	fi
	if [ ${#sel[@]} -gt 0 ]; then
		nb=0 miss=() builderr=() other=() gap=()
		for i in "${sel[@]}"; do
			IFS=$'\t' read -r rn lab labelled <<< "${rowlist[$i]}"
			if [ "$labelled" = 1 ]; then
				id=$lab
				line=$(head -1 "$img/$lab.built" 2> /dev/null || true)
			else
				id=img$rn
				line=$(grep -a "^img$rn rc=" "$img/build.out" 2> /dev/null | tail -1)
				# a row directory of another tag is another build's image
				if [ -n "$dtag" ]; then
					for d in "$img"/img$rn-*; do
						[ -e "$d" ] || continue
						if [ "${d##*/}" != "img$rn-$dtag" ]; then other+=("$id(directory ${d##*/})"); fi
					done
				fi
			fi
			if [ -z "$line" ]; then miss+=("$id"); continue; fi
			rc=$(printf %s "$line" | sed -n 's/^[^ ]* rc=\([^ ]*\).*/\1/p')
			sha=$(printf %s "$line" | sed -n 's/.* \(sha256\|bin\)=\([^ ]*\).*/\2/p')
			bld=$(printf %s "$line" | sed -n 's/.* build=\(.*\)$/\1/p')
			if [ "$rc" != 0 ] || [ -z "$sha" ] || [ "$sha" = none ]; then builderr+=("$id(rc=${rc:-?} image=${sha:-?})"); continue; fi
			nb=$((nb + 1))
			case $bld in
				'') ;;
				*-dirty) other+=("$id(build $bld: modified tree)") ;;
				none | unknown) gap+=("$id(build '$bld')") ;;
				*[!0-9a-f]*) other+=("$id(build '$bld' is not a commit)") ;;
				*) if [ -n "$commit" ]; then
					case $commit in
						"$bld"*) ;;
						*) case $bld in "$commit"*) ;; *) other+=("$id(build $bld)") ;; esac ;;
					esac
				fi ;;
			esac
		done
		if [ ${#miss[@]} -gt 0 ]; then bad "image(s) without a build record (<LABEL>.built or build.out line): ${miss[*]}"; fi
		if [ ${#builderr[@]} -gt 0 ]; then bad "image(s) not built (rc=0 and an image hash needed): ${builderr[*]}"; fi
		if [ ${#miss[@]} -eq 0 ] && [ ${#builderr[@]} -eq 0 ]; then ok "all ${#sel[@]} selected image(s) built, rc=0, with an image hash"; fi
		if [ ${#gap[@]} -gt 0 ]; then bad "commit not recorded for: ${gap[*]}"; fi
		if [ ${#other[@]} -gt 0 ]; then bad "image(s) not built from commit ${sha8:-?}: ${other[*]}"; fi
		if [ -n "$sha8" ] && [ "${dtag:-}" = "$sha8" ] && [ ${#gap[@]} -eq 0 ] && [ ${#other[@]} -eq 0 ] && [ "$nb" -eq "${#sel[@]}" ]; then
			ok "all $nb built image(s) are commit $sha8"
		fi
	fi
fi

# (d) the run bounds
rb=${RUN_BOUND:-} tb=${RETURN_BOUND:-}
case $rb in
	'') bad "RUN_BOUND is not set (board.sh's default, 780 s, ends a long 3D row early; set 900 or more)" ;;
	*[!0-9]*) bad "RUN_BOUND is not a number" ;;
	*) if [ "$rb" -ge 900 ]; then ok "RUN_BOUND is $rb (>= 900)"; else bad "RUN_BOUND is $rb, below 900"; fi ;;
esac
case $tb in
	'') bad "RETURN_BOUND is not set (board.sh's default is 180 s; the session wants 240)" ;;
	*[!0-9]*) bad "RETURN_BOUND is not a number" ;;
	*) if [ "$tb" -eq 240 ]; then ok "RETURN_BOUND is 240"; else bad "RETURN_BOUND is $tb, not 240"; fi ;;
esac

# (e) the env: set or unset only
for v in ZEPHYR_WS PVR_FW_BLOB BOARD_UART; do
	eval "val=\${$v:-}"
	if [ -n "$val" ]; then ok "$v is set"; else bad "$v is unset"; fi
done
if [ -n "${DEQP_WORK:-}" ]; then ok "DEQP_WORK is set"
elif [ -n "${GLES3_WORK:-}" ]; then ok "GLES3_WORK is set"
else bad "neither DEQP_WORK nor GLES3_WORK is set"; fi

# the gl_test image directory, if asked
if [ -n "$gltest" ]; then
	if [ ! -d "$gltest" ]; then bad "gl_test image directory does not exist"
	elif [ -z "$(ls -A "$gltest" 2> /dev/null)" ]; then bad "gl_test image directory is empty"
	else
		gb=$(grep -L ' rc=0 ' "$gltest"/*.built 2> /dev/null || true)
		if [ -n "$gb" ]; then bad "gl_test image record(s) without rc=0"
		else ok "gl_test image directory exists and is not empty"; fi
	fi
fi

if [ $fails -eq 0 ]; then echo "preflight: all ok"; exit 0; fi
echo "preflight: $fails check(s) FAILED"
exit 1

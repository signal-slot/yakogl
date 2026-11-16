#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Stage 2 of dEQP on Zephyr for a board whose tree runs one image per row:
# one driver/samples/zephyr/deqp image per "# image <LABEL>:" block of a
# table, from this checkout as it is (Step 1 of driver/docs/one-method.md:
# running and tallying the images is tools/board.sh <board> tree). The
# per-board names call it with their board:
#   tools/cts/pi4-deqp-images.sh   rpi_4b     (Raspberry Pi 4, V3D)
#   tools/cts/t6-deqp-images.sh    nanopc_t6  (NanoPC-T6, Mali-G610)
# The AM62P has its own builder (am62p-deqp-images.sh): its filter files may
# have rows without a label, and it reports on stdout (build.out).
#
#   tools/cts/deqp-tree-images.sh <board> [--dry-run] <table> <img-dir> [LABEL...]
#
# <board>    rpi_4b or nanopc_t6, the Zephyr board (the arms below)
# <table>    tools/cts/pi4-full-tree.txt's form: each CONFIG_DEQP_CASE line is
#            preceded by "# image <LABEL>: ..."; a marker saying
#            "(with texture-2048.conf)" adds -DEXTRA_CONF_FILE=texture-2048.conf.
#            tools/cts/pi4-gles3-tree.txt is the Pi 4's dEQP-GLES3 tree
#            (DEQP_MODULE=gles3), tools/cts/mali-gles2-tree.txt the T6's
#            dEQP-GLES2 tree.
# <img-dir>  one build directory per label, <img-dir>/<LABEL>/ (cmake -GNinja,
#            ninja, the board above). A directory that exists is refused, never
#            reused. After each image the line
#            "<LABEL> rc=<rc> sha256=<hash or none> build=<string in the image>"
#            goes to <img-dir>/<LABEL>.built: the file `board.sh <board> tree`
#            waits for (TREE_IMAGE_DIR=<img-dir>), so a tree can start while
#            later images still build.
# LABEL...   build only these images (all when none).
# --dry-run  print "<LABEL> <filter>" per selected image, followed by
#            " [<the cmake arguments this image gets beyond the common ones>]"
#            when there are any (texture-2048.conf's, DEQP_MODULE=gles3's), and
#            build nothing; needs none of the environment below.
#
# Environment (nothing defaults to a path on one machine):
#   ZEPHYR_WS   the west workspace ($ZEPHYR_WS/zephyr is ZEPHYR_BASE)
#   DEQP_WORK   stage 1 for the board (tools/cts/cts-build-zephyr.sh WORK=,
#               BOARD=<board>, MODULE=$DEQP_MODULE): $DEQP_WORK/cts and
#               $DEQP_WORK/build
#   DEQP_MODULE gles2 (default) or gles3: the dEQP package, as for
#               am62p-deqp-images.sh. gles3 needs a stage 1 built with
#               MODULE=gles3 as DEQP_WORK (each module its own WORK) and builds
#               every image with -DDEQP_MODULE=gles3 -DCONFIG_DEQP_REPORT_ES2=n
#               -DCONFIG_YAKOGL_REPORT_ES2=n: the context reports ES 3.0
#               (deqp/Kconfig says DEQP_REPORT_ES2=y for dEQP-GLES2; a
#               command-line CONFIG_ is merged after prj.conf and wins), and
#               the post-build conformance check runs with --gles3. gles2 adds
#               no argument and checks for ES 2.0, as before.
#   ZEPHYR_SDK_INSTALL_DIR as for any Zephyr sample; RUSTUP_TOOLCHAIN as for
#               any V3D sample (rpi_4b)
#   WEST_VENV   optional: a venv whose bin/ holds west (zephyr-modules.sh)
# nanopc_t6 also needs driver/src/fw/mali_csffw_blob.h in this checkout (made
# from the board's CSF firmware by driver/tools/mkmalifw.sh, never committed),
# and builds with -DYAKOGL_WERROR=ON, as the gate's nanopc_t6 rows do.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
ZBOARD=${1:-}
# Per board: NAME prefixes every message; BOARD_ARGS go to every image's
# cmake; BUILD_DEF is the define in build.ninja that carries the build string
# the image prints (the .built line's build= field).
case "$ZBOARD" in
rpi_4b) NAME=pi4-deqp-images BOARD_ARGS=() BUILD_DEF=V3D_BUILD ;;
nanopc_t6) NAME=t6-deqp-images BOARD_ARGS=(-DYAKOGL_WERROR=ON) BUILD_DEF=T6_DEQP_BUILD ;;
*) echo "deqp-tree-images: the board must be rpi_4b or nanopc_t6, not '$ZBOARD'" >&2; exit 2 ;;
esac
shift
die() { echo "$NAME: $*" >&2; exit 2; }
DRY=0
[ "${1:-}" = --dry-run ] && { DRY=1; shift; }
[ $# -ge 2 ] || die "usage: $0 $ZBOARD [--dry-run] <table> <img-dir> [LABEL...]"
TREE=$1 OUT=$2
shift 2
[ -f "$TREE" ] || die "no table $TREE"
DEQP_MODULE=${DEQP_MODULE:-gles2}
case "$DEQP_MODULE" in
	gles2) MODARGS=() ;;
	gles3) MODARGS=(-DDEQP_MODULE=gles3 -DCONFIG_DEQP_REPORT_ES2=n -DCONFIG_YAKOGL_REPORT_ES2=n) ;;
	*) die "DEQP_MODULE must be gles2 or gles3, not '$DEQP_MODULE'" ;;
esac
# "<LABEL>\t<conf or ->\t<filter>" per image, in file order.
images() {
	awk '/^# image [A-Za-z0-9]+:/ { l = $3; sub(/:$/, "", l); c = ($0 ~ /\(with texture-2048\.conf\)/) ? "texture-2048.conf" : "-"; next }
	     /^[[:space:]]*(#|$)/ { next }
	     { if (l == "") { print "filter line without a label: " $0 > "/dev/stderr"; exit 1 }
	       print l "\t" c "\t" $0; l = "" }' "$TREE"
}
selected() { # the images, narrowed to the labels given (all when none)
	local want=" $* "
	images | while IFS=$'\t' read -r l c f; do
		# an if, not an && list: under pipefail the loop's status was the last
		# row's failed test, so a selective build exited 1 with every image
		# built (found by the Pi 4 stage 1 session, 2026-10-08)
		if [ $# -eq 0 ] || [[ $want == *" $l "* ]]; then printf '%s\t%s\t%s\n' "$l" "$c" "$f"; fi
	done
}
# row_args CONF: EXTRA = the cmake arguments of one image beyond the common ones
row_args() {
	EXTRA=("${MODARGS[@]}")
	[ "$1" != - ] && EXTRA+=("-DEXTRA_CONF_FILE=$1")
}

if [ "$DRY" = 1 ]; then
	images > /dev/null || exit 2
	selected "$@" | while IFS=$'\t' read -r l c f; do
		row_args "$c"
		if [ ${#EXTRA[@]} -gt 0 ]; then echo "$l $f [${EXTRA[*]}]"; else echo "$l $f"; fi
	done
	exit 0
fi

cmd_build() {
	[ $# -ge 1 ] || die "usage: <img-dir> [LABEL...]"
	local OUT=$1; shift
	: "${ZEPHYR_WS:?set ZEPHYR_WS}"; : "${DEQP_WORK:?set DEQP_WORK (stage 1 for $ZBOARD)}"
	[ -d "$DEQP_WORK/cts" ] && [ -f "$DEQP_WORK/build/modules/$DEQP_MODULE/libdeqp-$DEQP_MODULE-package.a" ] ||
		die "$DEQP_WORK is not a $DEQP_MODULE stage 1 (cts-build-zephyr.sh MODULE=$DEQP_MODULE BOARD=$ZBOARD)"
	# the Mali's CSF firmware is embedded in every nanopc_t6 image; without the
	# header no image can build, so refuse before the first one
	if [ "$ZBOARD" = nanopc_t6 ] && [ ! -f "$REPO/driver/src/fw/mali_csffw_blob.h" ]; then
		die "driver/src/fw/mali_csffw_blob.h missing: make it from the board's CSF firmware with driver/tools/mkmalifw.sh <mali_csffw.bin> (never commit it)"
	fi
	[ -n "${WEST_VENV:-}" ] && PATH=$WEST_VENV/bin:$PATH
	export ZEPHYR_BASE=$ZEPHYR_WS/zephyr
	unset DEQP_GL_CONFIG_NAME   # the shared deqp Kconfig default reads it: no multisample config in a sweep image
	local MODS chk=()
	MODS=$("$HERE/zephyr-modules.sh" "$ZEPHYR_WS") || die "zephyr-modules.sh failed"
	[ "$DEQP_MODULE" = gles3 ] && chk=(--gles3)
	mkdir -p "$OUT"
	echo "$NAME: build from $(git -C "$REPO" describe --always --dirty) ($(git -C "$REPO" rev-parse HEAD))"
	[ "$DEQP_MODULE" = gles2 ] || echo "$NAME: DEQP_MODULE=$DEQP_MODULE (${MODARGS[*]})"
	selected "$@" | while IFS=$'\t' read -r l c f; do
		local B=$OUT/$l
		if [ -e "$B" ]; then
			echo "$l: $B exists, not reused" >&2
			echo "$l rc=exists sha256=none build=none" > "$OUT/$l.built"
			continue
		fi
		row_args "$c"
		{ cmake -GNinja -S "$REPO/driver/samples/zephyr/deqp" -B "$B" "-DBOARD=$ZBOARD" "${BOARD_ARGS[@]}" \
			"-DZEPHYR_MODULES=$MODS" "-DDEQP_SRC=$DEQP_WORK/cts" "-DDEQP_BUILD=$DEQP_WORK/build" \
			"${EXTRA[@]}" "-DCONFIG_DEQP_CASE=\"$f\"" && ninja -C "$B"; } > "$B.log" 2>&1 < /dev/null
		local rc=$? sha=none build=none
		# the resolved Kconfig must still be a conformance image (compiler, the
		# module's REPORT_ES2 answer, no table)
		if [ "$rc" = 0 ]; then
			python3 "$HERE/check-deqp-images.py" --config "$B/zephyr/.config" "${chk[@]}" >> "$B.log" 2>&1 || rc=not-conformance
		fi
		if [ -s "$B/zephyr/zephyr.bin" ]; then
			sha=$(sha256sum "$B/zephyr/zephyr.bin" | cut -d' ' -f1)
			build=$(grep -o "$BUILD_DEF=\\\\\"[^\\\\]*" "$B/build.ninja" | head -1 | sed 's/.*"//')
		fi
		echo "$l rc=$rc sha256=$sha build=${build:-none}" | tee "$OUT/$l.built"
	done
}

cmd_build "$OUT" "$@"

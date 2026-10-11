#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Stage 2 of dEQP on Zephyr, Raspberry Pi 4: one driver/samples/zephyr/v3d_deqp
# image per "# image <LABEL>:" block of a table, from this checkout as it is
# (the Pi 4's counterpart of am62p-deqp-images.sh; the build half of the old
# Pi 4 tree runner, unchanged, Step 1 of driver/docs/one-method.md: running
# and tallying the images is tools/board.sh pi4 tree).
#
#   tools/cts/pi4-deqp-images.sh [--dry-run] <table> <img-dir> [LABEL...]
#
# <table>    tools/cts/pi4-full-tree.txt's form: each CONFIG_DEQP_CASE line is
#            preceded by "# image <LABEL>: ..."; a marker saying
#            "(with texture-2048.conf)" adds -DEXTRA_CONF_FILE=texture-2048.conf.
#            tools/cts/pi4-gles3-tree.txt is the dEQP-GLES3 tree (DEQP_MODULE=gles3).
# <img-dir>  one build directory per label, <img-dir>/<LABEL>/ (cmake -GNinja,
#            ninja, board rpi_4b). A directory that exists is refused, never
#            reused. After each image the line
#            "<LABEL> rc=<rc> sha256=<hash or none> build=<string in the image>"
#            goes to <img-dir>/<LABEL>.built: the file `board.sh pi4 tree`
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
#   DEQP_WORK   stage 1 for rpi_4b (tools/cts/cts-build-zephyr.sh WORK=,
#               BOARD=rpi_4b, MODULE=$DEQP_MODULE): $DEQP_WORK/cts and
#               $DEQP_WORK/build
#   DEQP_MODULE gles2 (default) or gles3: the dEQP package, as for
#               am62p-deqp-images.sh. gles3 needs a stage 1 built with
#               MODULE=gles3 as DEQP_WORK (each module its own WORK) and builds
#               every image with -DDEQP_MODULE=gles3 -DCONFIG_DEQP_REPORT_ES2=n
#               -DCONFIG_YAKOGL_REPORT_ES2=n: the context reports ES 3.0
#               (v3d_deqp/prj.conf says REPORT_ES2=y for dEQP-GLES2; a
#               command-line CONFIG_ is merged after prj.conf and wins), and
#               the post-build conformance check runs with --gles3. gles2 adds
#               no argument and checks for ES 2.0, as before.
#   ZEPHYR_SDK_INSTALL_DIR, RUSTUP_TOOLCHAIN as for any V3D sample
#   WEST_VENV   optional: a venv whose bin/ holds west (zephyr-modules.sh)
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
die() { echo "pi4-deqp-images: $*" >&2; exit 2; }
DRY=0
[ "${1:-}" = --dry-run ] && { DRY=1; shift; }
[ $# -ge 2 ] || die "usage: $0 [--dry-run] <table> <img-dir> [LABEL...]"
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
	: "${ZEPHYR_WS:?set ZEPHYR_WS}"; : "${DEQP_WORK:?set DEQP_WORK (stage 1 for rpi_4b)}"
	[ -d "$DEQP_WORK/cts" ] && [ -f "$DEQP_WORK/build/modules/$DEQP_MODULE/libdeqp-$DEQP_MODULE-package.a" ] ||
		die "$DEQP_WORK is not a $DEQP_MODULE stage 1 (cts-build-zephyr.sh MODULE=$DEQP_MODULE BOARD=rpi_4b)"
	[ -n "${WEST_VENV:-}" ] && PATH=$WEST_VENV/bin:$PATH
	export ZEPHYR_BASE=$ZEPHYR_WS/zephyr
	unset DEQP_GL_CONFIG_NAME   # the shared deqp Kconfig default reads it: no multisample config in a sweep image
	local MODS chk=()
	MODS=$("$HERE/zephyr-modules.sh" "$ZEPHYR_WS") || die "zephyr-modules.sh failed"
	[ "$DEQP_MODULE" = gles3 ] && chk=(--gles3)
	mkdir -p "$OUT"
	echo "pi4-deqp-images: build from $(git -C "$REPO" describe --always --dirty) ($(git -C "$REPO" rev-parse HEAD))"
	[ "$DEQP_MODULE" = gles2 ] || echo "pi4-deqp-images: DEQP_MODULE=$DEQP_MODULE (${MODARGS[*]})"
	selected "$@" | while IFS=$'\t' read -r l c f; do
		local B=$OUT/$l
		if [ -e "$B" ]; then
			echo "$l: $B exists, not reused" >&2
			echo "$l rc=exists sha256=none build=none" > "$OUT/$l.built"
			continue
		fi
		row_args "$c"
		{ cmake -GNinja -S "$REPO/driver/samples/zephyr/v3d_deqp" -B "$B" -DBOARD=rpi_4b \
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
			build=$(grep -o 'V3D_BUILD=\\"[^\\]*' "$B/build.ninja" | head -1 | sed 's/.*"//')
		fi
		echo "$l rc=$rc sha256=$sha build=${build:-none}" | tee "$OUT/$l.built"
	done
}

cmd_build "$OUT" "$@"

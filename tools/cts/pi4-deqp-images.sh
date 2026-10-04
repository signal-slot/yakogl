#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Stage 2 of dEQP-GLES2 on Zephyr, Raspberry Pi 4: one driver/samples/zephyr/v3d_deqp
# image per "# image <LABEL>:" block of a table, from this checkout as it is
# (the Pi 4's counterpart of am62p-deqp-images.sh; the build half of the old
# Pi 4 tree runner, unchanged, Step 1 of driver/docs/one-method.md: running
# and tallying the images is tools/board.sh pi4 tree).
#
#   tools/cts/pi4-deqp-images.sh <table> <img-dir> [LABEL...]
#
# <table>    tools/cts/pi4-full-tree.txt's form: each CONFIG_DEQP_CASE line is
#            preceded by "# image <LABEL>: ..."; a marker saying
#            "(with texture-2048.conf)" adds -DEXTRA_CONF_FILE=texture-2048.conf.
# <img-dir>  one build directory per label, <img-dir>/<LABEL>/ (cmake -GNinja,
#            ninja, board rpi_4b). A directory that exists is refused, never
#            reused. After each image the line
#            "<LABEL> rc=<rc> sha256=<hash or none> build=<string in the image>"
#            goes to <img-dir>/<LABEL>.built: the file `board.sh pi4 tree`
#            waits for (TREE_IMAGE_DIR=<img-dir>), so a tree can start while
#            later images still build.
#
# Environment (nothing defaults to a path on one machine):
#   ZEPHYR_WS   the west workspace ($ZEPHYR_WS/zephyr is ZEPHYR_BASE)
#   DEQP_WORK   stage 1 for rpi_4b (tools/cts/cts-build-zephyr.sh WORK=,
#               BOARD=rpi_4b): $DEQP_WORK/cts and $DEQP_WORK/build
#   ZEPHYR_SDK_INSTALL_DIR, RUSTUP_TOOLCHAIN as for any V3D sample
#   WEST_VENV   optional: a venv whose bin/ holds west (zephyr-modules.sh)
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
die() { echo "pi4-deqp-images: $*" >&2; exit 2; }
[ $# -ge 2 ] || die "usage: $0 <table> <img-dir> [LABEL...]"
TREE=$1 OUT=$2
shift 2
[ -f "$TREE" ] || die "no table $TREE"
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
		[ $# -eq 0 ] || [[ $want == *" $l "* ]] && printf '%s\t%s\t%s\n' "$l" "$c" "$f"
	done
}

cmd_build() {
	[ $# -ge 1 ] || die "usage: <img-dir> [LABEL...]"
	local OUT=$1; shift
	: "${ZEPHYR_WS:?set ZEPHYR_WS}"; : "${DEQP_WORK:?set DEQP_WORK (stage 1 for rpi_4b)}"
	[ -d "$DEQP_WORK/cts" ] && [ -f "$DEQP_WORK/build/modules/gles2/libdeqp-gles2-package.a" ] || die "$DEQP_WORK is not a gles2 stage 1"
	[ -n "${WEST_VENV:-}" ] && PATH=$WEST_VENV/bin:$PATH
	export ZEPHYR_BASE=$ZEPHYR_WS/zephyr
	local MODS
	MODS=$("$HERE/zephyr-modules.sh" "$ZEPHYR_WS") || die "zephyr-modules.sh failed"
	mkdir -p "$OUT"
	echo "pi4-deqp-images: build from $(git -C "$REPO" describe --always --dirty) ($(git -C "$REPO" rev-parse HEAD))"
	selected "$@" | while IFS=$'\t' read -r l c f; do
		local B=$OUT/$l
		if [ -e "$B" ]; then
			echo "$l: $B exists, not reused" >&2
			echo "$l rc=exists sha256=none build=none" > "$OUT/$l.built"
			continue
		fi
		local extra=()
		[ "$c" != - ] && extra=("-DEXTRA_CONF_FILE=$c")
		{ cmake -GNinja -S "$REPO/driver/samples/zephyr/v3d_deqp" -B "$B" -DBOARD=rpi_4b \
			"-DZEPHYR_MODULES=$MODS" "-DDEQP_SRC=$DEQP_WORK/cts" "-DDEQP_BUILD=$DEQP_WORK/build" \
			"${extra[@]}" "-DCONFIG_DEQP_CASE=\"$f\"" && ninja -C "$B"; } > "$B.log" 2>&1 < /dev/null
		local rc=$? sha=none build=none
		if [ -s "$B/zephyr/zephyr.bin" ]; then
			sha=$(sha256sum "$B/zephyr/zephyr.bin" | cut -d' ' -f1)
			build=$(grep -o 'V3D_BUILD=\\"[^\\]*' "$B/build.ninja" | head -1 | sed 's/.*"//')
		fi
		echo "$l rc=$rc sha256=$sha build=${build:-none}" | tee "$OUT/$l.built"
	done
}

cmd_build "$OUT" "$@"

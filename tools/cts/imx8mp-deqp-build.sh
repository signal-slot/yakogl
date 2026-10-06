#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-deqp-build.sh - dEQP stage 2 for the Verdin iMX8M Plus: one Zephyr image of the deqp sample
# (driver/samples/zephyr/deqp), from this checkout, linked against the stage-1 build.
#
#   tools/cts/imx8mp-deqp-build.sh <name> <default case filter> [<Kconfig fragment>]
#   tools/cts/imx8mp-deqp-build.sh <name> <default case filter> --table <table file>
#   tools/cts/imx8mp-deqp-build.sh fbo-1234abcd 'dEQP-GLES2.info.*' table.conf
#   tools/cts/imx8mp-deqp-build.sh wt-1234abcd 'dEQP-GLES2.info.*' --table tools/cts/imx8mp-full-tree.txt
#
# <Kconfig fragment> usually holds CONFIG_DEQP_CASE_TABLE="filterA;filterB,filterC;..." (the run-time
# selector picks entry i; a ';' inside a -D argument is eaten by CMake, hence a file) and any switches
# (CONFIG_DEQP_PROBE_POINTS, CONFIG_DEQP_SNVS_ADDRESS, CONFIG_YAKOGL_GC_CARVEOUT_FILL, ...; the
# GC7000's masked clear is the library's default now, its image switch is gone). A
# conformance image takes no switch: REPORT_ES2 on, the compiler, no precompiled table, as the
# sample's prj.conf and boards/*.conf leave them (tools/cts/check-deqp-images.py checks that).
# --table: one dEQP filter per line ('#' lines and blank lines skipped), the form of
# tools/cts/imx8mp-full-tree.txt: the lines joined with ';' become CONFIG_DEQP_CASE_TABLE in
# $IMX8MP_OUT/<name>.table.conf, the fragment then used. That image runs every row by its
# selector: `TREE_IMAGE=<image> tools/board.sh imx8mp tree <table> <outdir>` (the step the old
# i.MX8MP tree runner did before its jumps, moved here in Step 1 of driver/docs/one-method.md).
# Name the image after the commit it is built from: a result is reported against that hash.
#
# Needs: the variables of imx8mp-env.sh (Zephyr part) and DEQP_STAGE1 (from imx8mp-deqp-stage1.sh).
# Output: $IMX8MP_OUT/<name>/zephyr/zephyr.bin, log $IMX8MP_OUT/<name>.log.
set -u
# absolute, resolved before the cd to the Zephyr workspace below: the script
# is documented as `tools/cts/imx8mp-deqp-build.sh ...` (a relative path that
# stops resolving after that cd), and it runs tools/cts/check-deqp-images.py
# after the build from here.
HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/imx8mp-env.sh"
[ $# -ge 2 ] || { echo "usage: $0 <name> <case filter> [<Kconfig fragment> | --table <table file>]" >&2; exit 2; }
imx8mp_zephyr_env
imx8mp_need DEQP_STAGE1
name=$1; filter=$2; frag=${3:-}
if [ "$frag" = --table ]; then
	table=${4:?--table needs a table file}
	[ -f "$table" ] || { echo "imx8mp: $table not found" >&2; exit 2; }
	frag=$IMX8MP_OUT/$name.table.conf
	printf 'CONFIG_DEQP_CASE_TABLE="%s"\n' "$(grep -vE '^[[:space:]]*(#|$)' "$table" | paste -sd';' -)" > "$frag"
	echo "imx8mp: $(grep -cvE '^[[:space:]]*(#|$)' "$table") table rows -> $frag"
fi
extra=()
if [ -n "$frag" ]; then
	frag=$(cd "$(dirname "$frag")" && pwd)/$(basename "$frag")
	[ -f "$frag" ] || { echo "imx8mp: $frag not found" >&2; exit 2; }
	extra=(-DEXTRA_CONF_FILE="$frag")
fi
unset DEQP_GL_CONFIG_NAME   # the Kconfig default reads it; a fragment says it explicitly
cd "$ZEPHYR_WORKSPACE" || exit 1
west build -p always -b "$BOARD" -d "$IMX8MP_OUT/$name" "$YAKOGL_DIR/driver/samples/zephyr/deqp" -- \
	-DZEPHYR_MODULES="$ZEPHYR_MODULES" \
	-DDEQP_SRC="$DEQP_STAGE1/cts" -DDEQP_BUILD="$DEQP_STAGE1/build" \
	-DCONFIG_DEQP_CASE="\"$filter\"" "${extra[@]}" > "$IMX8MP_OUT/$name.log" 2>&1
rc=$?
echo "rc=$rc"
grep -E " error|Error" "$IMX8MP_OUT/$name.log" | head -10
grep -E "DEQP_REPORT_ES2=y" "$IMX8MP_OUT/$name/zephyr/.config" 2>/dev/null || echo "imx8mp: warning: the image does not report ES 2.0"
# conformance: the compiler, REPORT_ES2, no precompiled tier (a fragment that asks for another is a measurement image)
python3 "$HERE/check-deqp-images.py" --config "$IMX8MP_OUT/$name/zephyr/.config" 2>&1 || echo "imx8mp: warning: not a conformance image"
exit $rc

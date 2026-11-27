#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gles3-artefacts.sh - print bytes, sha256 and path of the dEQP-GLES3 stage 1 libraries and of an image
# built by imx8mp-deqp-build.sh --gles3, for the session note.
#
#   tools/cts/imx8mp-gles3-artefacts.sh es3-<hash8>
#   tools/cts/imx8mp-gles3-artefacts.sh x2/es3-<hash8>          (or IMX8MP_BOARD=x2 ... es3-<hash8>)
#
# Needs IMX8MP_OUT (imx8mp-env.sh); DEQP_STAGE1_GLES3 defaults to $IMX8MP_OUT/deqp-stage1-gles3.
# The Armadillo-X2 (same i.MX 8M Plus and GC7000UL): its images are $IMX8MP_OUT/x2/es3-<hash8>
# (IMX8MP_BOARD=x2 imx8mp-deqp-build.sh). Name it x2/es3-<hash8>, or es3-<hash8> with IMX8MP_BOARD=x2; the
# output then starts with a line naming the board and also prints the image's CONFIG_BOARD and
# CONFIG_GC_BOARD_WDOG_SELF_ARM (tools/board.sh x2 needs the watchdog self-arm). Without either, the output is
# the Verdin's, as before.
set -eu
[ $# -eq 1 ] || { echo "usage: $0 es3-<hash8> | x2/es3-<hash8>" >&2; exit 2; }
: "${IMX8MP_OUT:?IMX8MP_OUT is not set (tools/cts/imx8mp-env.sh)}"
s1=${DEQP_STAGE1_GLES3:-$IMX8MP_OUT/deqp-stage1-gles3}
img=$1
case "${IMX8MP_BOARD:-}" in
'') ;;
x2) case $img in x2/*) ;; *) img=x2/$img ;; esac ;;
*) echo "$0: IMX8MP_BOARD must be empty or x2, not '$IMX8MP_BOARD'" >&2; exit 2 ;;
esac
x2=0
case $img in x2/*) x2=1 ;; esac
[ $x2 = 0 ] || echo "board x2: the Armadillo-X2 (i.MX 8M Plus, GC7000UL), image under \$IMX8MP_OUT/x2"
for f in "$IMX8MP_OUT/$img/zephyr/zephyr.bin" "$IMX8MP_OUT/$img/zephyr/zephyr.elf" "$IMX8MP_OUT/$img.table.conf" \
	"$s1/build/modules/gles3/libdeqp-gles3-package.a" \
	"$s1/build/modules/gles3/functional/libdeqp-gles3-functional.a"; do
	[ -f "$f" ] || { echo "$0: missing artefact: $f" >&2; exit 1; }
	printf '%s %s %s\n' "$(stat -c %s "$f")" "$(sha256sum "$f" | cut -d' ' -f1)" "$f"
done
keys=(CONFIG_DEQP_REPORT_ES2 CONFIG_YAKOGL_REPORT_ES2)
[ $x2 = 0 ] || keys+=(CONFIG_BOARD CONFIG_GC_BOARD_WDOG_SELF_ARM)
for k in "${keys[@]}"; do
	grep -E "^(# )?$k[= ]" "$IMX8MP_OUT/$img/zephyr/.config" || echo "$k: not in .config"
done
# the linker right-aligns the region names: its line is "             RAM:     233912 KB ..."
grep -E '^[[:space:]]*(Memory region|RAM:|FLASH:)' "$IMX8MP_OUT/$img.log" 2>/dev/null || true

#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gles3-artefacts.sh - print bytes, sha256 and path of the dEQP-GLES3 stage 1 libraries and of an image
# built by imx8mp-deqp-build.sh --gles3, for the session note.
#
#   tools/cts/imx8mp-gles3-artefacts.sh es3-<hash8>
#
# Needs IMX8MP_OUT (imx8mp-env.sh); DEQP_STAGE1_GLES3 defaults to $IMX8MP_OUT/deqp-stage1-gles3.
set -eu
[ $# -eq 1 ] || { echo "usage: $0 es3-<hash8>" >&2; exit 2; }
: "${IMX8MP_OUT:?IMX8MP_OUT is not set (tools/cts/imx8mp-env.sh)}"
s1=${DEQP_STAGE1_GLES3:-$IMX8MP_OUT/deqp-stage1-gles3}
img=$1
for f in "$IMX8MP_OUT/$img/zephyr/zephyr.bin" "$IMX8MP_OUT/$img/zephyr/zephyr.elf" "$IMX8MP_OUT/$img.table.conf" \
	"$s1/build/modules/gles3/libdeqp-gles3-package.a" \
	"$s1/build/modules/gles3/functional/libdeqp-gles3-functional.a"; do
	printf '%s %s %s\n' "$(stat -c %s "$f")" "$(sha256sum "$f" | cut -d' ' -f1)" "$f"
done
for k in CONFIG_DEQP_REPORT_ES2 CONFIG_YAKOGL_REPORT_ES2; do
	grep -E "^(# )?$k[= ]" "$IMX8MP_OUT/$img/zephyr/.config" || echo "$k: not in .config"
done
grep -E '^(Memory region|RAM:|FLASH:)' "$IMX8MP_OUT/$img.log" 2>/dev/null || true

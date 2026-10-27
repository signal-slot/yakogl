#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gles3-board.sh - tools/board.sh imx8mp with the dEQP-GLES3 table image and table filled in.
#
#   tools/cts/imx8mp-gles3-board.sh [--dry-run] tree <outdir>     the whole 133-row table
#   tools/cts/imx8mp-gles3-board.sh tally <outdir>
#
# TREE_IMAGE is the image built by `imx8mp-deqp-build.sh --gles3 es3-<hash8> 'dEQP-GLES3.info.*' --table
# tools/cts/imx8mp-gles3-tree.txt`; when unset, the newest $IMX8MP_OUT/es3-*/zephyr/zephyr.bin. Everything
# else (BOARD_HOST, BOARD_BOOT, TREE_ROWS, NAMES) is board.sh's own environment.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
dry=()
[ "${1:-}" = --dry-run ] && { dry=(--dry-run); shift; }
verb=${1:?usage: $0 [--dry-run] tree|tally <outdir>}
out=${2:?usage: $0 [--dry-run] tree|tally <outdir>}
case "$verb" in
	tree)
		if [ -z "${TREE_IMAGE:-}" ]; then
			: "${IMX8MP_OUT:?IMX8MP_OUT or TREE_IMAGE is not set}"
			TREE_IMAGE=$(ls -t "$IMX8MP_OUT"/es3-*/zephyr/zephyr.bin 2>/dev/null | head -1)
			[ -n "$TREE_IMAGE" ] || { echo "imx8mp-gles3-board: no es3-* image in $IMX8MP_OUT" >&2; exit 2; }
		fi
		export TREE_IMAGE
		exec "$ROOT/tools/board.sh" "${dry[@]}" imx8mp tree "$HERE/imx8mp-gles3-tree.txt" "$out" ;;
	tally) exec "$ROOT/tools/board.sh" imx8mp tally "$out" ;;
	*) echo "usage: $0 [--dry-run] tree|tally <outdir>" >&2; exit 2 ;;
esac

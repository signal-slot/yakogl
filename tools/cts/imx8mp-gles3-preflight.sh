#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gles3-preflight.sh - read-only host check before an i.MX8MP dEQP-GLES3 board session: the env, the
# stage 1 package, the table, the table image and its commit. One 'ok'/'FAIL' line per check; exit 0 only if all pass.
#   tools/cts/imx8mp-gles3-preflight.sh [--commit <sha, 8+ hex>] [--out <dir>] [--table <file>]
# Defaults: commit = git rev-parse origin/main, out = $IMX8MP_OUT, table = tools/cts/imx8mp-gles3-tree.txt.
# Writes nothing, uses no network or board; BOARD_UART is only reported set/unset. imx8mp-env.sh is not sourced
# (it exits and creates $IMX8MP_OUT); the image layout is imx8mp-deqp-build.sh's: <out>/es3-<hash8>/zephyr/zephyr.bin.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
commit='' out=${IMX8MP_OUT:-} table=$HERE/imx8mp-gles3-tree.txt
while [ $# -gt 0 ]; do
	case $1 in
		--commit) commit=${2:?--commit needs a sha}; shift 2 ;;
		--out) out=${2:?--out needs a directory}; shift 2 ;;
		--table) table=${2:?--table needs a file}; shift 2 ;;
		*) echo "usage: $0 [--commit <sha>] [--out <dir>] [--table <file>]" >&2; exit 2 ;;
	esac
done
fails=0
ok() { echo "  ok   $1"; }
bad() { echo "  FAIL $1"; fails=$((fails + 1)); }

IMX8MP_OUT=$out
for v in IMX8MP_OUT DEQP_STAGE1 ZEPHYR_WS HAL_NXP_DIR; do
	eval "val=\${$v:-}"
	if [ -z "$val" ]; then bad "$v is not set"
	elif [ -d "$val" ]; then ok "$v is set, directory exists"
	else bad "$v is set, directory does not exist"; fi
done
if [ -n "${BOARD_UART:-}" ]; then ok "BOARD_UART is set"; else bad "BOARD_UART is unset"; fi

pkg=${DEQP_STAGE1:-}/build/modules/gles3/libdeqp-gles3-package.a
if [ -n "${DEQP_STAGE1:-}" ] && [ -f "$pkg" ]; then ok "stage 1 package exists (build/modules/gles3/libdeqp-gles3-package.a)"
else bad "stage 1 package missing (<DEQP_STAGE1>/build/modules/gles3/libdeqp-gles3-package.a)"; fi

if [ ! -f "$table" ]; then bad "table file not found"
elif python3 "$HERE/check-imx8mp-gles3-tree.py" -q --table "$table" > /dev/null 2>&1; then ok "table passes check-imx8mp-gles3-tree.py"
else bad "table fails check-imx8mp-gles3-tree.py"; fi

if [ -z "$commit" ]; then commit=$(git -C "$ROOT" rev-parse origin/main 2> /dev/null || true); fi
case $commit in
	'') bad "no commit: origin/main does not resolve (give --commit)" ;;
	*[!0-9a-fA-F]*) bad "commit is not hex"; commit='' ;;
	*) if [ ${#commit} -lt 8 ]; then bad "commit is shorter than 8 hex digits"; commit=''; fi ;;
esac
commit=$(printf %s "$commit" | tr 'A-F' 'a-f')

images=()
if [ -n "$out" ]; then
	for d in "$out"/es3-*; do
		if [ -f "$d/zephyr/zephyr.bin" ]; then images+=("$(basename "$d")"); fi
	done
fi
if [ ${#images[@]} -gt 0 ]; then ok "table image exists (${images[*]})"
else bad "no table image <out>/es3-<hash8>/zephyr/zephyr.bin"; fi
if [ -n "$commit" ]; then
	match=''
	for n in "${images[@]}"; do
		h=${n#es3-}
		case $commit in "$h"*) match=$n ;; esac
		case $h in "$commit"*) match=$n ;; esac
	done
	if [ -n "$match" ]; then ok "image $match is commit ${commit:0:8}"
	else bad "no image is named after commit ${commit:0:8}"; fi
fi

if [ $fails -eq 0 ]; then echo "preflight: all ok"; exit 0; fi
echo "preflight: $fails check(s) FAILED"
exit 1

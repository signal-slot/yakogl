#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# t6-uboot-rows.sh - run NanoPC-T6 dEQP-GLES2 row images from U-Boot, one
# after another, resumable.
#
#   tools/cts/t6-uboot-rows.sh <image-dir> <out-dir> <LABEL>... [--go]
#
# <image-dir> is t6-deqp-images.sh's output directory: <LABEL>/zephyr/zephyr.bin
# (also taken: <LABEL>/zephyr.bin). Every LABEL must be a "# image <LABEL>:"
# line of tools/cts/mali-gles2-tree.txt (read only). Each label runs
# driver/tools/uboot-t6/prompt-go.sh <zephyr.bin> deqp [--go] (PROMPT_GO
# overrides the path). Without --go prompt-go makes its own dry run. The board
# address, T6_UART, T6_HAND and T6_RECORD_DIR are read by prompt-go from the
# environment; this script neither stores nor prints them.
#
# <out-dir>/rows.tsv gets one line per row: label, exit code, meaning, record
# directory (the one prompt-go printed, or "-"). 0 (PASS) and 11 (FAIL) are
# answers with Armbian back: the next label runs. Any other code stops at
# once (never retried): the board may need a person. A rerun with the same
# out-dir skips the labels already in rows.tsv with 0 or 11.
#
# Exit: 0 every label done, 2 usage or a refused label/image (nothing run),
# 3 stopped at a code other than 0/11.
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
TREE=${TREE:-$HERE/mali-gles2-tree.txt}
PROMPT_GO=${PROMPT_GO:-$REPO/driver/tools/uboot-t6/prompt-go.sh}

die() { echo "t6-uboot-rows.sh: $*" >&2; exit 2; }

meaning() {
	case $1 in
	0) echo "PASS, Armbian back" ;;
	1) echo "board did not come back into Armbian" ;;
	2) echo "prompt-go usage or missing tool" ;;
	3) echo "refused before any change" ;;
	4) echo "autoboot missed, Linux booted" ;;
	5) echo "stopped at the U-Boot prompt, needs power cut" ;;
	6) echo "refused at the prompt, reset entered, no sign of Linux" ;;
	7) echo "a command made U-Boot reset the board" ;;
	8) echo "an answer differed, no go, Armbian back" ;;
	9) echo "go not answered" ;;
	10) echo "image started, no sign of Linux: needs T6_HAND" ;;
	11) echo "FAIL, Armbian back" ;;
	12) echo "no autoboot line after the reboot" ;;
	40) echo "UART bridge: could not open the session" ;;
	41) echo "UART bridge failed during the run" ;;
	*) echo "unknown exit code" ;;
	esac
}

GO=()
ARGS=()
for a in "$@"; do
	case $a in
	--go) GO=(--go) ;;
	-*) die "unknown option $a" ;;
	*) ARGS+=("$a") ;;
	esac
done
[ "${#ARGS[@]}" -ge 3 ] || die "usage: t6-uboot-rows.sh <image-dir> <out-dir> <LABEL>... [--go]"
IMGDIR=${ARGS[0]}
OUT=${ARGS[1]}
LABELS=("${ARGS[@]:2}")

[ -f "$TREE" ] || die "no table $TREE"
[ -f "$PROMPT_GO" ] || die "no prompt-go at $PROMPT_GO"
[ -d "$IMGDIR" ] || die "no image directory $IMGDIR"

# Check everything before anything runs.
BINS=()
for l in "${LABELS[@]}"; do
	grep -qE "^# image $l:" "$TREE" || die "label $l is not an '# image' line of $(basename "$TREE")"
	bin=
	for c in "$IMGDIR/$l/zephyr/zephyr.bin" "$IMGDIR/$l/zephyr.bin"; do
		if [ -s "$c" ]; then bin=$c; break; fi
	done
	[ -n "$bin" ] || die "no image for $l: $IMGDIR/$l/zephyr/zephyr.bin is missing"
	BINS+=("$bin")
done

mkdir -p "$OUT" || die "cannot make $OUT"
TSV=$OUT/rows.tsv
touch "$TSV"

i=0
for l in "${LABELS[@]}"; do
	bin=${BINS[$i]}
	i=$((i + 1))
	if awk -F'\t' -v l="$l" '$1 == l && ($2 == 0 || $2 == 11) { f = 1 } END { exit !f }' "$TSV"; then
		echo "t6-uboot-rows.sh: $l already done, skipped"
		continue
	fi
	echo "t6-uboot-rows.sh: $l ($i of ${#LABELS[@]})"
	log=$OUT/$l.prompt-go.log
	"$PROMPT_GO" "$bin" deqp "${GO[@]}" 2>&1 | tee "$log"
	rc=${PIPESTATUS[0]}
	rec=$(sed -n 's/^prompt-go\.sh: record //p' "$log" | tail -1)
	printf '%s\t%s\t%s\t%s\n' "$l" "$rc" "$(meaning "$rc")" "${rec:-"-"}" >> "$TSV"
	case $rc in
	0 | 11) ;;
	*)
		echo "t6-uboot-rows.sh: $l ended with $rc ($(meaning "$rc")): stopped, the later labels are not run. The board may need a person; nothing is retried." >&2
		exit 3
		;;
	esac
done
echo "t6-uboot-rows.sh: done, rows in $TSV"
exit 0

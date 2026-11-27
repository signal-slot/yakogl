#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gles3-board.sh - tools/board.sh imx8mp (or x2) with the dEQP-GLES3 table image and table filled in.
#
#   tools/cts/imx8mp-gles3-board.sh [--dry-run] [--board imx8mp|x2] tree <outdir>     the whole 133-row table
#   tools/cts/imx8mp-gles3-board.sh [--board imx8mp|x2] tally <outdir>
#
# The board: --board, else IMX8MP_BOARD, else imx8mp (the Toradex Verdin iMX8M Plus, tools/board.sh imx8mp).
# x2 is the Armadillo-X2 (same i.MX 8M Plus and GC7000UL): tools/board.sh x2, the crash-route kexec, staged in
# the board's /tmp tmpfs. By the owner's decision of 2026-11-27 its runs are rows of this campaign although
# board.sh calls them DEV (docs/es3/board-recipe-imx8mp.md, "Running a GLES3 slice on the Armadillo-X2").
#
# TREE_IMAGE is the image built by `imx8mp-deqp-build.sh --gles3 es3-<hash8> 'dEQP-GLES3.info.*' --table
# tools/cts/imx8mp-gles3-tree.txt` (x2: `IMX8MP_BOARD=x2 DEQP_MODULE=gles3 imx8mp-deqp-build.sh es3-<hash8> ...`);
# when unset, the newest $IMX8MP_OUT/es3-*/zephyr/zephyr.bin (x2: $IMX8MP_OUT/x2/es3-*/zephyr/zephyr.bin; the
# two never stand in for each other). Everything else (BOARD_HOST, BOARD_BOOT, BOARD_UART, TREE_ROWS, NAMES) is
# board.sh's own environment. The board.sh command line is printed on stderr before it is run.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
BOARD_SH=${IMX8MP_GLES3_BOARD_SH:-$ROOT/tools/board.sh}	# the selftest points it at a stub
USAGE="usage: $0 [--dry-run] [--board imx8mp|x2] tree|tally <outdir>"
dry=()
board=${IMX8MP_BOARD:-imx8mp}
while [ $# -gt 0 ]; do
	case $1 in
		--dry-run) dry=(--dry-run); shift ;;
		--board) board=${2:-}; [ $# -ge 2 ] || { echo "$USAGE" >&2; exit 2; }; shift 2 ;;
		--board=*) board=${1#--board=}; shift ;;
		*) break ;;
	esac
done
case $board in
	imx8mp) imgs=es3-* ;;
	x2) imgs=x2/es3-* ;;
	*) echo "imx8mp-gles3-board: the board is imx8mp or x2, not '$board'" >&2; exit 2 ;;
esac
verb=${1:?$USAGE}
out=${2:?$USAGE}
case "$verb" in
	tree)
		if [ -z "${TREE_IMAGE:-}" ]; then
			: "${IMX8MP_OUT:?IMX8MP_OUT or TREE_IMAGE is not set}"
			# shellcheck disable=SC2086  # the glob is the point
			TREE_IMAGE=$(ls -t "$IMX8MP_OUT"/$imgs/zephyr/zephyr.bin 2>/dev/null | head -1)
			[ -n "$TREE_IMAGE" ] || { echo "imx8mp-gles3-board: no $imgs image in $IMX8MP_OUT" >&2; exit 2; }
		fi
		export TREE_IMAGE
		echo "imx8mp-gles3-board: tools/board.sh ${dry[*]:+${dry[*]} }$board tree tools/cts/imx8mp-gles3-tree.txt $out" >&2
		if [ ${#dry[@]} -gt 0 ]; then exec "$BOARD_SH" "${dry[@]}" "$board" tree "$HERE/imx8mp-gles3-tree.txt" "$out"; fi
		# A real run goes entry by entry (board.sh's own TREE_ROWS), so the chain can stop at the first
		# entry that failed to run: board.sh exits non-zero, or its verdict line says not_back, refused or a
		# GPU reset (done_with_resets). Other verdicts (failed, done_stopped, ...) are results, the chain goes on.
		# It prints "imx8mp-gles3-board: STOP e<NN> rc=<rc> verdict=<v>" and exits non-zero (the rc, else 1).
		# INT/TERM stop the running board.sh and exit 130/143: `kill <pid>` ends a session, no pkill -f.
		rows=${TREE_ROWS:-}
		if [ -z "$rows" ]; then
			n=$(grep -cvE '^[[:space:]]*(#|$)' "$HERE/imx8mp-gles3-tree.txt")
			rows=0-$((n - 1))
		fi
		list=()
		IFS=, read -ra parts <<<"$rows"
		for part in "${parts[@]}"; do
			a=${part%-*}; b=${part#*-}
			for ((k = a; k <= b; k++)); do list+=("$k"); done
		done
		child=0 cur=0 log=$(mktemp "${TMPDIR:-/tmp}/gles3-board.XXXXXX")
		on_sig() { # on_sig CODE
			trap '' INT TERM
			if [ "$child" -gt 0 ]; then kill -TERM "$child" 2>/dev/null; wait "$child" 2>/dev/null; fi
			echo "imx8mp-gles3-board: STOPPED by signal at e$(printf %02d "$cur"); the chain ends here" >&2
			rm -f "$log"; exit "$1"
		}
		trap 'on_sig 130' INT
		trap 'on_sig 143' TERM
		for cur in "${list[@]}"; do
			: > "$log"
			exec {fd}> >(tee "$log"); teepid=$!
			TREE_ROWS=$cur "$BOARD_SH" "$board" tree "$HERE/imx8mp-gles3-tree.txt" "$out" < /dev/null >&"$fd" 2>&1 &
			child=$!
			wait "$child"; rc=$?
			child=0
			exec {fd}>&-; wait "$teepid" 2>/dev/null	# let the tee behind the log drain
			vline=$(grep -aE '^board\.sh [^ ]+ [^ ]+ rc=[0-9]+ verdict=' "$log" | tail -1)
			verdict=$(sed -n 's/.* verdict=\([^ ]*\).*/\1/p' <<<"$vline")
			vrc=$(sed -n 's/.* rc=\([0-9]*\) verdict=.*/\1/p' <<<"$vline")
			bad=0
			case $verdict in not_back | refused | *reset*) bad=1 ;; esac
			if [ "$rc" -ne 0 ] || [ "$bad" = 1 ]; then
				echo "imx8mp-gles3-board: STOP e$(printf %02d "$cur") rc=${vrc:-$rc} verdict=${verdict:-none}; the chain stops" >&2
				rm -f "$log"
				code=$rc; [ "$code" -ne 0 ] || code=${vrc:-1}; [ "$code" -ne 0 ] || code=1
				exit "$code"
			fi
		done
		rm -f "$log" ;;
	tally)
		echo "imx8mp-gles3-board: tools/board.sh $board tally $out" >&2
		exec "$BOARD_SH" "$board" tally "$out" ;;
	*) echo "$USAGE" >&2; exit 2 ;;
esac

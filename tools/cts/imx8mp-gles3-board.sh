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
#
# Two X2 knobs, read by `tree` only, for a host that reaches the X2 by ssh alone (docs/es3/board-recipe-imx8mp.md,
# "Running on the Armadillo-X2: the 9bc82915 slices (plan #410)"):
#   X2_PING=auto|icmp|ssh  board.sh x2 waits for the board after each jump with `ping`, then ssh. ssh puts
#                          tools/cts/x2-ping/ (a ping that answers when the board's sshd does) first in PATH; auto
#                          (the default) does that when `ping -c1 -W2 <host>` gets no answer; icmp leaves PATH alone.
#   X2_KEEP_KEY=1          the board's /root is a tmpfs: a key added to /root/.ssh/authorized_keys for this session
#                          (a lent host's lease key) is gone after the first jump's reboot, and every later row ends
#                          rc 5. With 1, the live authorized_keys is persisted (persist_file) before the tree when it
#                          holds lines the persisted copy lacks, and the persisted copy is put back byte for byte after
#                          board.sh ends, whatever its rc (sha256 checked; the live file keeps the session's key). A
#                          restore that cannot run prints the undo and exits 7 when board.sh's rc was 0.
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

# ---- X2 only: X2_PING and X2_KEEP_KEY (see the header)
AK=/root/.ssh/authorized_keys
AKP=/live/rootfs/root/.ssh/authorized_keys   # the persisted copy: Armadillo Base OS's rootfs under the overlay
x2_ssh() { timeout 60 ssh -o BatchMode=yes -o ConnectTimeout=10 "$BOARD_HOST" "$@"; }
x2_ping() {
	local mode=${X2_PING:-auto} host
	case $mode in auto | icmp | ssh) ;; *) echo "imx8mp-gles3-board: X2_PING is auto, icmp or ssh, not '$mode'" >&2; exit 2 ;; esac
	if [ "$mode" = auto ]; then
		host=${BOARD_HOST:-}; host=${host#*@}
		if [ -z "$host" ] || [ ${#dry[@]} -gt 0 ] || ping -c1 -W2 "$host" > /dev/null 2>&1; then mode=icmp; else mode=ssh; fi
	fi
	if [ "$mode" = ssh ]; then
		PATH=$HERE/x2-ping:$PATH; export PATH
		echo "imx8mp-gles3-board: X2_PING=ssh: board.sh's ping is tools/cts/x2-ping/ping (the board answers when its sshd does)" >&2
	fi
}
x2_keep_key() {
	local n
	: "${BOARD_HOST:?X2_KEEP_KEY=1 needs BOARD_HOST}"
	KD=$(mktemp -d "${TMPDIR:-$PWD}/x2-keep-key.XXXXXX") || exit 2
	if [ ${#dry[@]} -gt 0 ]; then
		echo "DRY board\$ cat $AKP   # the persisted copy, kept on the host for the restore" >&2
		echo "DRY board\$ grep -cvxF -f $AKP $AK; persist_file $AK   # only when the live file holds lines the copy lacks" >&2
		return 0
	fi
	x2_ssh "[ -f $AKP ] && command -v persist_file > /dev/null && cat $AKP" < /dev/null > "$KD/persisted" ||
		{ echo "imx8mp-gles3-board: X2_KEEP_KEY: cannot read $AKP or no persist_file on the board; nothing changed" >&2; exit 2; }
	n=$(x2_ssh "grep -cvxF -f $AKP $AK" < /dev/null) || n=0
	if [ "${n:-0}" = 0 ]; then
		echo "imx8mp-gles3-board: X2_KEEP_KEY: every live key is persisted already; nothing to keep" >&2
		: > "$KD/none"
		return 0
	fi
	x2_ssh "persist_file $AK && cmp -s $AK $AKP" < /dev/null ||
		{ echo "imx8mp-gles3-board: X2_KEEP_KEY: persist_file failed; restoring" >&2; x2_restore_key; exit 2; }
	echo "imx8mp-gles3-board: X2_KEEP_KEY: $n live key line(s) persisted for the session; the copy of $(wc -c < "$KD/persisted") bytes goes back after board.sh" >&2
}
x2_restore_key() {
	local want got
	[ -n "${KD:-}" ] || return 0
	if [ ${#dry[@]} -gt 0 ] || [ -e "$KD/none" ]; then
		[ ${#dry[@]} -eq 0 ] || echo "DRY board\$ the kept copy -> $AK; persist_file $AK; the live file back; sha256sum $AKP" >&2
		rm -rf "$KD"; return 0
	fi
	want=$(sha256sum < "$KD/persisted" | cut -d' ' -f1)
	got=$(x2_ssh "t=/tmp/.yk-ak.\$\$; cat > \$t.orig && cp $AK \$t.live && cp \$t.orig $AK && persist_file $AK; r=\$?; cp \$t.live $AK; rm -f \$t.orig \$t.live; [ \$r = 0 ] && sha256sum < $AKP" < "$KD/persisted" | cut -d' ' -f1)
	if [ -n "$got" ] && [ "$got" = "$want" ]; then
		echo "imx8mp-gles3-board: X2_KEEP_KEY: the persisted $AK is back as it was (sha256 ${want:0:16})" >&2
		rm -rf "$KD"; return 0
	fi
	echo "imx8mp-gles3-board: X2_KEEP_KEY: NOT restored (sha256 '${got:-no answer}', want ${want:0:16}): put $KD/persisted back as $AK on the board and run persist_file $AK, then put the live file back" >&2
	return 1
}
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
		keep=0
		if [ "$board" = x2 ]; then
			x2_ping
			if [ "${X2_KEEP_KEY:-0}" = 1 ]; then keep=1; x2_keep_key; fi
		fi
		# end CODE: put the X2's persisted authorized_keys back (X2_KEEP_KEY=1), then exit CODE (7 for a failed
		# restore after a run that was otherwise clean)
		end() {
			local code=$1
			if [ "$keep" = 1 ]; then x2_restore_key || { [ "$code" != 0 ] || code=7; }; fi
			exit "$code"
		}
		echo "imx8mp-gles3-board: tools/board.sh ${dry[*]:+${dry[*]} }$board tree tools/cts/imx8mp-gles3-tree.txt $out" >&2
		if [ ${#dry[@]} -gt 0 ]; then
			[ "$keep" = 1 ] || exec "$BOARD_SH" "${dry[@]}" "$board" tree "$HERE/imx8mp-gles3-tree.txt" "$out"
			"$BOARD_SH" "${dry[@]}" "$board" tree "$HERE/imx8mp-gles3-tree.txt" "$out"; end $?
		fi
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
			rm -f "$log"; end "$1"
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
				end "$code"
			fi
		done
		rm -f "$log"
		end 0 ;;
	tally)
		echo "imx8mp-gles3-board: tools/board.sh $board tally $out" >&2
		exec "$BOARD_SH" "$board" tally "$out" ;;
	*) echo "$USAGE" >&2; exit 2 ;;
esac

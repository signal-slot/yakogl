#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Self-test of tools/cts/t6-deqp-images.sh --run (the NanoPC-T6 whole-tree
# runner) without a board: a fake console and a fake tools/board.sh.
#
#   tools/cts/t6-deqp-run-selftest.sh
#
# The fake console is a script that prints canned board output (the image's
# lines of a row) for ever; it reaches the fake board.sh as a pty through
# tools/cts/t6-console-pty.py (T6_CONSOLE_CMD), and the fake board.sh reads
# each row's uart.log from that pty the way nanopc-t6.sh reads T6_UART (cat,
# read only, a character device nobody else holds). The fake board.sh is
# `board.sh nanopc-t6 tree` cut down: tree.txt, one run directory per row
# (meta.txt, INDEX.txt), "=== <label> rc=N" lines, the skip of a finished
# row, rc 5 and a stop for the row named in FAKE_HANG (the board not back),
# rc 3 for the row named in FAKE_BUSY (the runner's "busy" refusal), and the
# runner's T6_HAND / T6_UNATTENDED rule. Also t6-console-pty.py --self-test.
# Checks: the console reaches the row records, T6_HAND or --unattended is
# required, the unattended hang stops the tree with INCOMPLETE.txt (the row,
# the reason, the rows left, the resume command), the resume runs only the
# rows left and the hung row stays incomplete, a busy console is named, the
# lease's console variables become an rfc2217:// BOARD_UART whose value is
# never printed, and --help names the console and unattended options.
# cts-selftest.sh runs it (check 20c). No network, no board, about 15 s.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
S=$HERE/t6-deqp-images.sh
# outside any checkout: the record directory's rule
W=$(mktemp -d "${TMPDIR:-/tmp}/t6run.XXXXXX") || exit 1
trap 'rm -rf "$W"' EXIT
fail=0 n=0
ok() { n=$((n + 1)); echo "ok   $1"; }
bad() { n=$((n + 1)); echo "FAIL $1"; fail=1; }
chk() { if eval "$2"; then ok "$1"; else bad "$1"; fi; }
unset T6_HAND T6_UNATTENDED T6_UART BOARD_UART T6_CONSOLE_CMD T6_SCREEN BOARD_SCREEN \
	RESOURCE_NANOPC_T6_SSH RESOURCE_NANOPC_T6_CONSOLE_HOST RESOURCE_NANOPC_T6_CONSOLE_PORT \
	RESOURCE_SERIAL_NANOPC_T6_HOST RESOURCE_SERIAL_NANOPC_T6_PORT T6_CONSOLE_LEASE

# ---- the table and its images (three rows) ----
T=$W/tree.txt
printf '# image R1:\ndEQP-GLES2.a.*\n# image R2:\ndEQP-GLES2.b.*\n# image R3:\ndEQP-GLES2.c.*\n' > "$T"
mkdir -p "$W/img"
for l in R1 R2 R3; do
	mkdir -p "$W/img/$l/zephyr"
	printf 'image %s\n' "$l" > "$W/img/$l/zephyr/zephyr.bin"
	echo "$l rc=0 build=selftest" > "$W/img/$l.built"
done

# ---- the fake console: canned board output, for ever ----
cat > "$W/console.sh" <<'EOF'
while :; do
	printf 'T6:A\r\nyakogl-wdt: nanopc_t6 WDT0 armed at boot\r\ndeqp: summary: 3 passed\r\ndeqp: done\r\nt6: deqp: PSCI SYSTEM_RESET at 1234 ms\r\n'
	sleep 0.05
done
EOF

# ---- the fake board.sh ----
cat > "$W/board.sh" <<'EOF'
#!/bin/bash
# fake `board.sh [--dry-run] nanopc-t6 tree <table> <out>`
dry=0
[ "$1" = --dry-run ] && { dry=1; shift; }
[ "$1 $2" = "nanopc-t6 tree" ] || { echo "fake board.sh: bad args $*" >&2; exit 2; }
table=$3 out=$4
BOARD_UART=${BOARD_UART:-${T6_UART:-}}
printf 'fake board.sh: BOARD_HOST=%s BOARD_UART=%s T6_HAND=%s T6_UNATTENDED=%s via=%s\n' \
	"${BOARD_HOST:+set}" "${BOARD_UART%%://*}" "${T6_HAND:+set}" "${T6_UNATTENDED:-}" "${T6_UART_VIA:-}" > "$FAKE_LOG.env"
[ -n "${BOARD_UART:-}" ] && printf '%s' "$BOARD_UART" > "$FAKE_LOG.uart"
if [ "$dry" = 1 ]; then echo "DRY host\$ tree $table"; exit 0; fi
mkdir -p "$out"
awk -v dir="$TREE_IMAGE_DIR" '/^# image /{l=$3; sub(/:$/,"",l); next} /^#|^$/{next}
	{printf "%s\t%s/%s/zephyr/zephyr.bin\t-\t%s\n", l, dir, l, $0}' "$table" > "$out/tree.txt"
while IFS=$'\t' read -r lab img _ _; do
	sha8=$(sha256sum "$img" | cut -c1-8)
	if compgen -G "$out/*-$lab-$sha8/INDEX.txt" > /dev/null; then echo "fake board.sh: $lab: finished run, skipped"; continue; fi
	echo "=== $lab ($(basename "$img")) $(date '+%F %T')"
	rd=$out/$(date +%Y%m%d-%H%M)-$lab-$sha8
	# a second run of a row in the same minute: its own directory
	[ ! -d "$rd" ] || rd=$out/$(date +%Y%m%d-%H%M)-$lab-$sha8.x$RANDOM
	mkdir -p "$rd"
	echo "$lab" >> "$FAKE_LOG.rows"
	if [ "$lab" = "${FAKE_BUSY:-}" ]; then
		echo "t6-uart-rfc2217: busy: another client is connected" > "$rd/kexec.log"
		echo "refused_rc=3" > "$rd/meta.txt"
		echo "=== $lab rc=3 $(date '+%F %T')"; echo "fake board.sh: $lab refused; the chain stops"; exit 3
	fi
	# the runner's rule: T6_HAND, or T6_UNATTENDED=1
	if [ -z "${T6_HAND:-}" ] && [ "${T6_UNATTENDED:-}" != 1 ]; then
		echo "nanopc-t6.sh: REFUSED: --go needs T6_HAND" > "$rd/kexec.log"
		echo "=== $lab rc=3 $(date '+%F %T')"; exit 3
	fi
	# the runner's T6_UART checks and its read-only capture
	if [ -c "$BOARD_UART" ] && [ -r "$BOARD_UART" ] && ! fuser "$BOARD_UART" > /dev/null 2>&1; then
		timeout 0.6 cat < "$BOARD_UART" > "$rd/uart.log"
	else
		echo "fake board.sh: BOARD_UART is no readable free tty" > "$rd/uart.log"
	fi
	if [ "$lab" = "${FAKE_HANG:-}" ]; then
		printf 'jump=ok\nrc_cycle=5\n' > "$rd/meta.txt"
		echo "no Armbian 780 s after the jump: unattended run, nobody at the power switch: incomplete (hang or reset)" > "$rd/record.txt"
		echo "INDEX" > "$rd/INDEX.txt"
		echo "=== $lab rc=5 $(date '+%F %T')"
		echo "fake board.sh: $lab: the board did not come back; the chain stops (never keep booting)"
		exit 5
	fi
	printf 'jump=ok\nback=1\n' > "$rd/meta.txt"
	echo "INDEX" > "$rd/INDEX.txt"
	echo "=== $lab rc=0 $(date '+%F %T')"
done < "$out/tree.txt"
echo TREE-EXIT
EOF
chmod +x "$W/board.sh"
export T6_BOARD_SH=$W/board.sh FAKE_LOG=$W/fake BOARD_HOST=root@t6-selftest-host
R=$W/rec

# 1. --help names the console and the unattended options
h=$("$S" --help 2>&1)
chk "--help names --run, --unattended, T6_HAND, T6_CONSOLE_CMD, BOARD_UART and the lease's console variables" \
	'for w in "--run" "--unattended" "T6_UNATTENDED=1" "T6_HAND" "T6_CONSOLE_CMD" "BOARD_UART" "RESOURCE_NANOPC_T6_CONSOLE" "INCOMPLETE.txt"; do grep -qF -- "$w" <<<"$h" || exit 1; done'

# 2. neither T6_HAND nor --unattended: exit 2, the board untouched
out=$(T6_CONSOLE_CMD="sh $W/console.sh" "$S" --run "$T" "$W/img" "$R" 2>&1); rc=$?
chk "no T6_HAND and no --unattended: exit 2 naming both, board.sh not started (rc $rc)" \
	'[ $rc = 2 ] && grep -qF "T6_HAND=<name>" <<<"$out" && grep -qF -- "--unattended" <<<"$out" && [ ! -e "$FAKE_LOG.env" ]'

# 3. no console at all: exit 2
out=$("$S" --run --unattended "$T" "$W/img" "$R" 2>&1); rc=$?
chk "no console (no T6_CONSOLE_CMD, T6_UART, BOARD_UART or lease variables): exit 2 (rc $rc)" \
	'[ $rc = 2 ] && grep -qF "no console" <<<"$out"'

# 4. a record directory inside a checkout is refused
out=$(T6_CONSOLE_CMD="sh $W/console.sh" "$S" --run --unattended "$T" "$W/img" "$HERE/t6run-rec" 2>&1); rc=$?
chk "a record directory inside the checkout is refused, exit 2 (rc $rc)" \
	'[ $rc = 2 ] && grep -qF "inside a git checkout" <<<"$out" && [ ! -e "$HERE/t6run-rec" ]'

# 5. unattended, the fake console through the pty, R2 hangs
out=$(FAKE_HANG=R2 T6_CONSOLE_CMD="sh $W/console.sh" "$S" --run --unattended "$T" "$W/img" "$R" 2>&1); rc=$?
r1=$(ls -d "$R"/*-R1-*/ 2>/dev/null | head -1)
chk "unattended: board.sh got T6_UNATTENDED=1, no T6_HAND, the console as a pty via T6_CONSOLE_CMD" \
	'grep -qx "fake board.sh: BOARD_HOST=set BOARD_UART=/dev/pts/[0-9]* T6_HAND= T6_UNATTENDED=1 via=T6_CONSOLE_CMD" "$FAKE_LOG.env"'
chk "the console path: R1's uart.log holds the canned board lines read from the pty" \
	'[ -n "$r1" ] && grep -qaF "deqp: done" "$r1/uart.log" && grep -qaF "PSCI SYSTEM_RESET" "$r1/uart.log"'
chk "the hang stops the tree with board.sh's rc 5, R3 not run (rc $rc)" \
	'[ $rc = 5 ] && [ "$(cat "$FAKE_LOG.rows" | paste -sd" ")" = "R1 R2" ]'
inc=$(cat "$R/INCOMPLETE.txt" 2>/dev/null)
chk "INCOMPLETE.txt: stopped at R2 rc 5, hang or reset, unattended" \
	'grep -qF "stopped: rc 5 at R2 (rc=5): hang or reset" <<<"$inc" && grep -qF "unattended: nobody at the power switch" <<<"$inc"'
chk "INCOMPLETE.txt: R2 listed incomplete with its run directory, R3 left, the resume command" \
	'grep -qE "^  R2 $R/[0-9]{8}-[0-9]{4}-R2-[0-9a-f]{8}$" <<<"$inc" && grep -qx "left (no finished run): 1: R3" <<<"$inc" && grep -qxF "resume: $S --run --unattended $T $W/img $R" <<<"$inc"'
chk "the stop's lines are printed too, and board.sh's lines are kept in run-<date>.log" \
	'grep -qF "t6-deqp-images: --run: left (no finished run): 1: R3" <<<"$out" && grep -qF "=== R2 rc=5" "$R"/run-*.log'
chk "the console command's text is not printed" '! grep -qF "$W/console.sh" <<<"$out"'

# 6. the resume: only R3 runs, the tree ends, R2 stays incomplete
: > "$FAKE_LOG.rows"
out=$(T6_CONSOLE_CMD="sh $W/console.sh" "$S" --run --unattended "$T" "$W/img" "$R" 2>&1); rc=$?
inc=$(cat "$R/INCOMPLETE.txt" 2>/dev/null)
chk "the resume runs only R3 and ends (rc $rc)" \
	'[ $rc = 0 ] && [ "$(cat "$FAKE_LOG.rows" | paste -sd" ")" = "R3" ] && grep -qF "R1: finished run, skipped" <<<"$out"'
chk "after the resume INCOMPLETE.txt keeps R2 as incomplete, nothing left" \
	'grep -qF "stopped: rc 0" <<<"$inc" && grep -qE "^  R2 " <<<"$inc" && grep -qx "left (no finished run): 0" <<<"$inc"'

# 7. the hung row moved aside: the next resume runs it, the tree is complete
mkdir -p "$W/aside" && mv "$R"/*-R2-* "$W/aside"/
: > "$FAKE_LOG.rows"
out=$(T6_CONSOLE_CMD="sh $W/console.sh" "$S" --run --unattended "$T" "$W/img" "$R" 2>&1); rc=$?
chk "with R2's directory moved out, R2 runs again and the tree is complete: INCOMPLETE.txt removed (rc $rc)" \
	'[ $rc = 0 ] && [ "$(cat "$FAKE_LOG.rows" | paste -sd" ")" = "R2" ] && [ ! -e "$R/INCOMPLETE.txt" ] && grep -qF "the tree is complete" <<<"$out"'

# 8. the lease's console variables: rfc2217://, the value never printed; a busy adapter named
R2D=$W/rec2
: > "$FAKE_LOG.rows"
out=$(FAKE_BUSY=R1 RESOURCE_NANOPC_T6_CONSOLE_HOST=console-host-7c4e RESOURCE_NANOPC_T6_CONSOLE_PORT=port-7c4e \
	T6_HAND=selftest "$S" --run "$T" "$W/img" "$R2D" 2>&1); rc=$?
chk "the lease's RESOURCE_NANOPC_T6_CONSOLE_HOST/_PORT become BOARD_UART=rfc2217://<host>:<port>" \
	'[ "$(cat "$FAKE_LOG.uart")" = "rfc2217://console-host-7c4e:port-7c4e" ]'
chk "attended: T6_HAND passed on, the source named by variable name only, no host or port printed" \
	'grep -qF "T6_HAND=set" "$FAKE_LOG.env" && grep -qF "RESOURCE_NANOPC_T6_CONSOLE_HOST and RESOURCE_NANOPC_T6_CONSOLE_PORT" <<<"$out" && ! grep -qE "console-host-7c4e|port-7c4e" <<<"$out" "$R2D/INCOMPLETE.txt"'
chk "a busy console stops the tree (rc 3) and INCOMPLETE.txt says the adapter's slot is held (rc $rc)" \
	'[ $rc = 3 ] && grep -qF "the console is busy: the adapter'"'"'s one client slot is held" "$R2D/INCOMPLETE.txt" && grep -qx "left (no finished run): 3: R1 R2 R3" "$R2D/INCOMPLETE.txt" && grep -qxF "resume: $S --run $T $W/img $R2D" "$R2D/INCOMPLETE.txt"'

# 9. the board from the lease; --dry-run passes board.sh's --dry-run
out=$(env -u BOARD_HOST RESOURCE_NANOPC_T6_SSH=root@lease-host-7c4e BOARD_UART=/dev/null T6_UNATTENDED=1 \
	"$S" --run --dry-run "$T" "$W/img" "$W/rec3" 2>&1); rc=$?
chk "BOARD_HOST from RESOURCE_NANOPC_T6_SSH (not printed), --dry-run reaches board.sh's --dry-run, nothing made (rc $rc)" \
	'[ $rc = 0 ] && grep -qF "DRY host\$ tree" <<<"$out" && grep -qF "BOARD_HOST=set" "$FAKE_LOG.env" && ! grep -qF lease-host-7c4e <<<"$out" && [ ! -e "$W/rec3" ]'

# 10. both a console command and a tty: refused
out=$(T6_CONSOLE_CMD="sh $W/console.sh" BOARD_UART=/dev/null T6_UNATTENDED=1 "$S" --run "$T" "$W/img" "$W/rec4" 2>&1); rc=$?
chk "T6_CONSOLE_CMD and BOARD_UART both set: exit 2 (rc $rc)" '[ $rc = 2 ] && grep -qF "set one console" <<<"$out"'

# 11. the bridge's own self-test (pty raw/no echo, drop while closed, restart, read only)
pout=$(python3 -I "$HERE/t6-console-pty.py" --self-test 2>&1); prc=$?
chk "t6-console-pty.py --self-test passes ($(tail -n1 <<<"$pout"))" '[ $prc = 0 ]'

echo "t6-deqp-run-selftest: $n checks, $([ $fail = 0 ] && echo "all ok" || echo "FAILED")"
[ $fail = 0 ]

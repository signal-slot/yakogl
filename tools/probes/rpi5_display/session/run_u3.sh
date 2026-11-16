#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi5_display/session/run_u3.sh [--dry-run] <image>
#
# The U3 board session on the Raspberry Pi 5 (driver/docs/rpi5-display.md),
# one run, no retry. See README.md in this directory. In order:
#   1. print the undo commands (before anything is read or changed);
#   2. sha256 manifest of every /boot file the session may touch;
#   3. linux_ref.sh over ssh (read-only), get_throttled;
#   4. tools/board.sh pi5 run <image> ... (stage, tryboot, console, wait for
#      Linux after the PM watchdog reset, clean up: board.sh as it is);
#   5. manifest again; whatever differs among the files we stage is put back
#      byte for byte (tryboot.txt, the staged image), verified by manifest;
#   6. verdicts.sh on the console log, record.sh for the record directory.
#
# Environment (nothing of it is written to any file of this tree):
#   U3_SSH or BOARD_HOST (PI5_HOST)  ssh target of the board's Linux (lease)
#   BOARD_UART (PI5_UART)            the board's console, used by board.sh
#   U3_NAME           run name, default rpi5_display
#   U3_RETURN_WAIT    seconds to wait for Linux after board.sh (420), U3_POLL (10)
#   U3_BOOT_DIR       the firmware partition on the board (/boot/firmware)
#   overrides for the host self-test: U3_SSH_CMD, U3_BOARD_SH, U3_LINUX_REF,
#   U3_VERDICTS, U3_RECORD
# Exit: 0 done; 4 the probe's verdict is not done; 3 the board did not come
# back (stop, release, a person); 6 the /boot files could not be restored or
# the record keeps a value it must not; 2 usage or refused before any change;
# 1 any other board.sh result.

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PROBE=$(dirname "$HERE")
ROOT=$(cd "$PROBE/../../.." && pwd)

DRY=
if [ "${1:-}" = --dry-run ]; then DRY=1; shift; fi
IMAGE=${1:-}

TARGET=${U3_SSH:-${BOARD_HOST:-${PI5_HOST:-}}}
NAME=${U3_NAME:-rpi5_display}
BOOT=${U3_BOOT_DIR:-/boot/firmware}
RETURN_WAIT=${U3_RETURN_WAIT:-420}
POLL=${U3_POLL:-10}
SSH_CMD=${U3_SSH_CMD:-ssh}
BOARD_SH=${U3_BOARD_SH:-$ROOT/tools/board.sh}
LINUX_REF=${U3_LINUX_REF:-$PROBE/linux_ref.sh}
VERDICTS=${U3_VERDICTS:-$PROBE/verdicts.sh}
RECORD=${U3_RECORD:-$PROBE/record.sh}

say() { printf 'u3 session: %s\n' "$*"; }
die() { say "$*"; exit 2; }

[ -n "$IMAGE" ] && [ -f "$IMAGE" ] || die "usage: run_u3.sh [--dry-run] <image> (the rpi5_display probe image)"
[[ $NAME =~ ^[A-Za-z0-9_.-]+$ ]] || die "U3_NAME has characters outside [A-Za-z0-9_.-]"
[ -n "$TARGET" ] || die "no ssh target (U3_SSH or BOARD_HOST from the lease)"
[ -n "${BOARD_UART:-${PI5_UART:-}}" ] || die "BOARD_UART is not set (board.sh needs the console)"
export BOARD_UART=${BOARD_UART:-${PI5_UART:-}}
export BOARD_HOST=${BOARD_HOST:-$TARGET}

STAGED="yakogl-$NAME.bin"
TOUCH_FILES=("$BOOT/tryboot.txt" "$BOOT/$STAGED")    # staged and removed again
GUARD_FILES=("$BOOT/config.txt" "$BOOT/cmdline.txt") # read, never written
ALL_FILES=("${TOUCH_FILES[@]}" "${GUARD_FILES[@]}")

# 1. the undo, before anything else
say "undo (nothing was changed yet; run these on the board if the session stops half way):"
say "  sudo rm -f $BOOT/$STAGED"
say "  tryboot.txt: if the manifest says it existed, put the saved copy back (sudo tee $BOOT/tryboot.txt); else sudo rm -f $BOOT/tryboot.txt"
say "  config.txt and cmdline.txt are never written by this session"
say "  then: sync; the board boots its stock config on the next reboot (tryboot is one-shot)"

WORK=$(mktemp -d "${TMPDIR:-/tmp}/run_u3.XXXXXX") || exit 2
chmod 700 "$WORK"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/runs"

SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=10)
rsh() { timeout 120 $SSH_CMD "${SSH_OPTS[@]}" "$TARGET" "$@"; }

manifest() { # out-file
	local f cmd=""
	for f in "${ALL_FILES[@]}"; do
		cmd+="if [ -e '$f' ]; then sudo -n sha256sum '$f'; else echo \"absent  $f\"; fi; "
	done
	rsh "$cmd" >"$1" || return 1
	[ "$(wc -l <"$1")" -eq "${#ALL_FILES[@]}" ]
}

wait_linux() {
	local waited=0
	while ! rsh true >/dev/null 2>&1; do
		[ "$waited" -lt "$RETURN_WAIT" ] || return 1
		sleep "$POLL"
		waited=$((waited + POLL))
	done
}

# 2. manifest and a copy of tryboot.txt (small; the one file we may put back)
rsh true >/dev/null 2>&1 || die "the board's Linux does not answer over ssh (nothing was changed)"
manifest "$WORK/manifest.before" || die "cannot take the /boot manifest (nothing was changed)"
if grep -q "^absent  $BOOT/tryboot.txt\$" "$WORK/manifest.before"; then
	TRY_BEFORE=absent
	say "undo for tryboot.txt: sudo rm -f $BOOT/tryboot.txt (there was none)"
else
	TRY_BEFORE=present
	rsh "sudo -n cat '$BOOT/tryboot.txt'" >"$WORK/tryboot.before" || die "cannot read tryboot.txt (nothing was changed)"
	say "undo for tryboot.txt: put the saved copy back with sudo tee $BOOT/tryboot.txt"
fi
grep -q "^absent  $BOOT/$STAGED\$" "$WORK/manifest.before" || die "$BOOT/$STAGED exists already: look at it first (nothing was changed)"
say "manifest before: $(wc -l <"$WORK/manifest.before") files, sha256 taken"

# the record directory
REC=$("$RECORD" new "$NAME" | tail -n 1)
[ -n "$REC" ] && [ -d "$REC" ] || die "record.sh new gave no directory"
cp "$WORK/manifest.before" "$REC/manifest.before.txt"
throttled() { # label
	local v
	v=$(rsh 'vcgencmd get_throttled' 2>/dev/null | head -n 1)
	"$RECORD" throttled "$REC" "$1" <<<"${v:-unknown}" || true
}
throttled before

# 3. the Linux reference: read-only, run as root from stdin, scratch on tmpfs
say "linux_ref.sh over ssh"
rsh 'd=$(mktemp -d /dev/shm/u3ref.XXXXXX) && sudo -n bash -s -- "$d"; r=$?; rm -rf "$d"; exit $r' \
	<"$LINUX_REF" >"$REC/linux_ref.txt" 2>/dev/null ||
	say "linux_ref.sh reported a failure (kept going: the facts are references)"

# 4. board.sh as it is
say "board.sh pi5 run (one run, no retry)"
BS=("$BOARD_SH")
[ -n "$DRY" ] && BS+=(--dry-run)
"${BS[@]}" pi5 run "$IMAGE" "$WORK/runs" "$NAME" </dev/null
RC=$?
say "board.sh rc=$RC"

NOT_BACK=
if [ "$RC" -eq 5 ]; then
	NOT_BACK=1
elif ! wait_linux; then
	NOT_BACK=1
fi

# 5. restore and verify, only when the board answers
RESTORE_FAIL=
if [ -z "$NOT_BACK" ]; then
	if ! manifest "$WORK/manifest.after"; then
		RESTORE_FAIL=1
	else
		if ! diff -q "$WORK/manifest.before" "$WORK/manifest.after" >/dev/null; then
			say "manifest differs after the run; putting back the files we stage"
			rsh "sudo -n rm -f '$BOOT/$STAGED'"
			if [ "$TRY_BEFORE" = absent ]; then
				rsh "sudo -n rm -f '$BOOT/tryboot.txt' && sync"
			else
				rsh "sudo -n tee '$BOOT/tryboot.txt' >/dev/null && sync" <"$WORK/tryboot.before"
			fi
			manifest "$WORK/manifest.after" || RESTORE_FAIL=1
		fi
		if [ -z "$RESTORE_FAIL" ]; then
			if diff "$WORK/manifest.before" "$WORK/manifest.after" >"$WORK/manifest.diff"; then
				say "restore verified: every /boot file is byte for byte as found"
			else
				say "RESTORE FAILED: /boot differs from the manifest taken before:"
				sed 's/^/u3 session:   /' "$WORK/manifest.diff"
				RESTORE_FAIL=1
			fi
		fi
	fi
	cp "$WORK/manifest.after" "$REC/manifest.after.txt" 2>/dev/null
	throttled after
fi

# 6. verdicts and record
CONSOLE=$(find "$WORK/runs" -name console.txt -type f 2>/dev/null | head -n 1)
for f in INDEX.txt meta.txt console.txt; do
	p=$(find "$WORK/runs" -name "$f" -type f 2>/dev/null | head -n 1)
	[ -n "$p" ] && cp "$p" "$REC/$f"
done
if [ -n "$CONSOLE" ]; then
	"$VERDICTS" "$CONSOLE" "$REC/linux_ref.txt" >"$REC/verdicts.txt" 2>&1
	say "verdicts.sh rc=$? (1 is an incomplete log: information, not an error)"
else
	say "no console.txt from board.sh: no verdicts"
fi

HOSTPART=${TARGET#*@}
USERPART=
case $TARGET in *@*) USERPART=${TARGET%@*} ;; esac
REDACT_NAMES="$HOSTPART${USERPART:+,$USERPART}" "$RECORD" redact "$REC" >/dev/null
"$RECORD" check "$REC" >/dev/null || {
	say "record.sh check refused $REC"
	RESTORE_FAIL=1
}
LEAK=
for v in "$TARGET" "$HOSTPART" "$BOARD_UART"; do
	if [ -n "$v" ] && grep -rlF -- "$v" "$REC" >/dev/null 2>&1; then LEAK=1; fi
done
if [ -n "$LEAK" ]; then
	say "the record keeps an environment value: do not keep it, look at it by hand"
	RESTORE_FAIL=1
fi
say "record: $REC"

if [ -n "$NOT_BACK" ]; then
	say "board needs a person"
	exit 3
fi
[ -z "$RESTORE_FAIL" ] || exit 6
case $RC in
0) exit 0 ;;
4) exit 4 ;;
*) exit 1 ;;
esac

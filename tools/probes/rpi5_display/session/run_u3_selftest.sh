#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Host self-test of run_u3.sh: the whole flow with --dry-run against a fake
# /boot directory and fake ssh, board.sh, record.sh, verdicts.sh and
# linux_ref.sh under $TMPDIR. No board, no network.
#   bash tools/probes/rpi5_display/session/run_u3_selftest.sh

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
RUN=$HERE/run_u3.sh
T=$(mktemp -d "${TMPDIR:-/tmp}/run_u3_selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
FAIL=0
ok() { printf 'ok   %s\n' "$1"; }
bad() { printf 'FAIL %s\n' "$1"; FAIL=$((FAIL + 1)); }
check() { if eval "$2"; then ok "$1"; else bad "$1"; fi; }

# fake environment values, made up here so that no literal sits in this tree
N1=$((RANDOM % 80 + 20)); N2=$((RANDOM % 200 + 20))
FAKE_IP="$N1.$N2.7.$((RANDOM % 200 + 20))"
FAKE_HOST="hq$RANDOM-node"
FAKE_USER="u$RANDOM"
FAKE_UART="/dev/$(printf 't%s' ty)FAKE$RANDOM"

mkdir -p "$T/bin"
# ssh shim: skips -o options and the target, runs the command locally
cat >"$T/bin/ssh" <<'EOF'
#!/bin/bash
echo "SHIM ssh" >&2
[ -e "$FAKE_STATE/dead" ] && exit 255
while [ "$1" = -o ]; do shift 2; done
shift
exec bash -c "$*"
EOF
cat >"$T/bin/sudo" <<'EOF'
#!/bin/bash
[ "$1" = -n ] && shift
exec "$@"
EOF
cat >"$T/bin/vcgencmd" <<'EOF'
#!/bin/bash
echo "throttled=0x0"
EOF
# board.sh shim: acts as the scenario says
cat >"$T/bin/board.sh" <<'EOF'
#!/bin/bash
echo "SHIM board.sh $*" >&2
echo x >>"$FAKE_STATE/board_calls"
[ "$1" = --dry-run ] && shift
img=$3; rd=$4/$5; mkdir -p "$rd"
B=$FAKE_BOOT
cp "$img" "$B/yakogl-$5.bin"
printf 'staged tryboot\n' >"$B/tryboot.txt"
printf 'rpi5_display: PASS (x)\n%s %s %s\n' "$FAKE_UART" "$FAKE_TARGET" "$FAKE_IP" >"$rd/console.txt"
printf 'verdict=done back_after=1s\n' >"$rd/INDEX.txt"
printf 'throttled_before=0x0\n' >"$rd/meta.txt"
case "$SCEN" in
leftover) ;;                      # leaves tryboot.txt and the image behind
clean) rm -f "$B/yakogl-$5.bin"; if [ -e "$FAKE_STATE/had_try" ]; then cp "$FAKE_STATE/try.orig" "$B/tryboot.txt"; else rm -f "$B/tryboot.txt"; fi ;;
cfg) rm -f "$B/yakogl-$5.bin"; echo changed >>"$B/config.txt" ;;
dead) touch "$FAKE_STATE/dead"; exit 0 ;;
esac
exit "${BRC:-0}"
EOF
cat >"$T/bin/record.sh" <<'EOF'
#!/bin/bash
case "$1" in
new) d=$FAKE_OUT/records/$2-feedbeef; mkdir -p "$d"; echo "$d" ;;
throttled) { echo "$3 $(cat)"; } >>"$2/throttled.txt" ;;
redact)
	IFS=, read -ra names <<<"$REDACT_NAMES"
	for f in "$2"/*; do
		for n in "${names[@]}"; do sed -i "s|$n|<name>|g" "$f"; done
		sed -i -E -e 's/[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+/<ip>/g' -e 's|/dev/t[t]y[A-Za-z0-9]*|<uart>|g' "$f"
	done ;;
check) ! grep -rqE '[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+' "$2" ;;
esac
EOF
cat >"$T/bin/verdicts.sh" <<'EOF'
#!/bin/bash
echo "route A"; exit 0
EOF
cat >"$T/bin/linux_ref.sh" <<'EOF'
echo "host: $FAKE_TARGET"
echo "get_throttled: 0x0"
EOF
chmod +x "$T"/bin/*

# one scenario: name, scenario, board rc, tryboot before (yes|no)
scen() {
	local name=$1 sc=$2 brc=$3 hadtry=$4
	S=$T/$name
	mkdir -p "$S/boot" "$S/state" "$S/out"
	printf 'arm_64bit=1\nkernel=x' >"$S/boot/config.txt"      # no trailing newline
	printf 'console=x \000 root=y\n' >"$S/boot/cmdline.txt"   # a NUL byte
	rm -f "$S/state/had_try"
	if [ "$hadtry" = yes ]; then
		printf 'tryboot orig\r\n\n' >"$S/boot/tryboot.txt"
		cp "$S/boot/tryboot.txt" "$S/state/try.orig"
		touch "$S/state/had_try"
	fi
	printf 'IMG' >"$S/img"
	(cd "$S/boot" && sha256sum * 2>/dev/null | sort >"$S/before.sha")
	env PATH="$T/bin:$PATH" TMPDIR="$S/out" FAKE_STATE="$S/state" FAKE_BOOT="$S/boot" FAKE_OUT="$S/out" \
		FAKE_TARGET="$FAKE_USER@$FAKE_HOST" FAKE_UART="$FAKE_UART" SCEN=$sc BRC=$brc \
		U3_SSH="$FAKE_USER@$FAKE_HOST" FAKE_IP="$FAKE_IP" BOARD_UART="$FAKE_UART" U3_BOOT_DIR="$S/boot" \
		U3_BOARD_SH="$T/bin/board.sh" U3_RECORD="$T/bin/record.sh" U3_VERDICTS="$T/bin/verdicts.sh" \
		U3_LINUX_REF="$T/bin/linux_ref.sh" U3_RETURN_WAIT=2 U3_POLL=1 \
		bash "$RUN" --dry-run "$S/img" >"$S/log" 2>&1
	RC=$?
	(cd "$S/boot" && sha256sum * 2>/dev/null | sort >"$S/after.sha")
}

# 1. board.sh leaves tryboot.txt and the image behind; tryboot.txt existed
scen leftover_try leftover 0 yes
check "leftover, tryboot.txt existed: exit 0" '[ $RC -eq 0 ]'
check "restore is byte for byte" 'cmp -s "$S/before.sha" "$S/after.sha" && cmp -s "$S/boot/tryboot.txt" "$S/state/try.orig"'
check "staged image removed" '[ ! -e "$S/boot/yakogl-rpi5_display.bin" ]'
check "restore verified line" 'grep -q "restore verified" "$S/log"'
first_undo=$(grep -n 'undo (' "$S/log" | head -n 1 | cut -d: -f1)
first_shim=$(grep -n '^SHIM' "$S/log" | head -n 1 | cut -d: -f1)
check "the undo is printed before any ssh or board.sh call" '[ -n "$first_undo" ] && [ -n "$first_shim" ] && [ "$first_undo" -lt "$first_shim" ]'
check "board.sh called once, with --dry-run, pi5 run" '[ "$(wc -l <"$S/state/board_calls")" -eq 1 ] && grep -q "SHIM board.sh --dry-run pi5 run" "$S/log"'
rec=$(ls -d "$S"/out/records/*)
check "record written (verdicts, console, linux_ref, manifests, throttled)" 'for f in verdicts.txt console.txt linux_ref.txt manifest.before.txt manifest.after.txt throttled.txt; do [ -s "$rec/$f" ] || exit 1; done'
check "get_throttled recorded before and after" '[ "$(wc -l <"$rec/throttled.txt")" -eq 2 ]'
leaks=0
for v in "$FAKE_IP" "$FAKE_HOST" "$FAKE_USER" "$FAKE_UART"; do
	grep -rlF -- "$v" "$S/out" >/dev/null 2>&1 && leaks=1
done
check "no environment value in any written file (record or scratch)" '[ $leaks -eq 0 ]'

# 2. no tryboot.txt before: board.sh's leftover is removed again
scen leftover_none leftover 0 no
check "leftover, no tryboot.txt before: exit 0, restored" '[ $RC -eq 0 ] && cmp -s "$S/before.sha" "$S/after.sha" && [ ! -e "$S/boot/tryboot.txt" ]'

# 3. board.sh cleans up itself: nothing to restore
scen clean clean 0 yes
check "clean run: exit 0, byte for byte" '[ $RC -eq 0 ] && cmp -s "$S/before.sha" "$S/after.sha" && ! grep -q "putting back" "$S/log"'

# 4. board.sh says not back (rc 5): exit 3, one run, no retry, no /boot access after
scen notback leftover 5 yes
check "rc 5: exit 3 and the person message" '[ $RC -eq 3 ] && grep -qx "u3 session: board needs a person" "$S/log"'
check "rc 5: board.sh was not run twice" '[ "$(wc -l <"$S/state/board_calls")" -eq 1 ]'

# 5. board.sh says 0, but ssh stays dead: missing return gives 3
scen dead dead 0 yes
check "no return within the timeout: exit 3" '[ $RC -eq 3 ] && grep -qx "u3 session: board needs a person" "$S/log"'
check "no return: board.sh not retried" '[ "$(wc -l <"$S/state/board_calls")" -eq 1 ]'

# 6. a not-done verdict
scen notdone clean 4 yes
check "board.sh rc 4: exit 4, still restored and recorded" '[ $RC -eq 4 ] && cmp -s "$S/before.sha" "$S/after.sha" && ls "$S"/out/records/*/verdicts.txt >/dev/null'

# 7. config.txt changed by someone else: reported, never written by us
scen cfg cfg 0 yes
check "config.txt changed: exit 6, config.txt left as it was found by the run" '[ $RC -eq 6 ] && grep -q "RESTORE FAILED" "$S/log" && grep -q changed "$S/boot/config.txt"'

# 8. the sources hold no address, host or tty literal
check "no IP, host or tty literal in session/" '! grep -nE "[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}|/dev/t[t]y|raspberr[y]pi|\.loca[l]\b|\.ts\.ne[t]" "$HERE"/*'

echo
if [ $FAIL -eq 0 ]; then echo "run_u3_selftest: all ok"; exit 0; fi
echo "run_u3_selftest: $FAIL failed"
exit 1

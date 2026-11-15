#!/bin/sh
# Self-test of mboxdump.py against the fixtures next to it.
d=$(cd "$(dirname "$0")" && pwd)
f=$d/fixtures
n=0
bad=0

chk() { # name want_status want_text args...
	name=$1
	st=$2
	txt=$3
	shift 3
	n=$((n + 1))
	out=$(python3 -I "$d/mboxdump.py" "$@" 2>&1)
	rc=$?
	if [ "$rc" = "$st" ] && printf '%s\n' "$out" | grep -qF -- "$txt"; then
		echo "ok   $name"
	else
		echo "FAIL $name (rc=$rc want $st, text '$txt')"
		echo "$out"
		bad=$((bad + 1))
	fi
}

chk help 0 "Input format" --help
chk answered 0 "bus_addr=0x3e000000 size=0xbd0800" "$f/allocate_answered.hex"
chk answered-clean 0 "0 flag(s)" "$f/allocate_answered.hex"
chk unanswered 0 "not answered (response bit clear)" "$f/allocate_unanswered.hex"
chk unanswered-strict 1 "1 flag(s)" --strict "$f/allocate_unanswered.hex"
chk short 0 "answered short: 4 of 8 bytes" "$f/allocate_short.hex"
chk truncated 2 "only 6 words given" "$f/truncated.hex"
chk badhex 2 "bad hex word" "$f/badhex.hex"
chk throttle 0 "throttled=0x50005(undervoltage,throttled,undervoltage-seen" "$f/multi_throttle.hex"
chk bare-hex 0 "FRAMEBUFFER_SET_DEPTH" "$f/multi_throttle.hex"
chk two-buffers 0 "buffer 1:" "$f/allocate_answered.hex" "$f/multi_throttle.hex"

n=$((n + 1))
if python3 -I "$d/mboxdump.py" </dev/null >/dev/null 2>&1; then
	echo "FAIL empty input accepted"
	bad=$((bad + 1))
else
	echo "ok   empty input rejected"
fi

echo "$n of $n checks, $bad failed"
[ "$bad" = 0 ]

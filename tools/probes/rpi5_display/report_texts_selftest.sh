#!/usr/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Self-test of report_texts.sh: a fake complete log, a log with missing
# lines, and inputs the generator must refuse (IPv4, tty path, host name,
# USB serial) without echoing them. Needs no board.
set -u
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
RT=$HERE/report_texts.sh
T=$(mktemp -d "${TMPDIR:-/tmp}/rtst.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT

N=0 FAIL=0
chk() { # <name> <command...>: counts one check, passes when the command succeeds
    local name=$1
    shift
    N=$((N + 1))
    if ! "$@"; then FAIL=$((FAIL + 1)); echo "FAIL: $name"; fi
}
has() { grep -qF -- "$2" <<< "$1"; }
hasnt() { ! grep -qF -- "$2" <<< "$1"; }
is() { [ "$1" = "$2" ]; }
cells() { grep -o '|' <<< "$1" | wc -l; }

REC=20261115-1200-u3-0123abcd
SHA=22ca2094

cat > "$T/full.log" <<'EOF'
boot noise
u3 scanout: running (hvs dlist at chan 0)
u3 plane: addr=0x3e000000 fmt=XRGB8888 pitch=7680 w=1920 h=1080
u3 adopt-readback: PASS (4096 bytes)
u3 mailbox-fb: FAIL (tag 0x40001 not handled)
u3 mailbox-readback: FAIL skipped
u3 scanout-follows-mailbox: no
u3 split-screen: drawn
rpi5_display: PASS (u3)
EOF
cat > "$T/part.log" <<'EOF'
u3 scanout: stopped
u3 plane: addr=0x3e000000 fmt=XRGB8888 pitch=7680 w=1920 h=1080
EOF
cat > "$T/ref.txt" <<'EOF'
framebuffer: fmt=xrgb8888 pitch=7680 w=1920 h=1080
linux_ref: done (1 facts, 0 skipped)
EOF

# 1 complete log
out=$("$RT" --verdicts "$T/full.log" --linux-ref "$T/ref.txt" --throttled 0x0 --record $REC --commit $SHA)
rc=$?
chk "complete: exit 0" is "$rc" 0
chk "complete: route A named" has "$out" "route A"
chk "complete: record named" has "$out" "$REC"
chk "complete: commit named" has "$out" "$SHA"
chk "complete: throttled clean" has "$out" "get_throttled: 0x0 (clean)"
chk "complete: U3 row shape" has "$out" "| **U3** Display |"
chk "complete: P11 row shape" has "$out" "| P11 Display (U3) | M to L | P0 |"
chk "complete: no missing marker" hasnt "$out" "(missing)"
u3=$(grep '^| \*\*U3\*\*' <<< "$out" | head -n 1)
p11=$(grep '^| P11' <<< "$out" | head -n 1)
chk "U3 row has two columns" is "$(cells "$u3")" 3
chk "P11 row has four columns" is "$(cells "$p11")" 5

# 2 missing lines, two throttled values
out=$("$RT" --verdicts "$T/part.log" --throttled 0x50005 --throttled 0 --record $REC --commit $SHA)
rc=$?
chk "missing: exit 0" is "$rc" 0
chk "missing: marker" has "$out" "(missing)"
chk "missing: incomplete route" has "$out" "incomplete"
chk "missing: throttled decoded" has "$out" "under-voltage-now"
chk "missing: second throttled value" has "$out" "0 (clean)"
chk "missing: open item" has "$out" "complete the U3 verdict lines"
chk "missing: throttled item" has "$out" "get_throttled was"
out=$("$RT" --verdicts "$T/part.log" --record $REC --commit $SHA)
chk "no --throttled: marker" has "$out" "get_throttled: (missing)"

# 3 refusals: exit 2, the value never echoed
refuse() { # <name> <value> <args...>
    local name=$1 val=$2 o rc
    shift 2
    o=$("$RT" "$@" 2>&1)
    rc=$?
    chk "$name: exit 2" is "$rc" 2
    chk "$name: value not echoed" hasnt "$o" "$val"
    chk "$name: nothing printed" hasnt "$o" "U3 report texts"
}
ip="10.21.7.9"
printf 'u3 scanout: running at %s\n' "$ip" > "$T/ip.log"
refuse ip-in-log "$ip" --verdicts "$T/ip.log" --record $REC --commit $SHA
tty="/dev/ttyUSB7"
printf 'console on %s\n' "$tty" > "$T/tty.log"
refuse tty-in-log "$tty" --verdicts "$T/tty.log" --record $REC --commit $SHA
printf 'u3 scanout: running\n' > "$T/ok.log"
refuse ip-in-throttled "$ip" --verdicts "$T/ok.log" --throttled "$ip" --record $REC --commit $SHA
host="board7.example.net"
printf 'host: %s\n' "$host" > "$T/host.txt"
refuse host-in-ref "$host" --verdicts "$T/ok.log" --linux-ref "$T/host.txt" --record $REC --commit $SHA
ser="usb-FTDI_FT232R_A50285BI-if00"
printf 'by-id %s\n' "$ser" > "$T/ser.log"
refuse usb-serial "$ser" --verdicts "$T/ser.log" --record $REC --commit $SHA
printf 'ref 192.168.1.5 end\n' > "$T/ipref.txt"
refuse ip-in-ref "192.168.1.5" --verdicts "$T/ok.log" --linux-ref "$T/ipref.txt" --record $REC --commit $SHA

# 4 usage errors
"$RT" --verdicts "$T/full.log" --record badname --commit $SHA > /dev/null 2>&1
chk "bad record name: exit 2" is "$?" 2
"$RT" --verdicts "$T/full.log" --record $REC > /dev/null 2>&1
chk "no commit: exit 2" is "$?" 2

echo "report_texts_selftest: $((N - FAIL)) of $N checks, $FAIL failed"
[ "$FAIL" -eq 0 ]

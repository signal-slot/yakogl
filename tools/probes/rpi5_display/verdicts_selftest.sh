#!/usr/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi5_display/verdicts_selftest.sh
#
# Hermetic self-test of verdicts.sh on fixture logs written to $TMPDIR
# (removed at the end; nothing is written into the repository):
#
#   route A   a complete log with a running scanout and a matching adopt
#             read-back -> 'route A', exit 0
#   route B   a complete log where only the mailbox pair passes -> 'route B'
#   route C   a complete log where neither pair passes -> 'route C'
#   cut       a log cut after 'u3 mailbox-fb' -> 'incomplete', exit 1, the
#             stop point named, no route letter
#   gap       a result line with lines skipped between seen ones ->
#             incomplete, the gap named, no 'stopped there' claim
#   redact    a complete log carrying an IPv4, a host:port, an IPv6, a MAC
#             (colon and dash), a /dev/tty path and a tty name in printed
#             FAIL reasons and in noise -> none reach the output; the plane
#             address and a mailbox tag (0x40001) print unredacted
#   linuxref  the log's plane compared with linux_ref facts: agreeing facts,
#             a differ, register dumps and plumbing keys not compared
#   usage     wrong argument count and unreadable files -> exit 2; an
#             empty log -> incomplete, exit 1
set -u
unset LC_ALL LANGUAGE
export LANG=C
HERE=$(cd "$(dirname "$0")" && pwd)
V=$HERE/verdicts.sh
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
check() { case "$out" in *"$1"*) ok "$2" ;; *) bad "$2: '$1' not in output" ;; esac; }
nope()  { case "$out" in *"$1"*) bad "$2: '$1' appeared in output" ;; *) ok "$2" ;; esac; }

T=$(mktemp -d "${TMPDIR:-/tmp}/verdicts-selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
[ -r "$V" ] || { echo "verdicts_selftest: $V not found" >&2; exit 2; }

bash -n "$V" && ok "verdicts.sh parses (bash -n)" || bad "verdicts.sh: bash -n"
bash -n "$0" && ok "verdicts_selftest.sh parses (bash -n)" || bad "selftest: bash -n"

# ---- fixture logs ----------------------------------------------------------
cat > "$T/routeA.log" << 'EOF'
--- Zephyr rpi5_display on BCM2712 (probe fixture) ---
get_throttled 0x50020 next to the boot banner
u3 scanout: running
u3 plane: addr=0x3c000000 fmt=ARGB8888 pitch=7680 w=1920 h=1080
u3 adopt-readback: PASS (32 KiB read back byte-equal)
u3 mailbox-fb: PASS (1920 x 1080 pitch 7680 ARGB8888)
u3 mailbox-readback: PASS
u3 scanout-follows-mailbox: no (the firmware keeps its own scanout address)
u3 split-screen: drawn (text on top, pattern below)
rpi5_display: PASS (route A: the firmware scanout at 0x3c000000 adopted)
rpi5_display: feeding stopped, reset expected
EOF

cat > "$T/routeB.log" << 'EOF'
u3 scanout: stopped
u3 plane: addr=0x3c000000 fmt=ARGB8888 pitch=7680 w=1920 h=1080
u3 adopt-readback: FAIL (no active display list, nothing to read)
u3 mailbox-fb: PASS (1920 x 1080 pitch 7680 ARGB8888)
u3 mailbox-readback: PASS (pattern read back at 0x3c000000)
u3 scanout-follows-mailbox: yes
u3 split-screen: drawn
rpi5_display: PASS (route B: mailbox framebuffer adopted, split screen visible)
EOF

cat > "$T/routeC.log" << 'EOF'
u3 scanout: stopped
u3 plane: w=1920 h=1080 pitch=7680 fmt=ARGB8888 addr=0x3c000000
u3 adopt-readback: FAIL (no active display list)
u3 mailbox-fb: FAIL (tag FRAMEBUFFER_ALLOCATE 0x40001 rejected by the mailbox)
u3 mailbox-readback: FAIL (no framebuffer)
u3 scanout-follows-mailbox: no
u3 split-screen: drawn (on the firmware scanout only)
rpi5_display: FAIL (no framebuffer path works)
EOF

cat > "$T/cut.log" << 'EOF'
u3 scanout: stopped
u3 plane: addr=0x3c000000 fmt=ARGB8888 pitch=7680 w=1920 h=1080
u3 adopt-readback: FAIL (no active display list)
u3 mailbox-fb: PASS (1280 x 720 pitch 5120 RGB565)
EOF

cat > "$T/gap.log" << 'EOF'
u3 scanout: running
u3 adopt-readback: PASS
rpi5_display: PASS (the plane line was never printed)
EOF

cat > "$T/redact.log" << 'EOF'
--- Zephyr rpi5_display ---
get_throttled 0x50020 next to the boot banner
uart bridge arm.example.invalid:40001 on /dev/ttyAMA0 for 100.64.7.9
u3 scanout: running
u3 plane: addr=0x3c000000 fmt=ARGB8888 pitch=7680 w=1920 h=1080
u3 adopt-readback: PASS
u3 mailbox-fb: FAIL (tag FRAMEBUFFER_ALLOCATE 0x40001: arm.example.invalid:40001 timed out toward 100.64.7.9:443 over /dev/ttyUSB3, link local fe80::1, eth de:ad:be:ef:12:34, nic aa-bb-cc-dd-ee-ff, unit serial-getty@ttyS2.service)
u3 mailbox-readback: FAIL (no framebuffer allocated)
u3 scanout-follows-mailbox: no
u3 split-screen: drawn (on the adopted firmware scanout)
rpi5_display: FAIL (mailbox path broken, firmware scanout kept)
EOF

cat > "$T/empty.log" /dev/null

cat > "$T/ref.txt" << 'EOF'
simple-framebuffer: 1920x1080 pitch=7680 format=xrgb8888 addr=0x3c000000
hvs.plane1: addr=0x3c000000 fmt=ARGB8888 pitch=7680 w=1920 h=1080
hvs.plane2: 1920x1080 pitch=7680
hvs.reg.SCALER_DISPCTRL: 0x00040000
hvs.displist.dlist_head: 0x00f12000
framebuffer_alloc: 0x0
cmdline: 8250 quiet
kernel: 6.12.47-v8
get_throttled: 0x0
serial: 0123456789abcdef
host: redacted in the dump
firmware.scanout: addr=0x3c000000 fmt=RGB565 pitch=3840 w=1920 h=1080
linux_ref: done (12 facts, 2 skipped)
EOF

# ---- route A: complete, route named, every line seen -----------------------
out=$(bash "$V" "$T/routeA.log" 2>&1); rc=$?
[ $rc -eq 0 ] && ok "route A log: exit 0" || bad "route A log: rc=$rc, out: $out"
check "verdicts: route A (" "route A named"
check "missing: none" "route A: nothing missing"
check "8 of 8 expected verdict lines" "route A: all eight lines counted"
check "[9] rpi5_display: feeding stopped, reset expected" "the feeding-stopped line printed next to PASS"
check "u3 plane: addr=0x3c000000 fmt=ARGB8888 pitch=7680 w=1920 h=1080" "the plane line prints with its address"
nope "get_throttled" "noise lines are not verdict lines"

# ---- route B: complete, mailbox pair passes, route A does not -------------
out=$(bash "$V" "$T/routeB.log" 2>&1); rc=$?
[ $rc -eq 0 ] && ok "route B log: exit 0" || bad "route B log: rc=$rc, out: $out"
check "verdicts: route B (" "route B named"
nope "verdicts: route A" "route B log: not route A"
nope "verdicts: route C" "route B log: not route C"

# ---- route C: complete, neither pair passes -------------------------------
out=$(bash "$V" "$T/routeC.log" 2>&1); rc=$?
[ $rc -eq 0 ] && ok "route C log: exit 0" || bad "route C log: rc=$rc, out: $out"
check "verdicts: route C (" "route C named"
check "missing: none" "route C: nothing missing"
check "w=1920 h=1080 pitch=7680 fmt=ARGB8888 addr=0x3c000000" "the plane line accepts its fields in any order"
check "0x40001" "route C: the mailbox tag prints unredacted"
nope "verdicts: incomplete" "route C log: complete despite the FAILs"

# ---- cut after 'u3 mailbox-fb': incomplete, stop point named, no route ----
out=$(bash "$V" "$T/cut.log" 2>&1); rc=$?
[ $rc -eq 1 ] && ok "cut log: exit 1" || bad "cut log: rc=$rc, out: $out"
check "verdicts: incomplete" "cut log: incomplete"
check "4 of 8 expected verdict lines" "cut log: four lines seen"
check "missing: u3 mailbox-readback" "cut log names u3 mailbox-readback"
check "rpi5_display terminal" "cut log names the terminal line"
check "[4] u3 mailbox-fb: PASS (1280 x 720 pitch 5120 RGB565): the image stopped there" \
    "cut log: the stop point is named"
if grep -q '^verdicts: route' <<< "$out"; then bad "cut log names a route"; else ok "cut log names no route"; fi

# ---- gap: a result line, lines skipped between seen ones ------------------
out=$(bash "$V" "$T/gap.log" 2>&1); rc=$?
[ $rc -eq 1 ] && ok "gap log: exit 1" || bad "gap log: rc=$rc, out: $out"
check "verdicts: incomplete" "gap log: incomplete"
check "gap: missing between seen lines: u3 plane" "gap log names the gap"
nope "the image stopped there" "gap log: no stop claim after a terminal line"
if grep -q '^verdicts: route' <<< "$out"; then bad "gap log names a route"; else ok "gap log names no route"; fi

# ---- redaction: addresses and UART paths never reach the output ----------
out=$(bash "$V" "$T/redact.log" 2>&1); rc=$?
[ $rc -eq 0 ] && ok "redact log: exit 0" || bad "redact log: rc=$rc, out: $out"
check "verdicts: route A (" "redact log: still route A (adopt pair passes)"
check "addr=0x3c000000" "the plane DRAM address prints unredacted"
check "0x40001" "the mailbox tag 0x40001 prints unredacted"
nope "100.64.7.9" "no IPv4 address"
nope "100.64" "no IPv4 fragment"
nope "arm.example.invalid" "no host name with a port"
nope "fe80" "no IPv6 address"
nope "de:ad:be:ef" "no MAC address (colon form)"
nope "aa-bb-cc-dd-ee-ff" "no MAC address (dash form)"
nope "/dev/ttyUSB3" "no /dev/tty path"
nope "ttyUSB3" "no UART device name"
nope "ttyS2" "no tty name inside a unit"
nope "arm.example" "no host from the noise lines"
nope "0x50020" "the boot banner is not printed"
check "<host-port>" "a host:port marker is printed"
check "<ipv6/mac>" "an IPv6/MAC marker is printed"
check "<uart>" "a UART marker is printed"

# ---- linux_ref comparison --------------------------------------------------
out=$(bash "$V" "$T/routeA.log" "$T/ref.txt" 2>&1); rc=$?
[ $rc -eq 0 ] && ok "with linux_ref: exit 0 (the comparison changes nothing)" \
    || bad "with linux_ref: rc=$rc, out: $out"
check "verdicts: route A (" "linux_ref: the route line still names route A"
check "linux_ref: the done line is there" "linux_ref: the done line seen"
check "ref [simple-framebuffer]: fmt-differ(xrgb8888/ARGB8888)" \
    "simple-framebuffer: fmt differ, and its pitch/w/h agree"
check "pitch-agree" "a pitch agreement is reported"
check "ref [hvs.plane1]: fmt-agree pitch-agree w-agree h-agree" "hvs plane fact agrees field by field"
check "ref [hvs.plane2]: pitch-agree w-agree h-agree" "the WxH pair fills w and h"
check "fmt-differ(RGB565/ARGB8888) pitch-differ(3840/7680)" "a firmware fact with other values differs"
check "4 fact(s) compared, 2 agree, 2 differ" "the comparison counts"
nope "ref [hvs.reg" "a bare register dump is not compared"
nope "ref [framebuffer_alloc" "a bare 0x status value is not a fact"
nope "ref [serial" "plumbing keys are not facts"
nope "ref [cmdline" "the cmdline is not a fact"

# ---- usage and unreadable files --------------------------------------------
out=$(bash "$V" 2>&1); rc=$?
[ $rc -eq 2 ] && ok "no arguments: exit 2" || bad "no arguments: rc=$rc, out: $out"
out=$(bash "$V" "$T/routeA.log" "$T/ref.txt" extra 2>&1); rc=$?
[ $rc -eq 2 ] && ok "three arguments: exit 2" || bad "three arguments: rc=$rc, out: $out"
out=$(bash "$V" "$T/no-such.log" 2>&1); rc=$?
[ $rc -eq 2 ] && ok "unreadable log: exit 2" || bad "unreadable log: rc=$rc, out: $out"
out=$(bash "$V" "$T/routeA.log" "$T/no-such-ref.txt" 2>&1); rc=$?
[ $rc -eq 2 ] && ok "unreadable linux_ref: exit 2" || bad "unreadable linux_ref: rc=$rc, out: $out"
out=$(bash "$V" "$T/empty.log" 2>&1); rc=$?
[ $rc -eq 1 ] && ok "empty log: exit 1" || bad "empty log: rc=$rc, out: $out"
check "verdicts: incomplete" "empty log: incomplete"
check "no expected verdict line was seen at all" "empty log says nothing was seen"

# ---- summary ---------------------------------------------------------------
if [ $fail -eq 0 ]; then
    echo "verdicts_selftest: all checks passed"
else
    echo "verdicts_selftest: $fail failed"
    exit 1
fi

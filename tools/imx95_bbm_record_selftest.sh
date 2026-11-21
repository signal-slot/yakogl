#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# imx95_bbm_record_selftest.sh - tools/imx95_bbm_record.sh without a board.
#
#   tools/imx95_bbm_record_selftest.sh
#
# 1. The command bytes: the header words soc::imx95::bbm's test pins
#    (driver/rust/src/soc/imx95/bbm_tests.rs), GPR and value after them, a
#    value with a '%' byte in it intact.
# 2. The reply checks: a value, DENIED and NOT_FOUND (rc 4), another
#    command's header and no reply (rc 5).
# 3. read and clear against a fake System Manager (BBM_XFER: the BBM
#    protocol's GPR_SET and GPR_GET over eight GPRs, AP-NS's permissions of
#    Toradex's verdin-imx95 configuration, every message logged): the four
#    words of a stored record in the order w0..w3; clear zeroes GPR4 first;
#    a GPR the agent has no permission on stops the read (rc 4). Run through
#    "sh -s" as tools/board.sh runs it on the board, under dash and busybox
#    when the host has them.
# 4. Without the raw interface: probe and read exit 3 and say why.
# Prints "imx95_bbm_record_selftest: N checks, F failed"; exit 0 only when F
# is 0.
set -uo pipefail
export LC_ALL=C
HERE=$(cd "$(dirname "$0")" && pwd)
T="$HERE/imx95_bbm_record.sh"
S=$(mktemp -d)
trap 'rm -rf "$S"' EXIT
n=0
fails=0
check() {
    local what=$1
    shift
    n=$((n + 1))
    if "$@"; then echo "  PASS  $what"; else echo "  FAIL  $what"; fails=$((fails + 1)); fi
}

echo "== 1. the command bytes"
check "get GPR4: header 0x00520404 (0x81 << 10 | 4 | token 0x14 << 18), then 4" \
    eval '[ "$(sh "$T" encode get 4)" = "00520404 00000004" ]'
check "set GPR7 to 0x594b0225: header 0x005e0403, 7, the value ('%' byte kept)" \
    eval '[ "$(sh "$T" encode set 7 0x594b0225)" = "005e0403 00000007 594b0225" ]'
check "set GPR5 to 0: header 0x00560403" eval '[ "$(sh "$T" encode set 5 0)" = "00560403 00000005 00000000" ]'

echo "== 2. the reply"
check "a get's value" eval '[ "$(sh "$T" decode get 5 00560404 00000000 deadbeef)" = deadbeef ]'
out=$(sh "$T" decode get 5 00560404 fffffffd 2>&1); rc=$?
check "DENIED: rc 4, named" eval '[ "$rc" = 4 ] && grep -q DENIED <<<"$out"'
out=$(sh "$T" decode set 9 00660403 fffffffc 2>&1); rc=$?
check "NOT_FOUND: rc 4, named" eval '[ "$rc" = 4 ] && grep -q NOT_FOUND <<<"$out"'
out=$(sh "$T" decode get 5 00520404 00000000 1 2>&1); rc=$?
check "another command's header: rc 5" eval '[ "$rc" = 5 ] && grep -q "not 00560404" <<<"$out"'
out=$(sh "$T" decode get 5 2>&1); rc=$?
check "no reply: rc 5" eval '[ "$rc" = 5 ] && grep -q "no reply" <<<"$out"'
check "a set's reply (header, status 0): nothing printed, rc 0" eval '[ -z "$(sh "$T" decode set 4 00520403 00000000)" ]'

echo "== 3. read and clear against a fake System Manager"
# the fake SM: one command on stdin, its reply on stdout; GPRs and the log in files
cat > "$S/sm.py" <<'PY'
import os, struct, sys
st, log = os.environ["FAKE_GPR"], os.environ["FAKE_LOG"]
gpr = [int(x, 16) for x in open(st).read().split()]
perm = set(int(x) for x in os.environ.get("FAKE_PERM", "4 5 6 7").split())
cmd = sys.stdin.buffer.read()
w = struct.unpack("<%dI" % (len(cmd) // 4), cmd)
hdr, msg, proto = w[0], w[0] & 0xff, (w[0] >> 10) & 0xff
assert proto == 0x81 and (hdr >> 8) & 3 == 0, hex(hdr)
open(log, "a").write(" ".join("%08x" % x for x in w) + "\n")
def reply(status, *v):
    sys.stdout.buffer.write(struct.pack("<Ii%dI" % len(v), hdr, status, *v))
if w[1] >= 8:
    reply(-4)
elif w[1] not in perm:
    reply(-3)
elif msg == 3:
    gpr[w[1]] = w[2]
    open(st, "w").write(" ".join("%08x" % x for x in gpr))
    reply(0)
elif msg == 4:
    reply(0, gpr[w[1]])
else:
    reply(-1)
PY
export BBM_XFER="python3 -I $S/sm.py"
export FAKE_GPR=$S/gpr FAKE_LOG=$S/log
shells=(sh)
for s in dash "busybox sh"; do
    if command -v "${s%% *}" > /dev/null 2>&1; then shells+=("$s"); fi
done
for sh in "${shells[@]}"; do
    echo "a5a5a5a5 0 0 0 594b0201 00000001 0000beef 25252525" > "$S/gpr"
    : > "$S/log"
    out=$($sh -s -- read < "$T" 2>&1); rc=$?
    check "$sh -s read: rc 0, GPR4..7 as w0..w3" eval '[ "$rc" = 0 ] && [ "$out" = "594b0201 00000001 0000beef 25252525" ]'
    check "$sh -s read: four GPR_GETs, GPR4 to GPR7, GPR0..3 never touched" \
        eval '[ "$(cut -d" " -f1,2 "$S/log" | tr "\n" ,)" = "00520404 00000004,00560404 00000005,005a0404 00000006,005e0404 00000007," ]'
    : > "$S/log"
    out=$($sh -s -- clear < "$T" 2>&1); rc=$?
    check "$sh -s clear: rc 0, the record read back as zeros" \
        eval '[ "$rc" = 0 ] && [ "$out" = "00000000 00000000 00000000 00000000" ] && [ "$(cut -d" " -f1-4 "$S/gpr")" = "a5a5a5a5 00000000 00000000 00000000" ]'
    check "$sh -s clear: GPR4 (the magic) zeroed first, then GPR5..7, then the read" \
        eval '[ "$(head -4 "$S/log" | cut -d" " -f2,3 | tr "\n" ,)" = "00000004 00000000,00000005 00000000,00000006 00000000,00000007 00000000," ] &&
              [ "$(sed -n 5p "$S/log" | cut -d" " -f1)" = 00520404 ]'
    out=$(FAKE_PERM="4 5 7" $sh -s -- read < "$T" 2>&1); rc=$?
    check "$sh -s read, GPR6 not the agent's: rc 4, DENIED at GPR6" eval '[ "$rc" = 4 ] && grep -q "GPR6: .*DENIED" <<<"$out"'
done
unset BBM_XFER

echo "== 4. without the raw interface"
mkdir -p "$S/noraw"
out=$(BBM_RAW=$S/noraw sh "$T" probe 2>&1); rc=$?
check "probe: rc 3, names CONFIG_ARM_SCMI_RAW_MODE_SUPPORT_COEX" eval '[ "$rc" = 3 ] && grep -q RAW_MODE_SUPPORT_COEX <<<"$out"'
out=$(BBM_RAW=$S/noraw sh "$T" read 2>&1); rc=$?
check "read: rc 3, says the record cannot be read from Linux" eval '[ "$rc" = 3 ] && grep -q "cannot be read from Linux" <<<"$out"'
out=$(sh "$T" bogus 2>&1); rc=$?
check "an unknown verb: rc 2, the usage" eval '[ "$rc" = 2 ] && grep -q usage <<<"$out"'

echo "imx95_bbm_record_selftest: $n checks, $fails failed"
[ "$fails" = 0 ]

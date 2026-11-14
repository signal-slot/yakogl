#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_env_diff_selftest.sh - tools/mp2s_env_diff.py on generated dumps, no board.
#
#   tools/mp2s_env_diff_selftest.sh
#
# Restored, a leftover mp2s_left, a changed boot_prefixes, a changed bootcount,
# a mid-run state (--expect-default), redaction, and a raw image pair.
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/mp2s_env_diff.py
work=$(mktemp -d "$TMPDIR/mp2s-env-diff.XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT
fails=0
fail() { echo "FAIL: $*"; fails=$((fails + 1)); }

cat >"$work/before" <<'X'
bootcmd=run bootcmd_stm32mp
altbootcmd=run altbootcmd_stm32mp
bootcount=1
bootlimit=3
upgrade_available=0
boot_prefixes=/ /boot/
ethaddr=00:80:e1:12:34:56
serial#=SECRET123
ipaddr=10.9.8.7
loadaddr=0xc2000000
filter=a=b=c
X
mk() { # name sed-expr...
    local out=$work/$1
    shift
    sed "$@" "$work/before" >"$out"
}
mk after_ok -e 's/^bootcount=.*/bootcount=1/'
mk after_left -e '$a mp2s_left=0'
mk after_pfx -e 's|^boot_prefixes=.*|boot_prefixes=/boot/ /|'
mk after_cnt -e 's/^bootcount=1/bootcount=2/'
mk after_mid -e 's|^boot_prefixes=.*|boot_prefixes=/yakogl/standalone/ / /boot/|' \
    -e '$a mp2s_ver=1' -e '$a mp2s_left=2' -e '$a mp2s_prefixes=/ /boot/'
mk after_red -e 's/^ethaddr=.*/ethaddr=11:22:33:44:55:66/' -e 's/^serial#=.*/serial#=NEWSECRET/' \
    -e 's/^ipaddr=.*/ipaddr=10.0.0.1/' -e 's/^loadaddr=.*/loadaddr=0xc3000000/'

run() {
    out=$("$@" 2>&1)
    rc=$?
}
want() { # label rc verdict
    [ "$rc" = "$2" ] || fail "$1: exit $rc, want $2: $out"
    [ "$(printf '%s\n' "$out" | grep -c '^mp2s-env: ')" = 1 ] || fail "$1: not exactly one verdict line: $out"
    printf '%s\n' "$out" | grep -qF -- "$3" || fail "$1: no '$3' in: $out"
}

run python3 -I "$tool" "$work/before" "$work/after_ok"
want restored 0 "mp2s-env: restored"
[ "$(printf '%s\n' "$out" | tail -n1)" = "mp2s-env: restored" ] || fail "restored: verdict not last"
run python3 -I "$tool" "$work/before" "$work/after_left"
want leftover 1 "mp2s-env: NOT restored: mp2s_left is still set"
run python3 -I "$tool" "$work/before" "$work/after_pfx"
want prefixes 1 "NOT restored: boot_prefixes differs"
run python3 -I "$tool" "$work/before" "$work/after_cnt"
want bootcount 1 "NOT restored: bootcount differs"
printf '%s\n' "$out" | grep -qF "changed bootcount: 1 -> 2" || fail "bootcount: no changed line"
run python3 -I "$tool" "$work/before" "$work/after_mid" --expect-default 2
want mid 0 "mp2s-env: default set"
run python3 -I "$tool" "$work/before" "$work/after_mid" --expect-default
want mid-any 0 "mp2s-env: default set"
run python3 -I "$tool" "$work/before" "$work/after_mid" --expect-default 1
want mid-wrong-n 1 "NOT default set: mp2s_left is not 1"
run python3 -I "$tool" "$work/before" "$work/after_ok" --expect-default
want mid-none 1 "NOT default set: mp2s_ver is not 1"
run python3 -I "$tool" "$work/before" "$work/after_mid"
want mid-as-final 1 "NOT restored"
run python3 -I "$tool" "$work/before" "$work/after_red"
for s in SECRET123 NEWSECRET 00:80:e1 11:22:33 10.9.8.7 10.0.0.1; do
    if printf '%s\n' "$out" | grep -qF "$s"; then fail "redaction: $s printed"; fi
done
printf '%s\n' "$out" | grep -qF "changed ethaddr: <redacted> -> <redacted>" || fail "redaction: ethaddr line: $out"
printf '%s\n' "$out" | grep -qF "loadaddr: 0xc2000000 -> 0xc3000000" || fail "redaction: loadaddr hidden: $out"

# raw images through tools/uboot_env.py
python3 -I - "$work" <<'PY'
import sys, zlib
w = sys.argv[1]
def img(path, items, size=256):
    d = b"".join(i.encode() + b"\0" for i in items) + b"\0"
    d = d.ljust(size - 4, b"\0")
    open(path, "wb").write(zlib.crc32(d).to_bytes(4, "little") + d)
img(w + "/i_before", ["boot_prefixes=/ /boot/", "bootcount=1"])
img(w + "/i_after", ["boot_prefixes=/ /boot/", "bootcount=1"])
img(w + "/i_bad", ["boot_prefixes=/ /boot/", "bootcount=1", "mp2s_ver=1"])
PY
run python3 -I "$tool" --image "$work/i_before" "$work/i_after"
want image-ok 0 "mp2s-env: restored"
run python3 -I "$tool" --image "$work/i_before" "$work/i_bad"
want image-bad 1 "NOT restored: mp2s_ver is still set"

if [ "$fails" = 0 ]; then
    echo "mp2s_env_diff_selftest: ok"
    exit 0
fi
echo "mp2s_env_diff_selftest: $fails failure(s)"
exit 1

#!/usr/bin/env bash
# No-board selftest of tools/fwu_metadata.py: synthetic v1 and v2 images
# (accepted, trial, bad CRC, copies disagree, truncated), every exit code
# and message.  Writes only to its own mktemp directory.
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/fwu_metadata.py
d=$(mktemp -d "${TMPDIR:-/tmp}/fwu_selftest.XXXXXX") || exit 2
trap 'rm -rf "$d"' EXIT

python3 -I - "$d" <<'PY' || exit 2
import struct, sys, zlib
d = sys.argv[1]
def pad(b): return b + b"\xff" * (4096 - len(b))
def v2(active=0, states=(0xFC, 0xFC, 0xFF, 0xFF), prev=1, ver=2, bad_crc=False):
    b = struct.pack("<IIIIIHH4BI", 0, ver, active, prev, 32, 0, 0, *states, 0)
    c = zlib.crc32(b[4:]) & 0xFFFFFFFF
    if bad_crc: c ^= 1
    return struct.pack("<I", c) + b[4:]
def v1(active=0, acc=(1, 1), prev=1, bad_crc=False):
    body = struct.pack("<III", 1, active, prev) + b"T" * 16 + b"L" * 16
    for a in acc:
        body += b"I" * 16 + struct.pack("<II", a, 0)
    c = zlib.crc32(body) & 0xFFFFFFFF
    if bad_crc: c ^= 1
    return struct.pack("<I", c) + body
def w(n, b, full=True):
    open("%s/%s" % (d, n), "wb").write(pad(b) if full else b)
w("v2_acc", v2())
w("v2_trial", v2(states=(0xFE, 0xFC, 0xFF, 0xFF)))
w("v2_trial_b1", v2(active=1, states=(0xFC, 0xFE, 0xFF, 0xFF)))
w("v2_badcrc", v2(bad_crc=True))
w("v2_other", v2(active=1))
w("v2_trunc", v2()[:20], False)
w("v2_ver9", v2(ver=9))
w("v2_invalid", v2(states=(0xFF, 0xFC, 0xFF, 0xFF)))
w("v1_acc", v1())
w("v1_trial", v1(acc=(0, 1)))
w("v1_badcrc", v1(bad_crc=True))
w("v1_other", v1(prev=0))
w("v1_trunc", v1()[:30], False)
w("empty", b"", False)
PY

fail=0
# t NAME WANT_RC WANT_TEXT args...
t() {
    local name=$1 rc=$2 want=$3
    shift 3
    local out got
    out=$(python3 -I "$tool" "$@" 2>&1)
    got=$?
    if [ "$got" != "$rc" ] || [[ $out != *"$want"* ]]; then
        echo "FAIL $name: rc=$got (want $rc), output: $out (want '$want')"
        fail=$((fail + 1))
    else
        echo "ok   $name"
    fi
}

t v2-accepted 0 "fwu: bank 0 accepted" check "$d/v2_acc"
t v2-two-copies 0 "fwu: bank 0 accepted" check "$d/v2_acc" "$d/v2_acc"
t v2-trial 3 "fwu: bank 0 in trial (0xFE), refusing" check "$d/v2_trial"
t v2-active-bank1-trial 3 "fwu: bank 1 in trial (0xFE), refusing" check "$d/v2_trial_b1"
t v2-booted-bank-override 0 "fwu: bank 1 accepted" check "$d/v2_trial" --booted-bank 1
t v2-bank-range 2 "out of range" check "$d/v2_acc" --booted-bank 7
t v2-invalid-state 2 "0xFF" check "$d/v2_invalid"
t v2-bad-crc 2 "bad CRC" check "$d/v2_badcrc"
t v2-disagree 2 "copies disagree" check "$d/v2_acc" "$d/v2_other"
t v2-one-bad-copy 2 "bad CRC" check "$d/v2_acc" "$d/v2_badcrc"
t v2-truncated 2 "truncated" check "$d/v2_trunc"
t v2-unknown-version 2 "unknown version 9" check "$d/v2_ver9"
t unreadable 2 "cannot open" check "$d/missing"
t empty 2 "truncated" check "$d/empty"
t v1-accepted 0 "fwu: bank 0 accepted" check "$d/v1_acc"
t v1-trial 3 "fwu: bank 0 in trial (accepted=0), refusing" check "$d/v1_trial"
t v1-bad-crc 2 "bad CRC" check "$d/v1_badcrc"
t v1-disagree 2 "copies disagree" check "$d/v1_acc" "$d/v1_other"
t v1-truncated 2 "truncated" check "$d/v1_trunc"
t show-v2 0 "bank 0: 0xFC" show "$d/v2_acc"
t show-v2-trial 0 "bank 0: 0xFE" show "$d/v2_trial"
t show-v1 0 "version: 1" show "$d/v1_acc"
t show-v1-trial 0 "bank 0: not accepted" show "$d/v1_trial"
t usage 1 "usage" check
t show-bad 2 "bad CRC" show "$d/v2_badcrc"

# read-only: inputs are unchanged
before=$(cksum "$d"/v2_acc "$d"/v1_acc)
python3 -I "$tool" check "$d/v2_acc" "$d/v1_acc" >/dev/null 2>&1
[ "$(cksum "$d"/v2_acc "$d"/v1_acc)" = "$before" ] || { echo "FAIL inputs changed"; fail=$((fail + 1)); }
chmod a-w "$d/v2_acc"
t read-only-input 0 "accepted" check "$d/v2_acc"

if [ "$fail" -ne 0 ]; then
    echo "fwu_metadata_selftest: $fail failed"
    exit 1
fi
echo "fwu_metadata_selftest: ok"

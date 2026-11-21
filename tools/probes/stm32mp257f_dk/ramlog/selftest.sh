#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# selftest.sh - ramlog_decode.py against synthetic ring dumps.
#
#   bash tools/probes/stm32mp257f_dk/ramlog/selftest.sh [--regen]
#
# A small Python generator builds the dumps (empty, short, exact, wrapped,
# bad magic, bad version, truncated, tiny, invalid UTF-8) in $TMPDIR; stdout
# of the decoder must equal testdata/<case>.want ("# " lines are the SPDX
# header and are left out); exit code and stderr line are checked here. Also:
# a pass log of the verdict tool, wrapped into a ring behind junk, decodes to
# the same text and gives the same "dk_verdict.py --expect return-trial"
# line; the U-Boot snippet is one line with the magic check, write and clear.
# Ends with "N checks, 0 failed". --regen rewrites the .want files.
set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
tool=$here/ramlog_decode.py
data=$here/testdata
vdir=$here/../verdict
regen=0
[ "${1:-}" = --regen ] && regen=1
tmp=$(mktemp -d "${TMPDIR:-/tmp}/ramlog-selftest.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT

n=0; failed=0
ok() { n=$((n + 1)); echo "ok   $1"; }
bad() { n=$((n + 1)); failed=$((failed + 1)); echo "FAIL $1"; }

python3 -I - "$tmp" "$vdir/testdata/return-trial.pass.log" <<'PY' || { echo "generator failed"; exit 1; }
import struct, sys
out, passlog = sys.argv[1], sys.argv[2]
def ring(stream, size, magic=0x474C4B59, version=1, flags=0, seq=1):
    """stream: all bytes ever written; the ring keeps the last `size`."""
    data = bytearray(size)
    for i, b in enumerate(stream):
        data[i % size] = b
    return struct.pack("<8I", magic, version, size, len(stream), seq, flags, 0, 0) + bytes(data)
def put(name, blob):
    open("%s/%s.bin" % (out, name), "wb").write(blob)
put("empty", ring(b"", 64))
put("short", ring(b"MP2:A EL2\nMP2:B up\n", 64))
put("exact", ring(b"0123456789abcd\n", 16))
put("wrapped", ring(b"".join(b"line%02d\n" % i for i in range(8)), 24, flags=1))   # 56 bytes, 24 kept
put("badmagic", ring(b"x", 16, magic=0x12345678))
put("badversion", ring(b"x", 16, version=2))
put("truncated", ring(b"0123456789abcd\n", 16)[:-5])
put("tiny", b"YKLG")
put("badutf8", ring(b"ok \xff\xfe bytes\n", 32))
text = b"".join(l for l in open(passlog, "rb") if not l.startswith(b"# "))
junk = b"".join(b"junk line %d\n" % i for i in range(5))
put("trial", ring(junk + text, len(text), flags=1))
open(out + "/trial.src", "wb").write(text)
PY

for c in empty short exact wrapped badmagic badversion truncated tiny badutf8; do
    case $c in
        wrapped) rc=0; err='[ramlog: 32 bytes lost]' ;;
        badmagic|badversion|tiny) rc=3; err='ramlog: no ring' ;;
        truncated) rc=4; err='*' ;;
        *) rc=0; err= ;;
    esac
    python3 -I "$tool" "$tmp/$c.bin" >"$tmp/$c.out" 2>"$tmp/$c.err"
    got=$?
    if [ "$got" = "$rc" ]; then ok "$c exit $rc"; else bad "$c exit $got, want $rc"; fi
    if [ "$err" = '*' ]; then
        if [ -s "$tmp/$c.err" ]; then ok "$c stderr set"; else bad "$c stderr empty"; fi
    elif [ "$(cat "$tmp/$c.err")" = "$err" ]; then ok "$c stderr"; else bad "$c stderr: $(cat "$tmp/$c.err")"; fi
    if [ $regen = 1 ]; then
        { printf '# SPDX-License-Identifier: Apache-2.0\n# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.\n'; cat "$tmp/$c.out"; } >"$data/$c.want"
    fi
    if [ -f "$data/$c.want" ] && grep -v '^# ' "$data/$c.want" >"$tmp/$c.exp" 2>/dev/null || [ -f "$data/$c.want" ]; then
        if cmp -s "$tmp/$c.exp" "$tmp/$c.out"; then ok "$c text"; else bad "$c text differs from testdata/$c.want"; fi
    else
        bad "testdata/$c.want missing"
    fi
done

# a wrapped ring behind junk decodes to the original pass log, same verdict
python3 -I "$tool" "$tmp/trial.bin" >"$tmp/trial.out" 2>"$tmp/trial.err"
if cmp -s "$tmp/trial.out" "$tmp/trial.src"; then ok "trial text equals the log"; else bad "trial text differs"; fi
if grep -q 'bytes lost' "$tmp/trial.err"; then ok "trial reports the junk lost"; else bad "trial: no loss line"; fi
want=$(python3 -I "$vdir/dk_verdict.py" --expect return-trial "$vdir/testdata/return-trial.pass.log" | head -1)
got=$(python3 -I "$vdir/dk_verdict.py" --expect return-trial "$tmp/trial.out" | head -1)
if [ -n "$want" ] && [ "$want" = "$got" ]; then ok "verdict line: $got"; else bad "verdict '$got' != '$want'"; fi

# the snippet: one command line with the magic check, the write and the clear
cmd=$(grep -v '^# ' "$here/uboot_snippet.txt")
if [ "$(printf '%s\n' "$cmd" | wc -l)" = 1 ]; then ok "snippet is one line"; else bad "snippet is not one line"; fi
case $cmd in
    *'itest.l *${yako_ramlog_addr} == 0x474C4B59'*'ext4write mmc ${boot_instance}:${yako_part} ${yako_ramlog_addr} /yakogl/ramlog.bin ${yako_ramlog_size}'*'mw.l ${yako_ramlog_addr} 0 1'*) ok "snippet order" ;;
    *) bad "snippet lacks the magic check, the write or the clear in order" ;;
esac

echo "$n checks, $failed failed"
[ "$failed" = 0 ]

#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mp2_deqp_slot_selftest.sh [zephyr.bin]
#
# Host-only check of tools/cts/mp2-deqp-slot.py on a synthetic image (random
# bytes around one slot laid out as driver/samples/zephyr/deqp/src/main.cpp
# lays it out): an unwritten slot, a selector and a filter written and read
# back, a case list, --clear, the refusals (no slot, two slots, a filter too
# long, a bad pattern, a selector past the table) and that no byte outside
# the slot changes. Given a DK dEQP zephyr.bin (built with
# CONFIG_DEQP_CASE_SLOT=y), the same round trip runs on a copy of it too.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/mp2-deqp-slot.py
REAL=${1:-}
D=$(mktemp -d "${TMPDIR:-/tmp}/mp2-deqp-slot.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
chk() { local l=$1; shift; if "$@"; then ok "$l"; else bad "$l"; fi; }
tool() { python3 -I "$TOOL" "$@" 2>&1; }
# outside <a> <b>: the files differ only inside the slot (offset 0x.. + 36 + size)
outside() {
    python3 -I - "$1" "$2" <<'PY'
import sys
a = open(sys.argv[1], "rb").read(); b = open(sys.argv[2], "rb").read()
m = b"yakogl-deqp-case-slot/1\0"; at = a.find(m)
size = int.from_bytes(a[at + 24:at + 28], "little"); end = at + 36 + size
sys.exit(0 if len(a) == len(b) and a[:at] == b[:at] and a[end:] == b[end:] else 1)
PY
}

# a synthetic image: 64 KiB random, the slot (room 512) at 0x1234, 4 KiB random
mk() {   # mk <out> <copies of the slot>
    python3 -I - "$1" "$2" <<'PY'
import os, struct, sys
slot = b"yakogl-deqp-case-slot/1\0" + struct.pack("<IiI", 512, -1, 0) + bytes(512)
img = bytearray(os.urandom(0x1234)) + slot
for _ in range(int(sys.argv[2]) - 1):
    img += os.urandom(100) + slot
img += os.urandom(4096)
open(sys.argv[1], "wb").write(img)
PY
}
mk "$D/img.bin" 1
cp "$D/img.bin" "$D/orig.bin"
printf '# three rows\ndEQP-GLES2.info.*\ndEQP-GLES2.functional.prerequisite.*\n\ndEQP-GLES2.functional.color_clear.*\n' >"$D/tree.txt"

echo "synthetic image"
out=$(tool show "$D/img.bin"); rc=$?
chk "unwritten: exit 0" test $rc -eq 0
chk "unwritten: offset" grep -qx 'mp2-deqp-slot: slot at 0x1234, room 511 bytes' <<<"$out"
chk "unwritten: runs DEQP_CASE" grep -q 'runs CONFIG_DEQP_CASE (slot not written)' <<<"$out"

out=$(tool write "$D/img.bin" --selector 2 --table "$D/tree.txt"); rc=$?
chk "selector: exit 0" test $rc -eq 0
chk "selector: names the row" grep -qx "mp2-deqp-slot: row 2 of $D/tree.txt: dEQP-GLES2.functional.color_clear.\*" <<<"$out"
out=$(tool show "$D/img.bin")
chk "selector: read back" grep -qx 'mp2-deqp-slot: selector 2' <<<"$out"
chk "selector: runs entry 2" grep -q 'runs DEQP_CASE_TABLE entry 2' <<<"$out"
chk "selector: only the slot changed" outside "$D/orig.bin" "$D/img.bin"
chk "selector: bytes at 28" test "$(od -An -tx1 -j $((0x1234 + 28)) -N 8 "$D/img.bin" | tr -d ' \n')" = 0200000000000000

out=$(tool write "$D/img.bin" --selector 3 --table "$D/tree.txt"); rc=$?
chk "selector past the table: exit 1" test $rc -eq 1
chk "selector past the table: named" grep -q "selector 3 is past" <<<"$out"
chk "selector past the table: slot kept" grep -qx 'mp2-deqp-slot: selector 2' <<<"$(tool show "$D/img.bin")"

F='dEQP-GLES2.functional.shaders.linkage.*,dEQP-GLES2.functional.fbo.render.color_clear.rbo_rgb565'
out=$(tool write "$D/img.bin" --filter "$F" -o "$D/out.bin"); rc=$?
chk "filter -o: exit 0" test $rc -eq 0
chk "filter -o: input untouched" grep -qx 'mp2-deqp-slot: selector 2' <<<"$(tool show "$D/img.bin")"
out=$(tool show "$D/out.bin")
chk "filter: read back, 2 patterns" grep -qx "mp2-deqp-slot: filter ${#F} bytes, 2 pattern(s)" <<<"$out"
chk "filter: selector cleared" grep -qx 'mp2-deqp-slot: selector -1' <<<"$out"
chk "filter: second pattern" grep -qx '  dEQP-GLES2.functional.fbo.render.color_clear.rbo_rgb565' <<<"$out"
chk "filter: runs the filter" grep -q 'the image runs the filter' <<<"$out"
chk "filter: bytes in the image" grep -aqF "$F" "$D/out.bin"
chk "filter: only the slot changed" outside "$D/orig.bin" "$D/out.bin"

printf '# P9 picks\ndEQP-GLES2.functional.a.b   # why\n\ndEQP-GLES2.functional.c.*\n' >"$D/list.txt"
out=$(tool write "$D/out.bin" --case-list "$D/list.txt"); rc=$?
chk "case list: exit 0" test $rc -eq 0
chk "case list: joined" grep -qx '  dEQP-GLES2.functional.c.\*' <<<"$(tool show "$D/out.bin")"
chk "case list: length" grep -qx 'mp2-deqp-slot: filter 51 bytes, 2 pattern(s)' <<<"$(tool show "$D/out.bin")"

long=$(python3 -c 'print(",".join("dEQP-GLES2.functional.x%03d" % i for i in range(40)))')
out=$(tool write "$D/out.bin" --filter "$long"); rc=$?
chk "too long: exit 1" test $rc -eq 1
chk "too long: names the room" grep -q 'the slot holds 511' <<<"$out"
out=$(tool write "$D/out.bin" --filter 'dEQP-GLES2.a, dEQP-GLES2.b'); rc=$?
chk "whitespace pattern: exit 1" test $rc -eq 1
out=$(tool write "$D/out.bin" --filter 'dEQP-GLES2.a,,dEQP-GLES2.b'); rc=$?
chk "empty pattern: exit 1" test $rc -eq 1
out=$(tool write "$D/out.bin" --filter 'GLES2.a'); rc=$?
chk "not dEQP-: exit 1" test $rc -eq 1
chk "refusals: slot kept" grep -qx 'mp2-deqp-slot: filter 51 bytes, 2 pattern(s)' <<<"$(tool show "$D/out.bin")"

out=$(tool write "$D/out.bin" --clear); rc=$?
chk "clear: exit 0" test $rc -eq 0
chk "clear: same bytes as the unwritten image" cmp -s "$D/orig.bin" "$D/out.bin"

head -c 4096 /dev/urandom >"$D/none.bin"
out=$(tool show "$D/none.bin"); rc=$?
chk "no slot: exit 1" test $rc -eq 1
chk "no slot: named" grep -q 'no case slot in the image' <<<"$out"
mk "$D/two.bin" 2
out=$(tool write "$D/two.bin" --selector 0); rc=$?
chk "two slots: exit 1" test $rc -eq 1
chk "two slots: named" grep -q 'more than once' <<<"$out"
out=$(tool show "$D/missing.bin"); rc=$?
chk "missing file: exit 2" test $rc -eq 2

if [ -n "$REAL" ]; then
    echo "real image: $REAL"
    cp "$REAL" "$D/real.bin"
    out=$(tool show "$D/real.bin"); rc=$?
    printf '%s\n' "$out" | sed 's/^/    /'
    chk "real: slot found" test $rc -eq 0
    chk "real: unwritten as built" grep -q 'runs CONFIG_DEQP_CASE (slot not written)' <<<"$out"
    out=$(tool write "$D/real.bin" --selector 5 -o "$D/real5.bin"); rc=$?
    printf '%s\n' "$out" | sed 's/^/    /'
    chk "real: selector written" test $rc -eq 0
    chk "real: selector read back" grep -qx 'mp2-deqp-slot: selector 5' <<<"$(tool show "$D/real5.bin")"
    chk "real: only the slot changed" outside "$D/real.bin" "$D/real5.bin"
    out=$(tool write "$D/real.bin" --filter 'dEQP-GLES2.functional.prerequisite.*,dEQP-GLES2.functional.color_clear.single_rgb' -o "$D/realf.bin"); rc=$?
    printf '%s\n' "$out" | sed 's/^/    /'
    chk "real: filter written" test $rc -eq 0
    chk "real: filter read back" grep -qx '  dEQP-GLES2.functional.color_clear.single_rgb' <<<"$(tool show "$D/realf.bin")"
    chk "real: only the slot changed (filter)" outside "$D/real.bin" "$D/realf.bin"
fi

echo "mp2_deqp_slot_selftest: $pass ok, $fail failed"
[ "$fail" -eq 0 ]

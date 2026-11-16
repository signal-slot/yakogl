#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_uimg_check_selftest.sh - tools/mp2s_uimg_check.py without a board and
# without mkimage (plan #362, item uimg-check).
#
#   tools/mp2s_uimg_check_selftest.sh
#
# The accepted cases come from the real builders: driver/tools/mp2-standalone/
# build.sh (MP2_NO_MKIMAGE=1, so tools/uboot_script.py) and uboot_script.py
# build directly, of the committed mp2-standalone.cmd and of synthetic
# scripts. The refused cases are the same bytes with one property broken: a
# header field mutated with the CRCs recomputed, a bit flipped inside the
# header or the data, a payload that differs from the audited source, a raw
# two-file or empty length table, a truncated file, a trailing byte. Every
# verdict pins the streams: exit 0 keeps exactly five "ok:" lines on stdout
# and an empty stderr; exit 2 puts the single "refused: <why>" line on
# stderr with the pinned wording, and stdout never says "refused". Usage
# errors exit 2 with empty stdout and no "refused:" line.
#
# A tools/check.sh row for this selftest waits for plan #341, so it is run
# by hand: tools/mp2s_uimg_check_selftest.sh
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
T=$(mktemp -d "${TMPDIR:-$here}/mp2s-uimg-check-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT
pass=0
fail=0
WANT_CHECKS=25
CHECK="$here/mp2s_uimg_check.py"
CMD="$here/../driver/tools/mp2-standalone/mp2-standalone.cmd"

report() { echo "FAIL $1"; sed 's/^/  out| /' "$T/out"; sed 's/^/  err| /' "$T/err"; fail=$((fail + 1)); }

# uimg OUT SRC [OPTS...]: the legacy script image of SRC with a pinned time
# and name, built by tools/uboot_script.py (the no-mkimage builder itself;
# later OPTS override, as argparse takes the last).
uimg() {
	local out=$1 src=$2
	shift 2
	python3 -I "$here/uboot_script.py" build "$src" -o "$out" \
		--name mp2s-selftest --time 1762387200 "$@" >"$T/build.out" 2>"$T/build.err"
}

# uimg_data OUT HEX: a legacy script image whose data part is the literal
# bytes HEX (its own CRCs, name empty) - for length tables no builder makes.
uimg_data() {
	python3 -I - "$@" <<'PY'
import binascii, struct, sys, zlib
data = binascii.unhexlify(sys.argv[2])
hdr = [0x27051956, 0, 1762387200, len(data), 0, 0, zlib.crc32(data), 5, 22, 6, 0, b""]
hdr[1] = zlib.crc32(struct.pack(">7I4B32s", *hdr))
open(sys.argv[1], "wb").write(struct.pack(">7I4B32s", *hdr) + data)
PY
}

# mutate IN OUT FIELD=VALUE...: rewrite IN's header fields with the CRCs
# recomputed (field: type, comp, arch, os).
mutate() {
	python3 -I - "$@" <<'PY'
import sys, zlib
path, out = sys.argv[1], sys.argv[2]
raw = open(path, "rb").read()
hdr, body = bytearray(raw[:64]), raw[64:]
for a in sys.argv[3:]:
    k, v = a.split("=", 1)
    hdr[{"type": 30, "comp": 31, "arch": 29, "os": 28}[k]] = int(v, 0)
hdr[24:28] = zlib.crc32(body).to_bytes(4, "big")
hdr[4:8] = b"\0\0\0\0"
hdr[4:8] = zlib.crc32(bytes(hdr)).to_bytes(4, "big")
open(out, "wb").write(bytes(hdr) + body)
PY
}

# flip IN OUT OFFSET: XOR one byte of IN into OUT.
flip() {
	python3 -I - "$@" <<'PY'
import sys
raw = bytearray(open(sys.argv[1], "rb").read())
raw[int(sys.argv[3])] ^= 0x01
open(sys.argv[2], "wb").write(bytes(raw))
PY
}

# expect_ok DESC ARGS...  (exit 0, exactly five "ok:" lines, stderr empty)
expect_ok() {
	local what=$1
	shift
	if python3 -I "$CHECK" "$@" >"$T/out" 2>"$T/err"; then
		local got_ok
		got_ok=$(grep -c '^ok: ' "$T/out" || true)
		if [ "$got_ok" = 5 ] && ! grep -q 'refused\|skip' "$T/out" && [ ! -s "$T/err" ]; then
			pass=$((pass + 1))
			return
		fi
		report "$what (want 5 ok lines and empty stderr)"
	else
		report "$what (exit $?)"
	fi
}

# expect_refuse DESC NEEDLE ARGS...
expect_refuse() {
	local what=$1 needle=$2
	shift 2
	local rc=0
	python3 -I "$CHECK" "$@" >"$T/out" 2>"$T/err" || rc=$?
	if [ "$rc" = 0 ]; then
		report "$what (accepted)"
		return
	fi
	local n
	n=$(grep -c '^refused: ' "$T/err" || true)
	if [ "$rc" = 2 ] && [ "$n" = 1 ] && grep -qF "$needle" "$T/err" && ! grep -q 'refused' "$T/out"; then
		pass=$((pass + 1))
	else
		report "$what (want one 'refused:' naming '$needle' on stderr)"
	fi
}

# expect_usage DESC ARGS...  (exit 2, empty stdout, no "refused:" line)
expect_usage() {
	local what=$1
	shift
	local rc=0
	python3 -I "$CHECK" "$@" >"$T/out" 2>"$T/err" || rc=$?
	if [ "$rc" = 2 ] && [ ! -s "$T/out" ] && ! grep -q 'refused' "$T/err"; then
		pass=$((pass + 1))
	else
		report "$what (want exit 2, empty stdout, no refused line)"
	fi
}

# ---- the accepted cases ------------------------------------------------------------
uimg "$T/committed.uimg" "$CMD"
expect_ok "the committed script, built by tools/uboot_script.py, checks" "$T/committed.uimg"
expect_ok "the same with --source naming the committed script explicitly" \
	"$T/committed.uimg" --source "$CMD"

printf 'echo "mp2s: synthetic"\nexit\n' >"$T/syn.cmd"
uimg "$T/syn.uimg" "$T/syn.cmd"
expect_ok "a synthetic script passes against its own --source" \
	"$T/syn.uimg" --source "$T/syn.cmd"

uimg "$T/other.uimg" "$CMD" --name other-name --time 1
expect_ok "name and time are not pinned" "$T/other.uimg"

MP2_NO_MKIMAGE=1 "$here/../driver/tools/mp2-standalone/build.sh" "$T/stage" >"$T/stage.out"
expect_ok "what driver/tools/mp2-standalone/build.sh builds, checks (the runner stages that)" \
	"$T/stage/boot.scr.uimg"

# ---- the refused cases -------------------------------------------------------------
printf 'x%.0s' {1..134} >"$T/garbage.uimg"
expect_refuse "garbage refused: magic" "magic" "$T/garbage.uimg"

head -c 20 "$T/committed.uimg" >"$T/short.uimg"
expect_refuse "20 bytes refused" "shorter than the 64-byte header" "$T/short.uimg"

flip "$T/committed.uimg" "$T/hcrc.uimg" 40
expect_refuse "header bit flipped (a name byte) refused" "header CRC" "$T/hcrc.uimg"

flip "$T/committed.uimg" "$T/dcrc.uimg" $(( $(stat -c %s "$T/committed.uimg") - 1 ))
expect_refuse "data bit flipped refused" "data CRC" "$T/dcrc.uimg"

head -c $(( $(stat -c %s "$T/committed.uimg") - 1 )) "$T/committed.uimg" >"$T/trunc.uimg"
expect_refuse "one byte truncated refused" "(truncated)" "$T/trunc.uimg"

mutate "$T/committed.uimg" "$T/type2.uimg" type=2
expect_refuse "type kernel refused" "not 6 (script)" "$T/type2.uimg"
mutate "$T/committed.uimg" "$T/comp1.uimg" comp=1
expect_refuse "gzip refused" "compression" "$T/comp1.uimg"
mutate "$T/committed.uimg" "$T/arch2.uimg" arch=2
expect_refuse "arch arm refused" "expected 22 (arm64)" "$T/arch2.uimg"
mutate "$T/committed.uimg" "$T/os0.uimg" os=0
expect_refuse "os none refused" "expected 5 (linux)" "$T/os0.uimg"

# lengths 3,2,end, then "abc", one pad byte, "de"
uimg_data "$T/twofile.uimg" 000000030000000200000000616263006465
expect_refuse "two files in the script data refused" "script data holds 2 files" "$T/twofile.uimg"

uimg_data "$T/empty.uimg" 0000000000000000
expect_refuse "an image of an empty script refused" "length table is empty" "$T/empty.uimg"

sed 's/mp2s: undo/mp2s: UNDO/' "$CMD" >"$T/edited.cmd"
uimg "$T/edited.uimg" "$T/edited.cmd"
expect_refuse "a payload that differs from the committed source refused" \
	"differs from the audited source" "$T/edited.uimg"

cat "$T/committed.uimg" >"$T/trail.uimg"
printf 'x' >>"$T/trail.uimg"
expect_refuse "a byte after the image refused" "after the end of the image" "$T/trail.uimg"

expect_refuse "missing image refused" "not a readable file" "$T/none.uimg"
expect_refuse "a directory refused" "not a readable file" "$T"
expect_refuse "missing --source refused" "script source" \
	"$T/committed.uimg" --source "$T/none.cmd"

# ---- usage -------------------------------------------------------------------------
expect_usage "no arguments"
expect_usage "unknown flag" "$T/committed.uimg" --nope
expect_usage "two images" "$T/committed.uimg" "$T/committed.uimg"
expect_usage "empty --source" "$T/committed.uimg" --source ""

echo "mp2s_uimg_check_selftest: $pass checks, $fail failed (pinned: $WANT_CHECKS)"
[ "$fail" = 0 ] && [ "$pass" = "$WANT_CHECKS" ]

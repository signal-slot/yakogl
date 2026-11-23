#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_build_repro_selftest.sh: driver/tools/mp2-standalone/build.sh builds a
# reproducible boot.scr.uimg that tools/mp2s_uimg_check.py accepts. No board,
# no network. Cases: (1) two builds under different TZ, LC_ALL, umask and cwd
# are byte-identical and the printed sha256 is the file's; (2) the check
# accepts the image against mp2-standalone.cmd; (3) another SOURCE_DATE_EPOCH
# changes only the header time and header CRC, and the image is still
# accepted; (4) the check refuses a payload with one byte flipped.
# Prints "mp2s_build_repro_selftest: N checks, F failed"; non-zero on failure.
set -u

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
build="$root/driver/tools/mp2-standalone/build.sh"
src="$root/driver/tools/mp2-standalone/mp2-standalone.cmd"
chk="$root/tools/mp2s_uimg_check.py"

tmp=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_repro.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT

n=0
f=0
ok() { n=$((n + 1)); echo "ok   $*"; }
bad() { n=$((n + 1)); f=$((f + 1)); echo "FAIL $*"; }
check() { # desc, then command
	local d=$1
	shift
	if "$@" > /dev/null 2>&1; then ok "$d"; else bad "$d"; fi
}

mkdir "$tmp/cwd1" "$tmp/cwd2"
out1=$(cd "$tmp/cwd1" && umask 022 && TZ=UTC LC_ALL=C SOURCE_DATE_EPOCH=1700000000 \
	bash "$build" "$tmp/a" 2>&1 | tail -n 1)
out2=$(cd "$tmp/cwd2" && umask 077 && TZ=Asia/Tokyo LC_ALL=C.UTF-8 SOURCE_DATE_EPOCH=1700000000 \
	bash "$build" "$tmp/b/c" 2>&1 | tail -n 1)
a="$tmp/a/boot.scr.uimg"
b="$tmp/b/c/boot.scr.uimg"

# Without SOURCE_DATE_EPOCH too (commit time of the script, or 0).
out3=$(cd "$tmp/cwd1" && env -u SOURCE_DATE_EPOCH TZ=UTC bash "$build" "$tmp/d" 2>&1 | tail -n 1)
out4=$(cd "$tmp/cwd2" && umask 077 && env -u SOURCE_DATE_EPOCH TZ=Asia/Tokyo LC_ALL=C.UTF-8 \
	bash "$build" "$tmp/e" 2>&1 | tail -n 1)

case "$out1" in
*"(via mkimage)") via=mkimage ;;
*"(via uboot_script.py)") via=python ;;
*) via=unknown ;;
esac
echo "build path used: $via"
if [ "$via" = unknown ]; then bad "build.sh last line unrecognised: $out1"; fi

# (1)
if [ -f "$a" ] && [ -f "$b" ] && cmp -s "$a" "$b"; then ok "(1) two builds are byte-identical"; else bad "(1) two builds differ"; fi
if [ -f "$tmp/d/boot.scr.uimg" ] && cmp -s "$tmp/d/boot.scr.uimg" "$tmp/e/boot.scr.uimg"; then
	ok "(1) identical without SOURCE_DATE_EPOCH"
else bad "(1) differ without SOURCE_DATE_EPOCH"; fi
for pair in "$out1|$a" "$out2|$b" "$out3|$tmp/d/boot.scr.uimg" "$out4|$tmp/e/boot.scr.uimg"; do
	line=${pair%%|*}
	file=${pair#*|}
	want=$(sha256sum "$file" 2> /dev/null | cut -d' ' -f1)
	got=$(printf '%s\n' "$line" | sed -n 's/.* sha256 \([0-9a-f]\{64\}\) .*/\1/p')
	if [ -n "$want" ] && [ "$want" = "$got" ]; then ok "(1) printed sha256 equals sha256sum"; else bad "(1) printed sha256 '$got' != '$want'"; fi
done

# (2)
check "(2) uimg check accepts the image" python3 -I "$chk" --source "$src" "$a"

# (3)
SOURCE_DATE_EPOCH=1800000000 bash "$build" "$tmp/f" > /dev/null 2>&1
fimg="$tmp/f/boot.scr.uimg"
python3 -I - "$a" "$fimg" << 'PY'
import sys
x = open(sys.argv[1], 'rb').read()
y = open(sys.argv[2], 'rb').read()
if len(x) != len(y) or x[64:] != y[64:]:
    sys.exit(1)
diff = {i for i in range(64) if x[i] != y[i]}
# header: magic 0-3, hcrc 4-7, time 8-11
ok = diff and diff <= set(range(4, 12)) and diff & set(range(8, 12)) and diff & set(range(4, 8))
sys.exit(0 if ok else 1)
PY
rc=$?
if [ $rc -eq 0 ]; then ok "(3) only header time and CRC differ"; else bad "(3) images differ beyond header time/CRC"; fi
check "(3) uimg check accepts the other-epoch image" python3 -I "$chk" --source "$src" "$fimg"

# (4)
python3 -I - "$a" "$tmp/flip.uimg" << 'PY'
import sys
x = bytearray(open(sys.argv[1], 'rb').read())
x[-5] ^= 0x01
open(sys.argv[2], 'wb').write(x)
PY
if python3 -I "$chk" --source "$src" "$tmp/flip.uimg" > /dev/null 2>&1; then
	bad "(4) uimg check accepted a flipped payload byte"
else ok "(4) uimg check refuses a flipped payload byte"; fi

echo "mp2s_build_repro_selftest: $n checks, $f failed"
[ "$f" -eq 0 ]

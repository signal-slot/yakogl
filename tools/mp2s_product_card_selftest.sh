#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host selftest of tools/mp2s_product_card.py: no board, no device.
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/mp2s_product_card.py
t=$(mktemp -d "${TMPDIR:-/tmp}/mp2spc.XXXXXX") || exit 1
trap 'rm -rf "$t"' EXIT
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }

cat > "$t/env.txt" <<'E'
# synthetic, stock-like
baudrate=115200
bootcmd=run distro_bootcmd; run bootcmd_stm32mp
altbootcmd=run bootcmd_stm32mp
bootcount=0
bootlimit=3
boot_prefixes=/mmc0_
boot_targets=mmc1 mmc0
E
printf 'script' > "$t/boot.scr.uimg"
printf 'image' > "$t/default.bin"
run() { python3 -I "$tool" "$@"; }

if run --env-text "$t/env.txt" --script "$t/boot.scr.uimg" --image "$t/default.bin" \
      --env-size 4096 --no-linux --out "$t/o" >/dev/null; then ok build; else bad build; fi

# parse the result back with an independent reader
if python3 -I - "$t" <<'P'
import sys, struct, zlib, hashlib
t = sys.argv[1]
def rd(p):
    r = open(p, "rb").read()
    assert len(r) == 4096, "size"
    assert struct.unpack("<I", r[:4])[0] == zlib.crc32(r[5:]) & 0xffffffff, "crc"
    d = r[5:]
    assert d.rstrip(b"\0") == d[:d.index(b"\0\0")], "padding"
    kv = dict(e.split(b"=", 1) for e in d[:d.index(b"\0\0")].split(b"\0"))
    return r[4], kv
f1, a = rd(t + "/o/u-boot-env-copy1.bin")
f2, b = rd(t + "/o/u-boot-env-copy2.bin")
assert a == b and f1 == 1 and f2 == 0, "copies/flags"
assert a[b"boot_prefixes"] == b"/yakogl/standalone/ /mmc0_"
assert a[b"mp2s_prefixes"] == b"/mmc0_"
assert a[b"mp2s_left"] == b"forever" and a[b"mp2s_ver"] == b"1"
n_same = 0
for l in open(t + "/env.txt", "rb").read().split(b"\n"):
    if l.startswith((b"bootcmd=", b"bootcount=", b"altbootcmd=", b"bootlimit=")):
        n, v = l.split(b"=", 1)
        assert a[n] == v, n
        n_same += 1
assert n_same == 4
assert open(t + "/o/yakogl/standalone/boot.scr.uimg", "rb").read() == b"script"
assert open(t + "/o/yakogl/standalone/default.bin", "rb").read() == b"image"
m = open(t + "/o/manifest.txt").read()
for rel in ("u-boot-env-copy1.bin", "u-boot-env-copy2.bin",
            "yakogl/standalone/boot.scr.uimg", "yakogl/standalone/default.bin"):
    h = hashlib.sha256(open(t + "/o/" + rel, "rb").read()).hexdigest()
    assert h + "  " + rel in m, rel
assert "mmc0_extlinux" in m
P
then ok "crcs, variables, untouched bootcmd/bootcount, manifest"; else bad "readback"; fi

# the repository's own env reader agrees: both copies valid, mp2s_left=forever
cat "$t/o/u-boot-env-copy1.bin" "$t/o/u-boot-env-copy2.bin" > "$t/pair.bin"
ue=$here/uboot_env.py
if python3 -I "$ue" --redundant --size 4096 check "$t/pair.bin" >/dev/null &&
   [ "$(python3 -I "$ue" --redundant --size 4096 print "$t/pair.bin" -n mp2s_left)" = forever ]; then
  ok "uboot_env.py reads the pair"; else bad "uboot_env.py readback"; fi

# an env image carrying a default is refused; a plain env image works
if run --env-image "$t/o/u-boot-env-copy1.bin" --script "$t/boot.scr.uimg" \
      --image "$t/default.bin" --env-size 4096 --out "$t/o2" >/dev/null 2>&1; then
  bad "image input already carrying a default accepted"; else ok "default already set refused"; fi
python3 -I - "$t" <<'P'
import struct, zlib, sys
t = sys.argv[1]
d = b"".join(l.encode() + b"\0" for l in open(t + "/env.txt").read().splitlines()
             if l and l[0] != "#") + b"\0"
d += b"\0" * (4096 - 5 - len(d))
open(t + "/in.bin", "wb").write(struct.pack("<I", zlib.crc32(d)) + b"\1" + d)
P
if run --env-image "$t/in.bin" --script "$t/boot.scr.uimg" --image "$t/default.bin" \
      --env-size 4096 --out "$t/o3" >/dev/null &&
   cmp -s "$t/o/u-boot-env-copy1.bin" "$t/o3/u-boot-env-copy1.bin"; then
  ok "env image input == env text input"; else bad "env image input"; fi

expect_refuse() {
  local n=$1; shift
  if run "$@" >/dev/null 2>&1; then bad "$n accepted"; else ok "$n refused"; fi
}
base=(--env-text "$t/env.txt" --script "$t/boot.scr.uimg" --image "$t/default.bin" --env-size 4096)
expect_refuse "out under /dev" "${base[@]}" --out /dev/mp2s-card-x
expect_refuse "out /dev/shm" "${base[@]}" --out /dev/shm/mp2s-card-x
expect_refuse "out under /proc" "${base[@]}" --out /proc/mp2s-x
expect_refuse "out under /sys" "${base[@]}" --out /sys/mp2s-x
expect_refuse "input under /dev" --env-text /dev/null --script "$t/boot.scr.uimg" \
    --image "$t/default.bin" --out "$t/r1"
expect_refuse "image is a char device" --env-text "$t/env.txt" --script "$t/boot.scr.uimg" \
    --image /dev/zero --out "$t/r2"
ln -s /dev/null "$t/lnk"
expect_refuse "symlink to a device" --env-text "$t/env.txt" --script "$t/lnk" \
    --image "$t/default.bin" --out "$t/r3"
expect_refuse "set bootcmd" "${base[@]}" --set bootcmd=x --out "$t/r4"
expect_refuse "set bootcount" "${base[@]}" --set bootcount=0 --out "$t/r5"
expect_refuse "set preboot" "${base[@]}" --set preboot=x --out "$t/r6"
expect_refuse "set upgrade_available" "${base[@]}" --set upgrade_available=1 --out "$t/r7"
expect_refuse "set non-mp2s" "${base[@]}" --set foo=1 --out "$t/r8"
expect_refuse "non-empty out" "${base[@]}" --out "$t/o"
expect_refuse "env too small" "${base[@]}" --env-size 64 --out "$t/r9"
grep -v '^boot_prefixes' "$t/env.txt" > "$t/noprefix.txt"
expect_refuse "no boot_prefixes" --env-text "$t/noprefix.txt" --script "$t/boot.scr.uimg" \
    --image "$t/default.bin" --env-size 4096 --out "$t/r10"
head -c 20 "$t/in.bin" > "$t/short.bin"
expect_refuse "bad env image" --env-image "$t/short.bin" --script "$t/boot.scr.uimg" \
    --image "$t/default.bin" --env-size 4096 --out "$t/r11"
for r in r1 r2 r3 r4 r5 r6 r7 r8 r9 r10 r11; do
  [ -e "$t/$r" ] && bad "refusal $r left output"
done
if [ ! -e /dev/mp2s-card-x ] && [ ! -e /dev/shm/mp2s-card-x ]; then ok "nothing written under /dev"; fi

if [ $fail = 0 ]; then echo "mp2s_product_card selftest: ok"; exit 0; fi
echo "mp2s_product_card selftest: FAILED"; exit 1

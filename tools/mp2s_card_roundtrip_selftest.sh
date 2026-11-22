#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Selftest: a 'forever' product card built by mp2s_product_card.py and undone
# by mp2s_card_undo.sh is back to ST's stock boot. A sparse GPT disk image of
# the DK's microSD shape under a temp dir (no root, no loop device, no real
# device): fsbl/metadata/fip partitions with fixed bytes, u-boot-env with the
# pristine env (copy 1 at 0x7e000, copy 2 at 0x7c000), and an ext4 bootfs.
# card_undo refuses a whole disk, so the bootfs partition is cut out into its
# own file, undone there and written back, as `dd` would on a real card.
#
# What the undo does and does not do (note section 5): it renames
# yakogl/standalone/boot.scr.uimg to boot.scr.uimg.off and nothing else. The
# env copies and default.bin are left for `nodefault`; they are checked as
# such (the stock boot needs only the rename: the prefix then finds no
# script), and the env edit of `nodefault` is applied with uboot_env.py to
# show that it takes both copies back to the pristine variables.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
export PATH=$PATH:/sbin:/usr/sbin
for c in mke2fs debugfs blkid dumpe2fs sgdisk python3 sha256sum dd truncate; do
  command -v "$c" >/dev/null 2>&1 || { echo "SKIP: $c is missing; mp2s_card_roundtrip_selftest not run"; exit 0; }
done
W=$(mktemp -d "${TMPDIR:-$HERE}/mp2s_card_rt.XXXXXX") || exit 2
trap 'rm -rf "$W"' EXIT
PY="python3 -I"
ENV=$HERE/testdata/mp2s-env/pristine.bin
fail=0 n=0
ok()  { n=$((n+1)); echo "ok   $*"; }
bad() { n=$((n+1)); fail=$((fail+1)); echo "FAIL $*"; }
sha() { sha256sum < "$1" | cut -d' ' -f1; }

# ---- the stock bootfs (as card_undo's selftest builds it)
mkdir -p "$W/stock/mmc0_extlinux" "$W/stock/mmc1_extlinux"
echo "stock script" > "$W/stock/boot.scr.uimg"
echo "label stock0" > "$W/stock/mmc0_extlinux/stm32mp257f-dk_extlinux.conf"
echo "label stock1" > "$W/stock/mmc1_extlinux/stm32mp257f-dk_extlinux.conf"
head -c 70000 /dev/zero | tr '\0' 'k' > "$W/stock/Image.gz"
mke2fs -q -t ext4 -d "$W/stock" "$W/bootfs.img" 8M >/dev/null 2>&1 || { echo "mke2fs failed"; exit 2; }
STOCK="/boot.scr.uimg /Image.gz /mmc0_extlinux/stm32mp257f-dk_extlinux.conf /mmc1_extlinux/stm32mp257f-dk_extlinux.conf"
dumpf() { if debugfs -R "dump $1 $W/dump" "$2" >/dev/null 2>&1; then sha "$W/dump"; else echo MISSING; fi; }
has() { debugfs -R "stat $1" "$2" 2>/dev/null | grep -q 'Inode:'; }
declare -A S0
for f in $STOCK; do S0[$f]=$(dumpf "$f" "$W/bootfs.img"); done

# ---- the disk: GPT, DK shape (512-byte sectors)
D=$W/card.img
truncate -s 64M "$D"
sgdisk -Z "$D" >/dev/null 2>&1
i=0
part() { i=$((i+1)); sgdisk -a 1 -n "$i:$2:$3" -c "$i:$1" "$D" >/dev/null 2>&1; }
part fsbl1 2048 2303; part fsbl2 2304 2559; part metadata1 2560 2623; part metadata2 2624 2687
part fip-a 2688 6783; part fip-b 6784 10879; part u-boot-env 10880 11903; part bootfs 11904 28287
start() { sgdisk -i "$1" "$D" | awk '/First sector/ {print $3}'; }
size()  { sgdisk -i "$1" "$D" | awk '/Partition size/ {print $3}'; }
[ "$(sgdisk -p "$D" 2>/dev/null | grep -c '^ *[0-9]')" = 8 ] || { echo "FAIL: sgdisk did not lay out the 8 partitions"; exit 1; }
pdd_in()  { dd if="$2" of="$D" bs=512 seek="$(start "$1")" conv=notrunc status=none; }
pdd_out() { dd if="$D" of="$2" bs=512 skip="$(start "$1")" count="$(size "$1")" status=none; }
for p in 1 2 3 4 5 6; do head -c $(( $(size $p) * 512 )) /dev/urandom > "$W/p$p.bin"; pdd_in $p "$W/p$p.bin"; done
# u-boot-env: copy 1 at 0x7e000, copy 2 at 0x7c000 (pristine.bin has them at 0 and 0x2000)
head -c $((0x80000)) /dev/zero > "$W/envpart.bin"
dd if="$ENV" of="$W/envpart.bin" bs=1 count=$((0x2000)) seek=$((0x7e000)) conv=notrunc status=none
dd if="$ENV" of="$W/envpart.bin" bs=1 skip=$((0x2000)) count=$((0x2000)) seek=$((0x7c000)) conv=notrunc status=none
pdd_in 7 "$W/envpart.bin"
pdd_in 8 "$W/bootfs.img"

# ---- apply the product card's outputs onto the stock card
head -c 20000 /dev/urandom > "$W/boot.scr.uimg"; head -c 30000 /dev/urandom > "$W/default.bin"
head -c 8192 "$ENV" > "$W/env1.bin"
if $PY "$HERE/mp2s_product_card.py" --env-image "$W/env1.bin" --env-size 0x2000 --script "$W/boot.scr.uimg" \
     --image "$W/default.bin" --out "$W/card" >"$W/pc.out" 2>&1; then ok "product card built (forever)"; else bad "product card"; cat "$W/pc.out"; fi
pdd_out 8 "$W/bootfs.img"
printf 'mkdir /yakogl\nmkdir /yakogl/standalone\nwrite %s /yakogl/standalone/boot.scr.uimg\nwrite %s /yakogl/standalone/default.bin\n' \
  "$W/card/yakogl/standalone/boot.scr.uimg" "$W/card/yakogl/standalone/default.bin" | debugfs -w -f - "$W/bootfs.img" >/dev/null 2>&1
pdd_in 8 "$W/bootfs.img"
dd if="$W/card/u-boot-env-copy1.bin" of="$D" bs=512 seek=$(( $(start 7) + 0x7e000/512 )) conv=notrunc status=none
dd if="$W/card/u-boot-env-copy2.bin" of="$D" bs=512 seek=$(( $(start 7) + 0x7c000/512 )) conv=notrunc status=none
if has /yakogl/standalone/boot.scr.uimg "$W/bootfs.img" && has /yakogl/standalone/default.bin "$W/bootfs.img"; then ok "product files staged on the card"; else bad "staging failed"; fi

# one copy's variables (CRC checked), sorted; a reader independent of uboot_env.py
envvars() { # <file> <offset>
  $PY -c '
import sys, struct, zlib
r = open(sys.argv[1], "rb").read()[int(sys.argv[2]):][:0x2000]
if struct.unpack("<I", r[:4])[0] != zlib.crc32(r[5:]) & 0xffffffff:
    sys.exit(1)
d = r[5:]
for e in d[:d.index(bytes(2))].split(bytes(1)):
    print(e.decode("latin1"))
' "$1" "$2" | LC_ALL=C sort
}
pdd_out 7 "$W/env_before.bin"
e1=$(envvars "$W/env_before.bin" $((0x7e000))); e2=$(envvars "$W/env_before.bin" $((0x7c000)))
if [ "$e1" = "$e2" ] && [ -n "$e1" ]; then ok "both env copies agree before the undo"; else bad "env copies differ or unreadable"; fi
if printf '%s\n' "$e1" | grep -qx 'mp2s_left=forever' && printf '%s\n' "$e1" | grep -qx 'boot_prefixes=/yakogl/standalone/ / /boot/'; then
  ok "env carries mp2s_left=forever and our prefix first"; else bad "env of the product card"; fi

for p in 1 2 3 4 5 6; do pdd_out $p "$W/b$p.bin"; done
PB=""; for p in 1 2 3 4 5 6; do PB="$PB $(sha "$W/b$p.bin")"; done

# ---- the undo, on the cut-out bootfs partition
pdd_out 8 "$W/bootfs.img"
bash "$HERE/mp2s_card_undo.sh" --yes "$W/bootfs.img" >"$W/undo.out" 2>"$W/undo.err"; rc=$?
if [ $rc = 0 ]; then ok "card_undo --yes exits 0"; else bad "card_undo rc $rc"; cat "$W/undo.err"; fi
pdd_in 8 "$W/bootfs.img"

# ---- after (read back from the disk image)
bfs=$W/bootfs_after.img
pdd_out 8 "$bfs"
same=1; for f in $STOCK; do [ "$(dumpf "$f" "$bfs")" = "${S0[$f]}" ] || { same=0; echo "  changed: $f"; }; done
if [ $same = 1 ]; then ok "stock bootfs files byte-identical"; else bad "stock bootfs files changed"; fi
if has /yakogl/standalone/boot.scr.uimg "$bfs"; then bad "staged boot.scr.uimg still under its name"; else ok "staged boot.scr.uimg is gone from the prefix"; fi
if [ "$(dumpf /yakogl/standalone/boot.scr.uimg.off "$bfs")" = "$(sha "$W/card/yakogl/standalone/boot.scr.uimg")" ]; then ok "boot.scr.uimg.off holds our script's bytes"; else bad "renamed content"; fi
if [ "$(dumpf /yakogl/standalone/default.bin "$bfs")" = "$(sha "$W/card/yakogl/standalone/default.bin")" ]; then ok "default.bin untouched (left for nodefault)"; else bad "default.bin changed"; fi
if command -v e2fsck >/dev/null 2>&1; then
  e2fsck -fn "$bfs" >/dev/null 2>&1 && ok "e2fsck clean" || bad "e2fsck"
fi

# the undo writes no env byte; the stock-boot-critical variables are the stock ones
pdd_out 7 "$W/env_after.bin"
if cmp -s "$W/env_before.bin" "$W/env_after.bin"; then ok "env partition untouched by the card undo (nodefault's job)"; else bad "env partition changed"; fi
stockenv=$(envvars "$W/env1.bin" 0)
for v in bootcmd bootcount altbootcmd; do
  a=$(printf '%s\n' "$stockenv" | grep "^$v=" || true)
  b=$(envvars "$W/env_after.bin" $((0x7e000)) | grep "^$v=" || true)
  c=$(envvars "$W/env_after.bin" $((0x7c000)) | grep "^$v=" || true)
  if [ "$a" = "$b" ] && [ "$a" = "$c" ]; then ok "$v unchanged in both copies (${a:-absent})"; else bad "$v changed"; fi
done

# nodefault's env edit (the runner's, from Linux) takes each copy back to the pristine variables
pfx=$(printf '%s\n' "$e1" | sed -n 's/^mp2s_prefixes=//p')
mkdir -p "$W/nd"
# uboot_env.py writes the copy that is not active: unset, set, then set again
# (the second set rewrites the copy the unset left without boot_prefixes)
LAY="--redundant --size 0x2000 --offset $((0x7e000)) --offset2 $((0x7c000))"
$PY "$HERE/uboot_env.py" $LAY unset "$W/env_after.bin" "$W/nd/a.bin" mp2s_ver mp2s_left mp2s_prefixes >/dev/null 2>&1
$PY "$HERE/uboot_env.py" $LAY set "$W/nd/a.bin" "$W/nd/b.bin" "boot_prefixes=$pfx" >/dev/null 2>&1
$PY "$HERE/uboot_env.py" $LAY set "$W/nd/b.bin" "$W/nd/c.bin" "boot_prefixes=$pfx" >/dev/null 2>&1
mv "$W/nd/c.bin" "$W/nd/b.bin"
for off in $((0x7e000)) $((0x7c000)); do
  got=$(envvars "$W/nd/b.bin" $off)
  if [ -n "$got" ] && [ "$got" = "$stockenv" ]; then ok "after nodefault's edit the copy at $(printf '0x%x' $off) equals the pristine env"; else bad "nodefault edit, copy at $off"; fi
done

# fsbl / metadata / fip untouched
PA=""; for p in 1 2 3 4 5 6; do pdd_out $p "$W/a$p.bin"; PA="$PA $(sha "$W/a$p.bin")"; done
if [ "$PA" = "$PB" ]; then ok "fsbl/metadata/fip partitions sha256 unchanged"; else bad "fixed partitions changed"; fi
if sgdisk -v "$D" 2>/dev/null | grep -q 'No problems found'; then ok "GPT still intact"; else bad "GPT"; fi

echo "mp2s_card_roundtrip_selftest: $n checks, $fail failed"
[ "$fail" = 0 ]

#!/usr/bin/env bash
# Selftest of tools/mp2s_card_undo.sh. No board, no real device: ext4 images
# built under $TMPDIR with mke2fs -d from fake trees.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/mp2s_card_undo.sh
for c in mke2fs debugfs blkid dumpe2fs; do
  command -v "$c" >/dev/null 2>&1 || PATH=$PATH:/sbin:/usr/sbin
  command -v "$c" >/dev/null 2>&1 || { echo "SKIP: $c is missing (e2fsprogs); mp2s_card_undo selftest not run"; exit 0; }
done
W=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_card_undo_st.XXXXXX") || exit 2
trap 'rm -rf "$W"' EXIT
export PATH

fail=0 n=0
ok()  { n=$((n+1)); echo "ok   $*"; }
bad() { n=$((n+1)); fail=$((fail+1)); echo "FAIL $*"; }
sha() { sha256sum < "$1" | cut -d' ' -f1; }

stock() { # <dir>
  mkdir -p "$1/mmc0_extlinux" "$1/mmc1_extlinux"
  echo "stock script" > "$1/boot.scr.uimg"
  echo "label stock" > "$1/mmc0_extlinux/stm32mp257f-dk_extlinux.conf"
  echo "label stock1" > "$1/mmc1_extlinux/stm32mp257f-dk_extlinux.conf"
  head -c 70000 /dev/zero | tr '\0' 'k' > "$1/Image.gz"
}
ours() {
  mkdir -p "$1/yakogl/standalone"
  echo "our script" > "$1/yakogl/standalone/boot.scr.uimg"
  echo "our image" > "$1/yakogl/standalone/default.bin"
}
mkimg() { mke2fs -q -t ext4 -d "$1" "$2" 8M >/dev/null 2>&1; }

# images
stock "$W/a"; ours "$W/a"; mkimg "$W/a" "$W/good.img"
stock "$W/b"; mkimg "$W/b" "$W/noyako.img"
dd if=/dev/zero of="$W/vfat.img" bs=1M count=8 2>/dev/null
if command -v mkfs.vfat >/dev/null 2>&1; then mkfs.vfat "$W/vfat.img" >/dev/null 2>&1; else printf 'x' > "$W/vfat.img"; fi
stock "$W/c"; ours "$W/c"; rm "$W/c/yakogl/standalone/boot.scr.uimg"; ln -s ../../boot.scr.uimg "$W/c/yakogl/standalone/boot.scr.uimg"; mkimg "$W/c" "$W/link.img"
stock "$W/d"; mkdir -p "$W/d/outside"; echo x > "$W/d/outside/boot.scr.uimg"; ours "$W/d"; rm -r "$W/d/yakogl/standalone"; ln -s ../outside "$W/d/yakogl/standalone"; mkimg "$W/d" "$W/dirlink.img"
mkdir -p "$W/e/yakogl/standalone"; echo y > "$W/e/yakogl/standalone/boot.scr.uimg"; mkimg "$W/e" "$W/nostock.img"
stock "$W/f"; ours "$W/f"; echo off > "$W/f/yakogl/standalone/boot.scr.uimg.off"; mkimg "$W/f" "$W/offexists.img"

run() { bash "$TOOL" "$@" >"$W/out" 2>"$W/err"; echo $? > "$W/rc"; }
rc() { cat "$W/rc"; }
dbg() { debugfs -R "$1" "$2" 2>/dev/null; }
dumpf() { dbg "dump $1 $W/dump" "$2" >/dev/null; sha "$W/dump"; }

# dry run (explicit and default) writes nothing and prints the commands
h0=$(sha "$W/good.img")
run "$W/good.img"
[ "$(rc)" = 0 ] && [ "$(sha "$W/good.img")" = "$h0" ] && grep -q 'DRY RUN' "$W/out" && ok "default is a dry run, image unchanged" || bad "default dry run"
grep -q 'ln /yakogl/standalone/boot.scr.uimg /yakogl/standalone/boot.scr.uimg.off.*unlink /yakogl/standalone/boot.scr.uimg.n' "$W/out" \
  && grep -q 'ln /yakogl/standalone/boot.scr.uimg.off /yakogl/standalone/boot.scr.uimg.*unlink /yakogl/standalone/boot.scr.uimg.off' "$W/out" \
  && ok "prints the write and the undo command lines" || bad "command lines"
run --dry-run "$W/good.img"
[ "$(rc)" = 0 ] && [ "$(sha "$W/good.img")" = "$h0" ] && ok "--dry-run, image unchanged" || bad "--dry-run"

# refusals: each leaves the image byte for byte as it was
for t in noyako vfat link dirlink nostock offexists; do
  h=$(sha "$W/$t.img")
  run --yes "$W/$t.img"
  if [ "$(rc)" = 1 ] && grep -q '^REFUSED' "$W/err" && [ "$(sha "$W/$t.img")" = "$h" ]; then ok "refused $t (--yes), image unchanged"; else bad "refuse $t (rc $(rc))"; cat "$W/err"; fi
done
run "$W/nothere.img"; [ "$(rc)" = 1 ] && ok "refused a missing target" || bad "missing target"
run --yes; [ "$(rc)" = 2 ] && ok "usage without target" || bad "usage"
mkdir "$W/dir"; run "$W/dir"; [ "$(rc)" = 1 ] && ok "refused a directory" || bad "directory"
ln -s "$W/good.img" "$W/lnk.img"; run "$W/lnk.img"; [ "$(rc)" = 1 ] && ok "refused a symlink target" || bad "symlink target"

# a whole disk image (partition table) is refused
if command -v sfdisk >/dev/null 2>&1; then
  cp "$W/good.img" "$W/disk.img"; head -c 1048576 /dev/zero > "$W/pt.img"
  printf 'label: gpt\nstart=2048, size=4096, type=L\n' | sfdisk -q "$W/pt.img" >/dev/null 2>&1
  if [ -n "$(blkid -p -o value -s PTTYPE "$W/pt.img" 2>/dev/null)" ]; then
    run --yes "$W/pt.img"; [ "$(rc)" = 1 ] && grep -q 'partition table' "$W/err" && ok "refused a disk image with a partition table" || bad "partition table"
  fi
fi

# --yes: only that file is renamed, the stock files and the rest stay
s_img=$(dumpf /Image.gz "$W/good.img"); s_boot=$(dumpf /boot.scr.uimg "$W/good.img")
s_e0=$(dumpf /mmc0_extlinux/stm32mp257f-dk_extlinux.conf "$W/good.img"); s_e1=$(dumpf /mmc1_extlinux/stm32mp257f-dk_extlinux.conf "$W/good.img")
s_def=$(dumpf /yakogl/standalone/default.bin "$W/good.img"); s_our=$(dumpf /yakogl/standalone/boot.scr.uimg "$W/good.img")
ls_before=$( { dbg "ls /" "$W/good.img"; dbg "ls /yakogl" "$W/good.img"; dbg "ls /mmc0_extlinux" "$W/good.img"; } | tr -s ' ' | sort)
run --yes "$W/good.img"
if [ "$(rc)" = 0 ]; then ok "--yes exits 0"; else bad "--yes rc $(rc)"; cat "$W/err"; fi
grep -q 'sha256 unchanged' "$W/out" && grep -q 'tools/board-stm32mp2-standalone.sh nodefault' "$W/out" && grep -qi 'put the card back' "$W/out" && ok "prints the read-back, sha256 and the next steps" || bad "output after --yes"
d=$(dbg "ls /yakogl/standalone" "$W/good.img")
case "$d" in *boot.scr.uimg.off*) ok "boot.scr.uimg.off exists";; *) bad "no .off";; esac
[ -z "$(dbg 'stat /yakogl/standalone/boot.scr.uimg' "$W/good.img" | grep -i 'Inode:')" ] && ok "boot.scr.uimg is gone from the prefix" || bad "boot.scr.uimg still there"
[ "$(dumpf /yakogl/standalone/boot.scr.uimg.off "$W/good.img")" = "$s_our" ] && ok "the renamed file is the same bytes" || bad "renamed content"
[ "$(dumpf /Image.gz "$W/good.img")" = "$s_img" ] && [ "$(dumpf /boot.scr.uimg "$W/good.img")" = "$s_boot" ] \
  && [ "$(dumpf /mmc0_extlinux/stm32mp257f-dk_extlinux.conf "$W/good.img")" = "$s_e0" ] \
  && [ "$(dumpf /mmc1_extlinux/stm32mp257f-dk_extlinux.conf "$W/good.img")" = "$s_e1" ] \
  && [ "$(dumpf /yakogl/standalone/default.bin "$W/good.img")" = "$s_def" ] && ok "stock files and default.bin byte-identical" || bad "stock files changed"
ls_after=$( { dbg "ls /" "$W/good.img"; dbg "ls /yakogl" "$W/good.img"; dbg "ls /mmc0_extlinux" "$W/good.img"; } | tr -s ' ' | sort)
[ "$ls_before" = "$ls_after" ] && ok "no other directory entry changed" || bad "other entries changed"
e2fsck -fn "$W/good.img" >/dev/null 2>&1 && ok "e2fsck clean after the write" || { command -v e2fsck >/dev/null 2>&1 && bad "e2fsck" || ok "e2fsck not available (skipped)"; }

# running again: already undone, no write
h=$(sha "$W/good.img"); run --yes "$W/good.img"
[ "$(rc)" = 0 ] && grep -q 'Already undone' "$W/out" && [ "$(sha "$W/good.img")" = "$h" ] && ok "second run: already undone, no write" || bad "second run"

echo "$n checks, $fail failed"
[ "$fail" = 0 ]

#!/usr/bin/env bash
# No-board selftest of tools/mp2s_boot_manifest.py (the /boot byte-for-byte
# comparison of docs/notes/stm32mp257f-dk-standalone.md section 5).
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool=$here/mp2s_boot_manifest.py
t=$(mktemp -d) || exit 2
trap 'rm -rf "$t"' EXIT
pass=0
fail=0
ok() { pass=$((pass + 1)); }
bad() { fail=$((fail + 1)); echo "FAIL: $1"; }
run() { python3 -I "$tool" "$@" >"$t/out" 2>&1; echo $?; }
expect() { # name want-rc args...
  local n=$1 want=$2 rc
  shift 2
  rc=$(run "$@")
  if [ "$rc" = "$want" ]; then ok; else bad "$n: rc $rc, want $want"; cat "$t/out"; fi
}
has() {
  if grep -q -- "$2" "$t/out"; then ok; else bad "$1: no '$2' in output"; cat "$t/out"; fi
}

b=$t/boot
m=$t/m.txt
mkdir -p "$b/mmc0_extlinux" "$b/mmc1_extlinux"
printf 'kernel' >"$b/Image.gz"
printf 'dtb' >"$b/stm32mp257f-dk.dtb"
printf 'scr' >"$b/boot.scr.uimg"
printf 'label' >"$b/mmc0_extlinux/stm32mp257f-dk_extlinux.conf"
printf 'e' >"$b/mmc1_extlinux/stm32mp257f-dk_extlinux.conf"
ln -s Image.gz "$b/Image"

expect record 0 record "$b" "$m"
if [ "$(wc -l <"$m")" = 6 ]; then ok; else bad "record: want 6 lines"; fi
want=$(printf kernel | sha256sum | cut -d' ' -f1)
if grep -q "^$want  Image.gz$" "$m"; then ok; else bad "record: sha256 line"; fi
expect identical 0 compare "$b" "$m"

# our directory may come and go, and its contents never count
mkdir -p "$b/yakogl/standalone/backup"
echo x >"$b/yakogl/standalone/boot.scr.uimg"
cp "$m" "$b/yakogl/standalone/boot-sha256.txt"
expect ours-allowed 0 compare "$b" "$m"
expect ours-not-gone 1 compare "$b" "$m" --gone
has ours-not-gone "must be gone"
rm -rf "$b/yakogl"
expect gone 0 compare "$b" "$m" --gone
# a sibling under yakogl/ is not ours
mkdir -p "$b/yakogl"
echo y >"$b/yakogl/once.scr"
expect sibling 1 compare "$b" "$m"
has sibling "new      yakogl/once.scr"
rm -rf "$b/yakogl"

printf 'kernelX' >"$b/Image.gz"
expect changed 1 compare "$b" "$m"
has changed "changed  Image.gz"
has changed "NEEDS A PERSON"
printf 'kernel' >"$b/Image.gz"
rm "$b/boot.scr.uimg"
expect missing 1 compare "$b" "$m"
has missing "missing  boot.scr.uimg"
printf 'scr' >"$b/boot.scr.uimg"
echo n >"$b/new.txt"
expect added 1 compare "$b" "$m"
has added "new      new.txt"
rm "$b/new.txt"
ln -sf stm32mp257f-dk.dtb "$b/Image"
expect link 1 compare "$b" "$m"
has link "changed  Image"
ln -sf Image.gz "$b/Image"
expect restored 0 compare "$b" "$m"

expect usage 2
expect nodir 2 compare "$t/none" "$m"
expect nomanifest 2 compare "$b" "$t/none.txt"
echo garbage >"$t/bad.txt"
expect badmanifest 2 compare "$b" "$t/bad.txt"

echo "mp2s_boot_manifest_selftest: $pass ok, $fail failed"
[ "$fail" = 0 ]

#!/bin/bash
# No-board selftest of tools/mp2s_boot_manifest.py (python3 only).
set -u
here=$(cd "$(dirname "$0")" && pwd)
M="python3 -I $here/mp2s_boot_manifest.py"
T=$(mktemp -d "${TMPDIR:-/tmp}/mp2s-manifest.XXXXXX")
trap 'chmod -R u+w "$T"; rm -rf "$T"' EXIT
fail=0
ok()  { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }
# expect NAME RC GREP cmd...
expect() {
    local n=$1 rc=$2 pat=$3; shift 3
    out=$("$@" 2>&1); r=$?
    if [ "$r" = "$rc" ] && echo "$out" | grep -q -- "$pat"; then ok "$n"; else bad "$n (rc=$r: $out)"; fi
}
tab=$(printf '\t')

D=$T/boot
mkdir -p "$D/extlinux" "$D/yakogl/standalone"
echo kernel > "$D/Image"; echo dtb > "$D/stm32.dtb"
printf 'label\n' > "$D/extlinux/extlinux.conf"
echo old > "$D/yakogl/standalone/default.bin"
chmod 644 "$D/Image"; chmod 755 "$D/stm32.dtb"
ln -s Image "$D/Image.lnk"
before=$(find "$D" -printf '%p %m %T@\n' | sort)
$M record "$D" -o "$T/rec.txt" || bad record
grep -q "^@link${tab}Image${tab}Image.lnk\$" "$T/rec.txt" && ok "record has link" || bad "record has link"
grep -q "^@mode${tab}0755${tab}stm32.dtb\$" "$T/rec.txt" && ok "record has mode" || bad "record has mode"
$M record "$D" -o "$T/rec2.txt"
cmp -s "$T/rec.txt" "$T/rec2.txt" && ok "record is stable" || bad "record is stable"
[ "$before" = "$(find "$D" -printf '%p %m %T@\n' | sort)" ] && ok "DIR untouched" || bad "DIR untouched"

expect same 0 '^same: 5 files' $M compare "$D" "$T/rec.txt"
$M record "$D" -o - | $M compare "$D" - >/dev/null && ok "manifest on stdin" || bad "manifest on stdin"

echo kernex > "$D/Image"
expect "changed byte" 1 'changed: Image (content)' $M compare "$D" "$T/rec.txt"
echo kernel > "$D/Image"
echo x > "$D/new.txt"
expect added 1 'added: new.txt' $M compare "$D" "$T/rec.txt"
rm "$D/new.txt"; rm "$D/stm32.dtb"
expect removed 1 'removed: stm32.dtb' $M compare "$D" "$T/rec.txt"
echo dtb > "$D/stm32.dtb"; chmod 755 "$D/stm32.dtb"
chmod 600 "$D/Image"
expect "changed mode" 1 'mode 0644 -> 0600' $M compare "$D" "$T/rec.txt"
chmod 644 "$D/Image"
rm "$D/Image.lnk"; ln -s stm32.dtb "$D/Image.lnk"
expect "changed symlink" 1 'link target Image -> stm32.dtb' $M compare "$D" "$T/rec.txt"
rm "$D/Image.lnk"; echo Image > "$D/Image.lnk"
expect "link to file" 1 'link -> file' $M compare "$D" "$T/rec.txt"
rm "$D/Image.lnk"; ln -s Image "$D/Image.lnk"
expect "restored" 0 '^same' $M compare "$D" "$T/rec.txt"

echo new > "$D/yakogl/standalone/default.bin"; echo n > "$D/yakogl/standalone/boot-sha256.txt"
expect "allowed prefix" 0 'default.bin (content) (allowed)' $M compare "$D" "$T/rec.txt"
expect "allow overridden" 1 'changed: yakogl/standalone/default.bin' $M compare "$D" "$T/rec.txt" --allow nothing/

$M record "$D" -o "$T/rec3.txt"
expect "records differ allowed" 0 'allowed' $M compare-records "$T/rec.txt" "$T/rec3.txt"
echo y >> "$D/Image"; $M record "$D" -o "$T/rec4.txt"
expect "records differ" 1 'changed: Image' $M compare-records "$T/rec.txt" "$T/rec4.txt"
$M record "$D" -o - | $M compare-records "$T/rec4.txt" - >/dev/null && ok "records with stdin" || bad "records with stdin"
expect "both stdin" 1 'only one' $M compare-records - -
expect "bad manifest" 1 'not a manifest' $M compare-records "$here/mp2s_boot_manifest.py" "$T/rec.txt"
expect "not a dir" 1 'not a directory' $M record "$T/nonexistent" -o -
exit $fail

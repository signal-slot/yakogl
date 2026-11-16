#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Selftest of tools/mp2s_boot_manifest.py (no board, no network): a fake
# /boot tree under $TMPDIR, a record, every way a difference shows up, the
# allowed prefix, and the sha256sum-listing forms the runner's captures
# take.  python3 and coreutils/findutils only.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
M="python3 -I $HERE/mp2s_boot_manifest.py"
T=$(mktemp -d "${TMPDIR:-/tmp}/mp2s-boot-manifest.XXXXXX")
trap 'chmod -R u+w "$T"; rm -rf "$T"' EXIT
fail=0
checks=0
ok()  { echo "ok   $1"; checks=$((checks + 1)); }
bad() { echo "FAIL $1"; fail=1; checks=$((checks + 1)); }
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

# ---- record -------------------------------------------------------------------
$M record "$D" -o "$T/rec.txt" || bad record
grep -q "^@link${tab}Image${tab}Image.lnk\$" "$T/rec.txt" && ok "record has the link" || bad "record has the link"
grep -q "^@mode${tab}0755${tab}stm32.dtb\$" "$T/rec.txt" && ok "record has the mode" || bad "record has the mode"
grep -q "  Image\$" "$T/rec.txt" && ok "record hashes Image" || bad "record hashes Image"
$M record "$D" > "$T/rec2.txt"
cmp -s "$T/rec.txt" "$T/rec2.txt" && ok "record is stable, stdout equals -o" || bad "record is stable, stdout equals -o"
[ "$before" = "$(find "$D" -printf '%p %m %T@\n' | sort)" ] && ok "DIR untouched" || bad "DIR untouched"

# ---- compare: the tree against its own record ----------------------------------
expect same 0 '^mp2s-boot: same: 5 files$' $M compare "$D" "$T/rec.txt"
$M record "$D" -o - | $M compare "$D" - >/dev/null && ok "manifest on stdin" || bad "manifest on stdin"

echo kernex > "$D/Image"
expect "changed byte" 1 '^changed: Image (content)$' $M compare "$D" "$T/rec.txt"
expect "changed byte, verdict" 1 '^mp2s-boot: NOT same: 1 difference' $M compare "$D" "$T/rec.txt"
echo kernel > "$D/Image"

echo x > "$D/new.txt"
expect added 1 '^added: new.txt$' $M compare "$D" "$T/rec.txt"
rm "$D/new.txt"; rm "$D/stm32.dtb"
expect removed 1 '^removed: stm32.dtb$' $M compare "$D" "$T/rec.txt"
echo dtb > "$D/stm32.dtb"; chmod 755 "$D/stm32.dtb"

chmod 600 "$D/Image"
expect "changed mode" 1 'mode 0644 -> 0600' $M compare "$D" "$T/rec.txt"
chmod 644 "$D/Image"

rm "$D/Image.lnk"; ln -s stm32.dtb "$D/Image.lnk"
expect "changed link target" 1 'link target Image -> stm32.dtb' $M compare "$D" "$T/rec.txt"
rm "$D/Image.lnk"; echo Image > "$D/Image.lnk"
expect "link to file" 1 'link -> file' $M compare "$D" "$T/rec.txt"
rm "$D/Image.lnk"; ln -s Image "$D/Image.lnk"
expect "restored" 0 '^mp2s-boot: same: 5 files$' $M compare "$D" "$T/rec.txt"

rm "$D/stm32.dtb"; ln -s Image "$D/stm32.dtb"
expect "file to link" 1 'file -> link' $M compare "$D" "$T/rec.txt"
rm "$D/stm32.dtb"; echo dtb > "$D/stm32.dtb"; chmod 755 "$D/stm32.dtb"

# ---- the allowed prefix: yakogl/standalone/ is ours, everything else is ST's ---
echo new > "$D/yakogl/standalone/default.bin"; echo n > "$D/yakogl/standalone/boot-sha256.txt"
expect "allowed content" 0 'changed: yakogl/standalone/default.bin (content) (allowed)' $M compare "$D" "$T/rec.txt"
expect "allowed verdict" 0 '^mp2s-boot: allowed: ' $M compare "$D" "$T/rec.txt"
expect "allow replaces default" 1 '^mp2s-boot: NOT same: 2 difference' $M compare "$D" "$T/rec.txt" --allow nothing/
$M compare "$D" "$T/rec.txt" --allow yakogl/standalone/ --allow new/ >/dev/null
[ $? = 0 ] && ok "allow lists stack" || bad "allow lists stack"

# ---- compare-records -------------------------------------------------------------
$M record "$D" -o "$T/rec3.txt"   # the tree as it is above: only allowed changes
expect "records differ, allowed" 0 '^mp2s-boot: allowed: 2 difference' $M compare-records "$T/rec.txt" "$T/rec3.txt"
echo old > "$D/yakogl/standalone/default.bin"; rm "$D/yakogl/standalone/boot-sha256.txt"
echo y >> "$D/Image"; $M record "$D" -o "$T/rec4.txt"
expect "records differ" 1 '^changed: Image (content)$' $M compare-records "$T/rec.txt" "$T/rec4.txt"
$M record "$D" -o - | $M compare-records "$T/rec4.txt" - >/dev/null && ok "records with stdin" || bad "records with stdin"
expect "both stdin" 2 'only one side' $M compare-records - -
expect "bad manifest" 2 'not a manifest' $M compare-records "$HERE/mp2s_boot_manifest.py" "$T/rec.txt"
expect "missing record file" 2 'cannot read' $M compare-records "$T/nonexistent.txt" "$T/rec.txt"

# ---- usage and the missing directory ---------------------------------------------
expect "no arguments" 2 usage $M
expect "record: no directory" 2 'not a directory' $M record "$T/nonexistent" -o -
expect "compare: no directory" 2 'not a directory' $M compare "$T/nonexistent" "$T/rec.txt"
expect "record: dir and --from" 2 'exactly one' $M record "$D" --from "$T/rec.txt"

# ---- the ssh capture forms: sha256sum listings ------------------------------------
# (back to the recorded state first: a capture is then the tree's own record)
echo kernel > "$D/Image"
# `cd /boot && find . -type f ... | xargs -0 sha256sum` relative paths, and the
# interim `find /boot ...` absolute paths: both parse (`./` is stripped,
# absolute paths are kept as they are), and a listing tracks neither modes
# nor symlinks.
(cd "$D" && find . -type f -print0 | sort -z | xargs -0 sha256sum) > "$T/cap.txt"
find "$D" -type f -print0 | sort -z | xargs -0 sha256sum > "$T/cap-abs.txt"
find "$D" -type f -print0 | sort -z | xargs -0 sha256sum > "$T/cap-abs2.txt"
cmp -s "$T/cap-abs.txt" "$T/cap-abs2.txt" && ok "absolute capture is stable" || bad "absolute capture is stable"
expect "absolute captures compare" 0 '^mp2s-boot: same: 4 files$' $M compare-records "$T/cap-abs.txt" "$T/cap-abs2.txt"
$M record --from "$T/cap.txt" -o "$T/caprec.txt" || bad "record --from"
# a listing records no modes, so its canonical manifest records none either
grep -q "@mode" "$T/caprec.txt" && bad "capture record has no modes" || ok "capture record has no modes"
expect "capture equals the tree record" 0 '^mp2s-boot: same: 4 files$' $M compare-records "$T/rec.txt" "$T/caprec.txt"
expect "live tree against a capture record" 0 '^mp2s-boot: same: 5 files$' $M compare "$D" "$T/caprec.txt"
echo kernex > "$D/Image"
(cd "$D" && find . -type f -print0 | sort -z | xargs -0 sha256sum) | $M record --from - -o "$T/cap2.txt"
expect "capture sees a changed byte" 1 '^changed: Image (content)$' $M compare-records "$T/caprec.txt" "$T/cap2.txt"
expect "raw listings compare too" 1 '^changed: Image (content)$' $M compare-records "$T/cap.txt" "$T/cap2.txt"
echo kernel > "$D/Image"

echo "$checks checks, $fail failed"
[ "$fail" = 0 ]

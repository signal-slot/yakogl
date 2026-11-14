#!/usr/bin/env bash
# Selftest of tools/mp2s_protect_scan.sh: good and bad fixtures in a mktemp
# dir under $TMPDIR, then the scan over the merged tree must exit 0.
# No board, no network. Run: tools/mp2s_protect_scan_selftest.sh
set -u

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
scan="$here/mp2s_protect_scan.sh"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_protect_scan.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT

fails=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fails=$((fails + 1)); }

# fixtures name protected things as @XX@ so this file holds no write form
xp() {
	local s=$1
	s=${s//@AB@/altbootcmd}
	s=${s//@BC@/bootcmd}
	s=${s//@CNT@/bootcount}
	s=${s//@FA@/fip-a}
	s=${s//@FB@/fip-b}
	s=${s//@S1@/fsbla1}
	s=${s//@S2@/fsbla2}
	s=${s//@M1@/metadata1}
	s=${s//@M2@/metadata2}
	s=${s//@EM@/mmcblk0}
	s=${s//@Z@/0}
	printf '%s\n' "$s"
}

# expect <name> <want-rc> <file...>
expect() {
	local name=$1 want=$2 got
	shift 2
	bash "$scan" "$@" >"$tmp/out" 2>"$tmp/err"
	got=$?
	if [ "$got" = "$want" ]; then ok "$name"; else bad "$name (rc $got, want $want)"; fi
}

# one fixture file per line: each BAD line must be a hit, each GOOD a pass
bad_lines=(
	'setenv @BC@ run mp2s'
	'setenv @AB@ run x'
	'setenv @CNT@ 0'
	'env set @BC@ foo'
	'fw_setenv @CNT@ 0'
	'fw_setenv -c /etc/fw.cfg @AB@ x'
	'subprocess.run(["fw_setenv", "@BC@", "x"])'
	'dd if=x.bin of=/dev/disk/by-partlabel/@FA@ bs=1M'
	'dd if=x of=/dev/disk/by-partlabel/@S1@'
	'dd if=x of=/dev/@EM@p3'
	'cat x > /dev/disk/by-partlabel/@M1@'
	'echo 0 >> /dev/disk/by-partlabel/@M2@'
	'cat x | tee /dev/disk/by-partlabel/@FB@'
	'cp x.bin /dev/disk/by-partlabel/@S2@'
	'mkfs.ext4 /dev/disk/by-partlabel/@FA@'
	'sgdisk -d 3 /dev/@EM@'
	'parted /dev/@EM@ rm 2'
	'flash_erase /dev/mtd0 0 0 # @FA@'
	'mmc write ${loadaddr} 0 0x100 # mmc @Z@'
	'mmc dev @Z@'
	'echo x > /dev/@EM@p1'
)
good_lines=(
	'fw_printenv @BC@'
	'fw_printenv -n @CNT@'
	'cat /dev/disk/by-partlabel/@FA@'
	'sha256sum /dev/disk/by-partlabel/@S1@'
	'dd if=/dev/disk/by-partlabel/@FA@ of=/tmp/@FA@.bin bs=1M'
	'cp /dev/disk/by-partlabel/@M1@ /tmp/m1.bin'
	'echo ok > /tmp/out 2>&1'
	'setenv mp2s_left 2'
	'setenv boot_prefixes ${mp2s_prefixes}'
	'saveenv'
	'# setenv @BC@ run x'
	'echo undo: not a write of @CNT@'
	'if test "${@CNT@}" = 1; then echo seen; fi'
	'dd if=default.bin of=/boot/yakogl/standalone/default.bin'
	'cat x > /boot/yakogl/standalone/boot.scr.uimg'
	'mmc info'
)

i=0
for l in "${bad_lines[@]}"; do
	i=$((i + 1))
	xp "$l" >"$tmp/bad$i"
	expect "bad $i: $l" 1 "$tmp/bad$i"
	grep -q "^$tmp/bad$i:1:" "$tmp/out" && ok "bad $i prints file:line" || bad "bad $i prints file:line"
done
i=0
for l in "${good_lines[@]}"; do
	i=$((i + 1))
	xp "$l" >"$tmp/good$i"
	expect "good $i: $l" 0 "$tmp/good$i"
done

# file:line is the line of the hit, not the first line
xp "$(printf 'echo a\nfw_printenv @BC@\nsetenv @BC@ x\n')" >"$tmp/where"
expect "hit in line 3" 1 "$tmp/where"
grep -q "^$tmp/where:3:" "$tmp/out" && ok "reports line 3" || bad "reports line 3"
grep -q ':2:' "$tmp/out" && bad "line 2 not reported" || ok "line 2 not reported"

# allow marker: exact, at the end, after whitespace
xp "$(printf 'setenv @BC@ x # mp2s-protect: read-only\n')" >"$tmp/m1"
expect "marker allows" 0 "$tmp/m1"
xp "$(printf 'setenv @BC@ x   # mp2s-protect: read-only\n')" >"$tmp/m2"
expect "marker after spaces allows" 0 "$tmp/m2"
for variant in '# mp2s-protect: readonly' '#mp2s-protect: read-only' '# mp2s-protect: read-only.' \
	'# mp2s-protect:read-only' '# MP2S-PROTECT: read-only' '# mp2s-protect: read-only # x'; do
	xp "$(printf 'setenv @BC@ x %s\n' "$variant")" >"$tmp/mv"
	expect "marker variant refused: $variant" 1 "$tmp/mv"
done
xp "$(printf 'setenv @BC@ "x# mp2s-protect: read-only"\n')" >"$tmp/mglue"
expect "marker glued to a word refused" 1 "$tmp/mglue"
xp "$(printf '# mp2s-protect: read-only\nsetenv @BC@ x\n')" >"$tmp/mprev"
expect "marker on another line does not cover" 1 "$tmp/mprev"

# usage and defaults
expect "missing file is usage" 2 "$tmp/does-not-exist"
bash "$scan" -h >/dev/null 2>&1
[ $? = 2 ] && ok "-h exits 2" || bad "-h exits 2"
: >"$tmp/empty"
expect "empty file" 0 "$tmp/empty"
expect "two files, one bad" 1 "$tmp/good1" "$tmp/bad1"

# the merged tree
bash "$scan" >"$tmp/tree.out" 2>&1
rc=$?
if [ "$rc" = 0 ]; then ok "scan over the tree"; else bad "scan over the tree (rc $rc)"; cat "$tmp/tree.out"; fi

if [ "$fails" -gt 0 ]; then
	echo "mp2s_protect_scan_selftest: $fails FAILED"
	exit 1
fi
echo "mp2s_protect_scan_selftest: all ok"

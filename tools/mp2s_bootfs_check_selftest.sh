#!/bin/bash
# No-board selftest of tools/mp2s_bootfs_check.py (fixtures generated inline).
set -u
here=$(cd "$(dirname "$0")" && pwd)
chk="$here/mp2s_bootfs_check.py"
t=$(mktemp -d "${TMPDIR:-/tmp}/mp2sbf.XXXXXX") || exit 2
trap 'rm -rf "$t"' EXIT
fail=0

part() { # name label fstype(json) mountpoint(json) parent
  printf '{"name":"%s","kname":"%s","pkname":"%s","partlabel":"%s","fstype":%s,"mountpoint":%s,"type":"part","tran":null,"subsystems":"block:mmc:mmc_host:platform"}' \
    "$1" "$1" "$5" "$2" "$3" "$4"
}
disk() { # name boot-fstype boot-mountpoint(json)
  local d=$1 i=0 s="" l
  for l in fsbla1 fsbla2 metadata1 metadata2 fip-a fip-b u-boot-env; do
    i=$((i + 1))
    s="$s$(part "${d}p$i" "$l" null null "$d"),"
  done
  s="$s$(part "${d}p8" bootfs "\"$2\"" "$3" "$d"),$(part "${d}p9" rootfs '"ext4"' null "$d")"
  printf '{"name":"%s","kname":"%s","pkname":null,"partlabel":null,"fstype":null,"mountpoint":null,"type":"disk","tran":null,"subsystems":"block:mmc:mmc_host:platform","children":[%s]}' "$d" "$d" "$s"
}
fm() { printf '{"filesystems":[{"target":"/boot","source":"%s","fstype":"%s","options":"rw"}]}' "$1" "$2"; }

# mmcblk1 is the card, mmcblk0 the eMMC; both carry the same labels
echo "{\"blockdevices\":[$(disk mmcblk0 ext4 null),$(disk mmcblk1 ext4 '"/boot"')]}" > "$t/lsblk_sd"
echo "{\"blockdevices\":[$(disk mmcblk0 ext4 null),$(disk mmcblk1 vfat '"/boot"')]}" > "$t/lsblk_vfat"
echo "{\"blockdevices\":[$(disk mmcblk0 ext4 '"/boot"'),$(disk mmcblk1 ext4 null)]}" > "$t/lsblk_emmc"
printf 'mmcblk0 MMC\nmmcblk1 SD\n' > "$t/types"
printf 'MMC\n' > "$t/bare_mmc"
fm /dev/mmcblk1p8 ext4 > "$t/fm_sd"
fm /dev/mmcblk1p8 vfat > "$t/fm_vfat"
fm /dev/mmcblk0p8 ext4 > "$t/fm_emmc"
echo '{"filesystems":[]}' > "$t/fm_none"

want() { # name expected-rc cmd...
  local n=$1 w=$2 out rc; shift 2
  out=$("$@" 2>&1 </dev/null); rc=$?
  if [ "$rc" = "$w" ]; then echo "ok   $n: $out"; else echo "FAIL $n: rc $rc want $w: $out"; fail=1; fi
}
c() { python3 -I "$chk" "$@"; }

want sd-bootfs 0 c --findmnt "$t/fm_sd" --lsblk "$t/lsblk_sd" --disk-type "$t/types"
want emmc-same-labels 1 c --findmnt "$t/fm_emmc" --lsblk "$t/lsblk_emmc" --disk-type "$t/types"
want emmc-bare-type 1 c --findmnt "$t/fm_emmc" --lsblk "$t/lsblk_emmc" --disk-type "$t/bare_mmc"
want sd-claims-mmc 1 c --findmnt "$t/fm_sd" --lsblk "$t/lsblk_sd" --disk-type "$t/bare_mmc"
want not-mounted 1 c --findmnt "$t/fm_none" --lsblk "$t/lsblk_sd" --disk-type "$t/types"
want non-ext4 1 c --findmnt "$t/fm_vfat" --lsblk "$t/lsblk_vfat" --disk-type "$t/types"
want missing-file 2 c --findmnt "$t/nope" --lsblk "$t/lsblk_sd" --disk-type "$t/types"
want missing-args 2 c --findmnt "$t/fm_sd"
want garbage-json 2 c --findmnt "$t/types" --lsblk "$t/lsblk_sd" --disk-type "$t/types"
printf "mmcblk0 MMC\n" > "$t/types_other"
want no-type-for-disk 2 c --findmnt "$t/fm_sd" --lsblk "$t/lsblk_sd" --disk-type "$t/types_other"
sed 's/"fip-b"/"x"/g' "$t/lsblk_sd" > "$t/lsblk_short"
want missing-partition 1 c --findmnt "$t/fm_sd" --lsblk "$t/lsblk_short" --disk-type "$t/types"

# one JSON bundle on stdin
python3 -I - "$t" <<'PY' > "$t/bundle"
import json, sys
t = sys.argv[1]
r = lambda n: open(t + "/" + n).read()
print(json.dumps({"findmnt": r("fm_sd"), "lsblk": r("lsblk_sd"), "disk_type": r("types")}))
PY
want bundle 0 sh -c "python3 -I '$chk' --bundle < '$t/bundle'"
want bundle-empty 2 sh -c "echo '' | python3 -I '$chk' --bundle"
want collect 0 c --collect
c --collect | grep -q '^findmnt -J /boot' || { echo "FAIL collect text"; fail=1; }

if [ $fail = 0 ]; then echo "selftest ok"; else echo "selftest FAILED"; fi
exit $fail

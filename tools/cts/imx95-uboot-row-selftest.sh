#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx95-uboot-row.py on a synthetic image directory.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d "${TMPDIR:-.}/uboot-row.XXXXXX") || exit 2
trap 'rm -rf "$t"' EXIT
mk() { # label image_size magic(good|bad)
	mkdir -p "$t/img/$1/zephyr"
	python3 -I - "$t/img/$1/zephyr/zephyr.bin" "$2" "$3" <<'PY'
import struct, sys
b = bytearray(4096)
struct.pack_into("<Q", b, 16, int(sys.argv[2]))
b[56:60] = b"ARM\x64" if sys.argv[3] == "good" else b"XXXX"
open(sys.argv[1], "wb").write(b)
PY
	printf 'CONFIG_YAKOGL_BOARD_LIFETIME_S=300\nCONFIG_YAKOGL_BOARD_ALIVE_S=60\n' > "$t/img/$1/zephyr/.config"
	echo "$1 rc=0 sha256=x build=b" > "$t/img/$1.built"
}
mk TS1R01 100000 good
mk TS1R02 200000 good
mk TS1R03 268435457 good
mk TS1R04 100000 bad
run() { python3 -I "$here/imx95-uboot-row.py" --storage 0:2 --stage /mnt/vi95/ "$@"; }
o=$(run "$t/img" TS1R01-TS1R02)
for w in ext4load 'md5sum 0xd0000000 0x1000' 'dcache off; icache flush; go 0xd0000000' \
	'$VI95_SSH' '360 s'; do
	echo "$o" | grep -F -q -- "$w" || { echo "missing: $w"; exit 1; }
done
echo "$o" | grep -c '^== ' | grep -qx 2 || { echo "range"; exit 1; }
u=$(echo "$o" | grep -n 'rm -f' | head -1 | cut -d: -f1)
s=$(echo "$o" | grep -n 'cat > ' | head -1 | cut -d: -f1)
[ "$u" -lt "$s" ] || { echo "undo not before staging"; exit 1; }
if run "$t/img" TS1R03 >/dev/null 2>&1; then echo "oversize accepted"; exit 1; fi
if run "$t/img" TS1R04 >/dev/null 2>&1; then echo "no-header accepted"; exit 1; fi
if run "$t/img" TS1R01 TS1R03 >/dev/null 2>&1; then echo "mixed accepted"; exit 1; fi
if python3 -I "$here/imx95-uboot-row.py" "$t/img" TS1R01 >/dev/null 2>&1; then echo "args defaulted"; exit 1; fi
# --stage file name + a range: every row would land in one file
if python3 -I "$here/imx95-uboot-row.py" --storage 0:2 --stage "$t/x.bin" "$t/img" TS1R01-TS1R02 >/dev/null 2>&1; then echo "colliding --stage accepted"; exit 1; fi
if python3 -I "$here/imx95-uboot-row.py" --storage 0:2 --stage "/a b/" "$t/img" TS1R01 >/dev/null 2>&1; then echo "whitespace --stage accepted"; exit 1; fi
# --stage file name for a single label is the session's choice
o=$(python3 -I "$here/imx95-uboot-row.py" --storage 0:2 --stage "$t/vi95.bin" "$t/img" TS1R01)
echo "$o" | grep -F -q -- 'rm -f '"$t/vi95.bin" || { echo "single-label --stage file"; exit 1; }
# no per-image .config: the bound comes from driver/samples/zephyr/deqp/verdin_imx95.conf
mk TS1R05 100000 good
rm "$t/img/TS1R05/zephyr/.config"
o=$(run "$t/img" TS1R05)
[ "$(grep -c 'LIFETIME_S=600' "$here/../../driver/samples/zephyr/deqp/verdin_imx95.conf")" -eq 1 ] || { echo "conf moved"; exit 1; }
echo "$o" | grep -F -q -- '600 s lifetime + 60 s WDOG3 = 660 s' || { echo "conf fallback"; exit 1; }
# --fs fat: fatload with the name relative to the partition (S1a's route)
fat() { python3 -I "$here/imx95-uboot-row.py" --storage 0:1 --stage /boot/ --fs fat "$@"; }
mk T6S1R01 100000 good
truncate -s 12768592 "$t/img/T6S1R01/zephyr/zephyr.bin"   # 0xc2d550, as S1a's README
o=$(fat "$t/img" T6S1R01)
for w in 'fatload mmc 0:1 0xd0000000 T6S1R01.bin' 'cat > /boot/T6S1R01.bin' \
	"rm -f /boot/T6S1R01.bin" 'md5sum /boot/T6S1R01.bin' 'md5sum 0xd0000000 0xc2d550'; do
	echo "$o" | grep -F -q -- "$w" || { echo "fat missing: $w"; exit 1; }
done
if echo "$o" | grep -F -q ext4load; then echo "fat printed ext4load"; exit 1; fi
o=$(fat "$t/img" TS1R01-TS1R02)
echo "$o" | grep -c '^fatload mmc 0:1 0xd0000000 TS1R0[12]\.bin$' | grep -qx 2 || { echo "fat range"; exit 1; }
o=$(python3 -I "$here/imx95-uboot-row.py" --storage 0:1 --stage /boot/x.bin --fs fat "$t/img" TS1R01)
echo "$o" | grep -F -q 'fatload mmc 0:1 0xd0000000 x.bin' || { echo "fat file"; exit 1; }
o=$(python3 -I "$here/imx95-uboot-row.py" --storage 1:3 --stage /mnt/b/ --mount /mnt/b --fs fat "$t/img" TS1R01)
echo "$o" | grep -F -q 'fatload mmc 1:3 0xd0000000 TS1R01.bin' || { echo "fat mount"; exit 1; }
if python3 -I "$here/imx95-uboot-row.py" --storage 0:1 --stage /mnt/ --fs fat "$t/img" TS1R01 >/dev/null 2>&1; then echo "fat outside mount accepted"; exit 1; fi
run "$t/img" TS1R01 | grep -F -q 'ext4load mmc 0:2 0xd0000000 /mnt/vi95/TS1R01.bin' || { echo "ext4 default"; exit 1; }
echo ok

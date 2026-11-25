#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of imx95-uboot-row.py on a synthetic image directory.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d "${TMPDIR:-/tmp}/uboot-row.XXXXXX")
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
run() { python3 -I "$here/imx95-uboot-row.py" --storage 0:2 --stage /var/tmp/ "$@"; }
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
echo ok

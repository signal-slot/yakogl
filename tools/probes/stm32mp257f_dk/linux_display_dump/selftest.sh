#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host selftest of linux_display_dump.py with fake registers and a fake
# /sys tree; also checks the allow file and that the script has no write path.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
allow=$repo/driver/docs/stm32mp257f-dk-display-allow.txt
py=$here/linux_display_dump.py
t=$(mktemp -d "${TMPDIR:-.}/lddump.XXXXXX")
trap 'rm -rf "$t"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

# 1. the allow file: every non-comment line is "hex hex text"
awk '!/^[[:space:]]*(#|$)/ {
         n++
         if ($1 !~ /^0x[0-9a-f]+$/ || $2 !~ /^0x[0-9a-f]+$/ || NF < 3) { print "bad line " NR ": " $0; bad = 1 }
     }
     END { if (bad || n == 0) exit 1 }' "$allow" || fail "allow file does not parse"

# 2. read-only: no write, no mount, no module, no service in the script
if grep -nE 'O_RDWR|O_WRONLY|PROT_WRITE|insmod|modprobe|mount |systemctl|tailscale|sshd|chmod|shutil' "$py"; then
    fail "the dump script contains a write-capable construct"
fi

# 3. a fake world
d=$t/root/sys/kernel/debug/regmap
mkdir -p "$t/root/sys/class/drm/card0-HDMI-A-1" "$d/2-003d" "$d/2-003c" "$d/2-003f" "$d/2-0038" "$d/5-003d"
printf '00: 14\n01: 75\n41: 50\n' > "$d/2-003d/registers"
printf '03: 89\n' > "$d/2-003c/registers"
printf '00: 00\n01: ff\n' > "$d/2-003f/registers"
printf '00: aa\n' > "$d/2-0038/registers"
printf '00: ee\n' > "$d/5-003d/registers"
c=$t/root/sys/class/drm/card0-HDMI-A-1
printf '\000\377\377\377\377\377\377\000' > "$c/edid"
printf '1920x1080\n1280x720\n' > "$c/modes"
echo connected > "$c/status"
cat > "$t/regs" <<'R'
0x48010000 0x00040100
0x48010004 0x00000002
0x48010008 0x00000003
0x4801000c 0x00000004
0x48000000 0x3134312a
0x48000004 0x00000001
R
cat > "$t/list" <<'R'
#@read 0x48010000 4 LTDC
#@name 0x48010000 LTDC_IDR
#@read 0x48000000 2 DSI
#@name 0x48000004 DSI_PWR_UP
#@read 0x48000100 1 UNREADABLE
R
python3 -I "$py" --readlist "$t/list" --reader "file:$t/regs" --root "$t/root" \
    --no-modetest | grep -v '^# kernel ' > "$t/out"

cat > "$t/want" <<'R'
# Linux display dump of the DK (read only). Format: see README.md.
[regs]
0x48010000 0x00040100 LTDC_IDR
0x48010004 0x00000002 LTDC+0x4
0x48010008 0x00000003 LTDC+0x8
0x4801000c 0x00000004 LTDC+0xc
0x48000000 0x3134312a DSI+0x0
0x48000004 0x00000001 DSI_PWR_UP
0x48000100 ERR UNREADABLE+0x0
[adv7535]
# map packet i2c 0x38
packet 0x00 0xaa
# map cec i2c 0x3c
cec 0x03 0x89
# map main i2c 0x3d
main 0x00 0x14
main 0x01 0x75
main 0x41 0x50
# map edid i2c 0x3f
edid 0x00 0x00
edid 0x01 0xff
[edid]
card0-HDMI-A-1 00ffffffffffff00
[modes]
card0-HDMI-A-1 status connected
card0-HDMI-A-1 mode 1920x1080
card0-HDMI-A-1 mode 1280x720
[modetest]
# modetest not run
R
diff -u "$t/want" "$t/out" || fail "dump output differs"

# 4. the real read list parses and has the display blocks
python3 -I -c '
import sys
sys.path.insert(0, sys.argv[1])
import linux_display_dump as d
b, n = d.parse_readlist(open(sys.argv[2]).read())
bases = {a for a, _, _ in b}
for need in (0x48010000, 0x48000000, 0x48000400, 0x44200840, 0x44235000):
    assert need in bases, hex(need)
assert n[0x48010134] == "LTDC_L1CFBAR"
for bad in (0x48000070, 0x480000bc, 0x480000c0):
    assert not any(a <= bad < a + 4 * w for a, w, _ in b), hex(bad)
' "$here" "$allow" || fail "read list"

echo "linux_display_dump selftest: ok"

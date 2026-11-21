#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host selftest of day_one.sh on a fake board: a fake /proc, /sys and /etc
# tree and fake commands (dmesg, i2cdetect, i2cdump, uname, findmnt) on PATH,
# and a register fixture for M4. Checks the record's lines for M1..M3, that
# M4 reads only behind its gates, that the read list (the allow file's
# #@read lines) parses and leaves out the DW host's read-sensitive words,
# and that the script holds no write-capable construct.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
allow=$repo/driver/docs/imx95-display-allow.txt
sh_=$here/day_one.sh
t=$(mktemp -d "${TMPDIR:-/tmp}/vi95day1.XXXXXX")
trap 'rm -rf "$t"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
has() { grep -qxF -- "$1" "$2" || fail "no line '$1' in $2"; }

# 1. read only: no write, no mount, no module, no service, no forced or
# writing I2C, no devmem with a value
if grep -vE '^[[:space:]]*#' "$sh_" | grep -nE 'O_RDWR|O_WRONLY|PROT_WRITE|i2cset|i2ctransfer|i2cdump[^|]*-f|modprobe|insmod|mount |systemctl|devmem "\$1" 32 [^2]|> *"?\$ROOT'; then
	fail "day_one.sh contains a write-capable construct"
fi

# 2. the allow file: its non-comment lines are "hex hex text"; its read
# list parses, covers M4's blocks and leaves out GEN_PLD_DATA and INT_ST0/1
awk '!/^[[:space:]]*(#|$)/ {
         n++
         if ($1 !~ /^0x[0-9a-f]+$/ || $2 !~ /^0x[0-9a-f]+$/ || NF < 3) { print "bad line " NR ": " $0; bad = 1 }
     }
     END { if (bad || n == 0) exit 1 }' "$allow" || fail "allow file does not parse"
grep '^#@read ' "$allow" > "$t/readlist" || fail "no read list in the allow file"
while read -r _ a n name gate; do
	case $a in 0x[0-9a-f]*) ;; *) fail "read list: address $a" ;; esac
	case $n in *[!0-9]* | '') fail "read list: count $n" ;; esac
	case $gate in display | camera) ;; *) fail "read list: $name has gate '$gate'" ;; esac
	i=0
	while [ "$i" -lt "$n" ]; do
		x=$((a + 4 * i))
		for bad in 0x4acf0070 0x4acf00bc 0x4acf00c0; do
			[ "$x" -ne $((bad)) ] || fail "read list: $name reads $bad, which changes on read"
		done
		i=$((i + 1))
	done
done < "$t/readlist"
for need in 0x4b400000 0x4b5d0008 0x4b510008 0x4b6b0008 0x4b6d0008 0x4b6d0400 0x4b010000 \
	0x4b0d0000 0x4ad00000 0x4ad10000 0x4ad20100 0x4acf0000; do
	grep -q "^#@read $need " "$t/readlist" || fail "read list: no block at $need"
done

# 3. a fake board
r=$t/root
b=$t/bin
mkdir -p "$r/proc/device-tree/chosen" "$r/etc" "$r/boot" "$b" \
	"$r/sys/class/drm/card1-DSI-1" "$r/sys/class/drm/card1-HDMI-A-1" \
	"$r/sys/bus/i2c/devices/i2c-2" "$r/sys/bus/i2c/devices/i2c-1" \
	"$r/sys/kernel/debug/pm_genpd" "$r/sys/kernel/debug/clk" "$r/sys/kernel/debug/scmi/0/info" \
	"$r/sys/kernel/debug/regmap/2-002c" "$r/of/i2c@42530000" "$r/of/i2c@44350000"
printf 'Toradex Verdin iMX95 WB on Mallow Board\000' > "$r/proc/device-tree/model"
printf 'toradex,verdin-imx95-wifi-mallow\000fsl,imx95\000' > "$r/proc/device-tree/compatible"
printf 'U-Boot 2024.04-6.8.0+git.abc\000' > "$r/proc/device-tree/chosen/u-boot,version"
printf 'NAME="TDX Wayland with XWayland"\nVERSION_ID=7.3.0\n' > "$r/etc/os-release"
printf 'fdt_overlays=verdin-imx95_dsi-to-lvds_panel-cap-touch-10inch-dsi_overlay.dtbo\n' > "$r/boot/overlays.txt"
printf '80000000-87ffffff : System RAM\n4d900000-4dd7ffff : 4d900000.gpu\n' > "$r/proc/iomem"
printf '           CPU0\n 309:  12  GICv3 289 Level  4d900000.gpu\n' > "$r/proc/interrupts"
printf 'MemTotal: 8000000 kB\nMemFree: 1 kB\nMemAvailable: 2 kB\n' > "$r/proc/meminfo"
echo 'root=/dev/mmcblk0p2 console=ttyLP2,115200' > "$r/proc/cmdline"
c=$r/sys/class/drm/card1-DSI-1
echo connected > "$c/status"; echo enabled > "$c/enabled"; echo On > "$c/dpms"; printf '1280x800\n' > "$c/modes"
c=$r/sys/class/drm/card1-HDMI-A-1
echo disconnected > "$c/status"; echo disabled > "$c/enabled"; echo Off > "$c/dpms"
ln -s "$r/of/i2c@42530000" "$r/sys/bus/i2c/devices/i2c-2/of_node"
ln -s "$r/of/i2c@44350000" "$r/sys/bus/i2c/devices/i2c-1/of_node"
printf '00: 35\n18: 78\n' > "$r/sys/kernel/debug/regmap/2-002c/registers"
printf 'NXP\n' > "$r/sys/kernel/debug/scmi/0/info/vendor_id"
printf '0x1000\n' > "$r/sys/kernel/debug/scmi/0/info/impl_version"
cat > "$r/sys/kernel/debug/clk/clk_summary" <<'C'
                                 enable  prepare  protect                                duty  hardware                            connection
   clock                          count    count    count        rate   accuracy phase  cycle    enable   consumer                         id
---------------------------------------------------------------------------------------------------------------------------------------------
    disp1pix                         1       1        0        70000000    0          0     50000      Y      4b400000.display-controller  pix
    dispapb                          1       1        0        133333333   0          0     50000      Y      deviceless                   no_connection_id
    uart1                            1       1        0        24000000    0          0     50000      Y      deviceless                   no_connection_id
C
make_genpd() {
	cat > "$r/sys/kernel/debug/pm_genpd/pm_genpd_summary" <<G
domain                          status          children                           performance
    /device                                             runtime status
----------------------------------------------------------------------------------------------
display                         $1                                                  0
camera                          $2                                                  0
gpu                             off-0                                               0
G
}
make_genpd on on
cat > "$t/regs" <<'R'
0x4b400000 0x12345678
0x4acf0000 0x3133302a
R
for f in dmesg i2cdetect i2cdump uname findmnt; do
	cat > "$b/$f" <<F
#!/bin/sh
case $f in
dmesg) printf '[ 1.0] arm-scmi arm-scmi.0.auto: SCMI Protocol v2.1 '"'"'NXP:i.MX95 SM'"'"' Firmware version 0x1000\n[ 2.0] dpu95 4b400000.display-controller: Using Pixel Link0\n[ 3.0] panthor 4d900000.gpu: [drm] mali-g310 id 0xac74 major 0x0 minor 0x0 status 0x1\n' ;;
i2cdetect) echo "     0  1  2  3"; echo "20: -- -- -- -- -- -- -- -- -- -- -- -- UU -- -- --" ;;
i2cdump) echo "i2cdump ran \$*" ;;
uname) [ "\${1:-}" = -r ] && echo 6.6.94-7.3.0 || echo "Linux verdin-imx95 6.6.94-7.3.0 aarch64" ;;
findmnt) printf 'TARGET SOURCE\n/ /dev/mmcblk0p2\n' ;;
esac
F
	chmod +x "$b/$f"
done

run() { PATH="$b:$PATH" sh "$sh_" --root "$r" --reader "file:$t/regs" "$@"; }

# 4. both domains on, the DSI connector live: M1..M4
run --readlist "$allow" > "$t/out" || fail "day_one.sh failed"
has '[m1]' "$t/out"
has 'model: Toradex Verdin iMX95 WB on Mallow Board' "$t/out"
has 'kernel release: 6.6.94-7.3.0' "$t/out"
has 'os-release: VERSION_ID=7.3.0' "$t/out"
has 'overlays /boot/overlays.txt: fdt_overlays=verdin-imx95_dsi-to-lvds_panel-cap-touch-10inch-dsi_overlay.dtbo' "$t/out"
has 'u-boot version (device tree): U-Boot 2024.04-6.8.0+git.abc' "$t/out"
has 'dpu95 in this kernel: 1 dmesg lines' "$t/out"
has 'iomem: 4d900000-4dd7ffff : 4d900000.gpu' "$t/out"
has 'gpu dmesg: [ 3.0] panthor 4d900000.gpu: [drm] mali-g310 id 0xac74 major 0x0 minor 0x0 status 0x1' "$t/out"
has 'card1-DSI-1 status connected' "$t/out"
has 'card1-DSI-1 mode 1280x800' "$t/out"
has 'dsi connector connected and enabled: 1' "$t/out"
has 'i2c_2_dsi bus: 2' "$t/out"
has 'bridge regmap: 18: 78' "$t/out"
has "sm dmesg: [ 1.0] arm-scmi arm-scmi.0.auto: SCMI Protocol v2.1 'NXP:i.MX95 SM' Firmware version 0x1000" "$t/out"
has 'scmi info 0/info/impl_version: 0x1000' "$t/out"
grep -q '^clk:     disp1pix ' "$t/out" || fail "no disp1pix row"
grep -q '^clk:     uart1 ' "$t/out" && fail "a clock outside the list"
has 'display domain on: 1' "$t/out"
has 'camera domain on: 1' "$t/out"
has 'crash reservation: none in /proc/iomem' "$t/out"
has 'reader: file:'"$t/regs" "$t/out"
has '0x4b400000 0x12345678 dpu_COMCTRL_IPIDENTIFIER+0x0' "$t/out"
has '0x4acf0000 0x3133302a dw_dsi+0x0' "$t/out"
has '0x4b5d0008 ERR dpu_FetchLayer0+0x0' "$t/out"
grep -q '^0x4acf0070 ' "$t/out" && fail "GEN_PLD_DATA was read"
has '# end' "$t/out"

# 5. CAMERA off: the camera side is not read, the display side is
make_genpd on off
run --readlist "$allow" > "$t/out2" || fail "day_one.sh failed (camera off)"
has '# dw_dsi: gate camera does not hold, not read' "$t/out2"
grep -q '^0x4acf' "$t/out2" && fail "a DSI word read with CAMERA off"
has '0x4b400000 0x12345678 dpu_COMCTRL_IPIDENTIFIER+0x0' "$t/out2"

# 6. DISPLAY off: nothing of M4 is read (U6)
make_genpd off off
run --readlist "$allow" > "$t/out3" || fail "day_one.sh failed (display off)"
has '# dpu_COMCTRL_IPIDENTIFIER: gate display does not hold, not read' "$t/out3"
grep -qE '^0x4[ab]' "$t/out3" && fail "a register read with DISPLAY off"

# 7. no read list, and --no-m4
run > "$t/out4" || fail "day_one.sh failed (no list)"
has '# M4 skipped: no --readlist' "$t/out4"
run --readlist "$allow" --no-m4 > "$t/out5" || fail "day_one.sh failed (--no-m4)"
has '# M4 skipped (--no-m4)' "$t/out5"

echo "day_one selftest: ok"

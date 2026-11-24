#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host selftest of day_one.sh and run_day_one.sh on a fake board: a fake
# /proc, /sys and /etc tree and fake commands (dmesg, i2cdetect, i2cdump,
# uname, findmnt, python3 without mmap, devmem2, ssh) on PATH, and a
# register fixture for M4. Checks the record's lines for M1..M3, that M4
# reads only behind its gates (the LVDS panel of the owner's board, a DSI
# panel, CAMERA off, DISPLAY off), that the reader is chosen only once it
# has read (the board's python3 has no mmap; its devmem2 reads), that the
# read list (the allow file's #@read lines) parses and covers none of its
# #@noread words (the seven words that faulted on the board, the DW host's
# read-sensitive ones), that the script holds no write-capable construct,
# and that run_day_one.sh sends only short ssh commands, runs from the
# board's /tmp and removes what it copied.
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
# writing I2C, no devmem or devmem2 with a value
if grep -vE '^[[:space:]]*#' "$sh_" | grep -nE 'O_RDWR|O_WRONLY|PROT_WRITE|i2cset|i2ctransfer|i2cdump[^|]*-f|modprobe|insmod|mount |systemctl|devmem "\$1" 32 [^2]|> *"?\$ROOT'; then
	fail "day_one.sh contains a write-capable construct"
fi
grep -vE '^[[:space:]]*#' "$sh_" | grep -F 'devmem2 "' | grep -vqF 'devmem2 "$1" w 2>/dev/null' &&
	fail "day_one.sh calls devmem2 otherwise than 'devmem2 \"\$1\" w' (a third argument writes)"

# 2. the allow file: its non-comment lines are "hex hex text"; its read
# list parses, covers M4's blocks and none of its #@noread words
awk '!/^[[:space:]]*(#|$)/ {
         n++
         if ($1 !~ /^0x[0-9a-f]+$/ || $2 !~ /^0x[0-9a-f]+$/ || NF < 3) { print "bad line " NR ": " $0; bad = 1 }
     }
     END { if (bad || n == 0) exit 1 }' "$allow" || fail "allow file does not parse"
grep '^#@read ' "$allow" > "$t/readlist" || fail "no read list in the allow file"
grep '^#@noread ' "$allow" | awk '{ print $2 }' > "$t/noread" || fail "no #@noread list in the allow file"
# the words a single read of which faulted on the board (2026-11-25), and
# the DW host's GEN_PLD_DATA and INT_ST0/1
for need in 0x4b5d0014 0x4b5d004c 0x4b600024 0x4b510010 0x4b6b0090 0x4b6b00a0 0x4b6d0020 \
	0x4acf0070 0x4acf00bc 0x4acf00c0; do
	grep -qx "$need" "$t/noread" || fail "#@noread misses $need"
done
while read -r _ a n name gate; do
	case $a in 0x[0-9a-f]*) ;; *) fail "read list: address $a" ;; esac
	case $n in *[!0-9]* | '') fail "read list: count $n" ;; esac
	case $gate in display | lvds | dsi | camera) ;; *) fail "read list: $name has gate '$gate'" ;; esac
	i=0
	while [ "$i" -lt "$n" ]; do
		x=$((a + 4 * i))
		while read -r bad; do
			[ "$x" -ne $((bad)) ] || fail "read list: $name reads $bad, a #@noread word"
		done < "$t/noread"
		i=$((i + 1))
	done
done < "$t/readlist"
for need in 0x4b0c0000 0x4b400000 0x4b5d0008 0x4b510008 0x4b6b0008 0x4b6d0008 0x4b6d0400 0x4b010000 \
	0x4b0d0000 0x4ad00000 0x4ad10000 0x4ad20100 0x4acf0000; do
	grep -q "^#@read $need " "$t/readlist" || fail "read list: no block at $need"
done
grep -q '^#@read 0x4b0c0000 4 lvds_csr lvds$' "$t/readlist" || fail "read list: the LVDS CSR is not behind the lvds gate"

# 3. a fake board: Toradex's image of 2026-11-25 (python3 without mmap,
# devmem2, no busybox devmem)
r=$t/root
b=$t/bin
mkdir -p "$r/proc/device-tree/chosen" "$r/etc" "$r/boot" "$b" \
	"$r/sys/class/drm/card0-LVDS-1" "$r/sys/class/drm/card0-DSI-1" \
	"$r/sys/class/backlight/backlight-lvds-native" \
	"$r/sys/bus/i2c/devices/i2c-2" "$r/sys/bus/i2c/devices/i2c-1" \
	"$r/sys/kernel/debug/pm_genpd" "$r/sys/kernel/debug/clk" "$r/sys/kernel/debug/scmi/0/info" \
	"$r/sys/kernel/debug/regmap/2-002c" "$r/of/i2c@42530000" "$r/of/i2c@44350000"
printf 'Toradex Verdin iMX95 WB on Verdin Development Board\000' > "$r/proc/device-tree/model"
printf 'toradex,verdin-imx95-wifi-dev\000fsl,imx95\000' > "$r/proc/device-tree/compatible"
printf 'U-Boot 2024.04-7.8.0+git.b7c747d8c084\000' > "$r/proc/device-tree/chosen/u-boot,version"
printf 'NAME="TDX Wayland with XWayland"\nVERSION_ID=7.8.0-build.17\n' > "$r/etc/os-release"
printf 'fdt_overlays=verdin-imx95_panel-cap-touch-10inch-lvds_overlay.dtbo verdin-imx95_spidev_overlay.dtbo\n' > "$r/boot/overlays.txt"
printf '80000000-87ffffff : System RAM\n4d900000-4dd7ffff : 4d900000.gpu\n' > "$r/proc/iomem"
printf '           CPU0\n 309:  12  GICv3 320 Level  gpu\n' > "$r/proc/interrupts"
printf 'MemTotal: 7884200 kB\nMemFree: 1 kB\nMemAvailable: 2 kB\n' > "$r/proc/meminfo"
echo 'root=/dev/mmcblk0p2 console=ttyLP2,115200' > "$r/proc/cmdline"
echo 4 > "$r/sys/class/backlight/backlight-lvds-native/brightness"
echo 7 > "$r/sys/class/backlight/backlight-lvds-native/max_brightness"
printf 'platform/42500000.pwm, 2 PWM devices\n pwm-0   (backlight-lvds-nativ): requested enabled period: 6666667 ns duty: 3111111 ns polarity: inverse\n' > "$r/sys/kernel/debug/pwm"
make_drm() { # $1 LVDS state, $2 DSI state: connected|disconnected
	for k in LVDS DSI; do
		c=$r/sys/class/drm/card0-$k-1
		if [ "$k" = LVDS ]; then s=$1; else s=$2; fi
		echo "$s" > "$c/status"
		if [ "$s" = connected ]; then
			echo enabled > "$c/enabled"; echo On > "$c/dpms"; printf '1280x800\n' > "$c/modes"
		else
			echo disabled > "$c/enabled"; echo Off > "$c/dpms"; rm -f "$c/modes"
		fi
	done
}
ln -s "$r/of/i2c@42530000" "$r/sys/bus/i2c/devices/i2c-2/of_node"
ln -s "$r/of/i2c@44350000" "$r/sys/bus/i2c/devices/i2c-1/of_node"
printf '00: 35\n18: 78\n' > "$r/sys/kernel/debug/regmap/2-002c/registers"
printf 'NXP\n' > "$r/sys/kernel/debug/scmi/0/info/vendor_id"
printf '0x23e\n' > "$r/sys/kernel/debug/scmi/0/info/impl_version"
cat > "$r/sys/kernel/debug/clk/clk_summary" <<'C'
                                 enable  prepare  protect                                duty  hardware                            connection
   clock                          count    count    count        rate   accuracy phase  cycle    enable   consumer                         id
---------------------------------------------------------------------------------------------------------------------------------------------
    ldbpll_vco                       1       1        0        2986200000  0          0     50000      Y      deviceless                   no_connection_id
       ldbpll                        1       1        0        497700000   0          0     50000      Y      deviceless                   no_connection_id
          lvds_ch1_gate              0       0        0        497700000   0          0     50000      N      deviceless                   no_connection_id
          ldb_pll_div7               1       1        0        71100000    0          0     50000      Y      deviceless                   no_connection_id
    disp1pix                         0       0        0        24000000    0          0     50000      N      deviceless                   no_connection_id
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
cat > "$t/regs" <<'R'
0x4b0c0000 0x00000014
0x4b0c0004 0x00082221
0x4b400000 0x21260400
0x4acf0000 0x3133302a
R
for f in dmesg i2cdetect i2cdump uname findmnt; do
	cat > "$b/$f" <<F
#!/bin/sh
case $f in
dmesg) printf '[ 1.0] arm-scmi arm-scmi.0.auto: SCMI Protocol v2.1 '"'"'NXP:IMX'"'"' Firmware version 0x23e\n[ 2.0] imx95-dpu 4b400000.display-controller: [drm] fb0: imx95-dpudrmfb\n[ 3.0] mali 4d900000.gpu: GPU identified as 0x4 arch 10.12.7 r0p0 status 0\n' ;;
i2cdetect) echo "     0  1  2  3"; echo "20: -- -- -- -- -- -- -- -- -- -- -- -- -- -- -- --" ;;
i2cdump) echo "i2cdump ran \$*" ;;
uname) [ "\${1:-}" = -r ] && echo 6.6.157-7.8.0 || echo "Linux verdin-imx95 6.6.157-7.8.0 aarch64" ;;
findmnt) printf 'TARGET SOURCE\n/ /dev/mmcblk0p2\n' ;;
esac
F
	chmod +x "$b/$f"
done
# the board's python3: no mmap module
cat > "$b/python3" <<'F'
#!/bin/sh
echo "ModuleNotFoundError: No module named 'mmap'" >&2
exit 1
F
# the board's devmem2: "Value at address 0x... (0x...): 0x...", from the
# fixture; a third argument (a value) is refused here and logged
cat > "$b/devmem2" <<F
#!/bin/sh
echo "devmem2 \$*" >> "$t/devmem2.log"
[ \$# -eq 2 ] && [ "\$2" = w ] || { echo "devmem2: asked to write" >&2; exit 9; }
v=\$(awk -v a="\$1" 'tolower(\$1) == tolower(a) { print \$2; exit }' "$t/regs")
[ -n "\$v" ] || { echo "Bus error" >&2; exit 1; }
printf '/dev/mem opened.\nMemory mapped at address 0x7f8a000000.\nValue at address 0x%X (0x7f8a000000): 0x%X\n' "\$1" "\$v"
F
chmod +x "$b/python3" "$b/devmem2"

run() { PATH="$b:/usr/bin:/bin" sh "$sh_" --root "$r" "$@"; }

# 4. the owner's board: the LVDS panel on, DISPLAY and CAMERA on, the
# reader found by trying (python3 fails, devmem2 reads)
make_drm connected disconnected
make_genpd on on
run --readlist "$allow" > "$t/out" || fail "day_one.sh failed"
has '[m1]' "$t/out"
has 'model: Toradex Verdin iMX95 WB on Verdin Development Board' "$t/out"
has 'kernel release: 6.6.157-7.8.0' "$t/out"
has 'overlays /boot/overlays.txt: fdt_overlays=verdin-imx95_panel-cap-touch-10inch-lvds_overlay.dtbo verdin-imx95_spidev_overlay.dtbo' "$t/out"
has 'u-boot version (device tree): U-Boot 2024.04-7.8.0+git.b7c747d8c084' "$t/out"
has 'gpu dmesg: [ 3.0] mali 4d900000.gpu: GPU identified as 0x4 arch 10.12.7 r0p0 status 0' "$t/out"
has 'card0-LVDS-1 status connected' "$t/out"
has 'card0-LVDS-1 mode 1280x800' "$t/out"
has 'dsi connector connected and enabled: 0' "$t/out"
has 'lvds connector connected and enabled: 1' "$t/out"
has 'backlight backlight-lvds-native: brightness 4 of 7' "$t/out"
has 'pwm:  pwm-0   (backlight-lvds-nativ): requested enabled period: 6666667 ns duty: 3111111 ns polarity: inverse' "$t/out"
has "sm dmesg: [ 1.0] arm-scmi arm-scmi.0.auto: SCMI Protocol v2.1 'NXP:IMX' Firmware version 0x23e" "$t/out"
grep -q '^clk:     ldbpll_vco ' "$t/out" || fail "no ldbpll_vco row"
grep -q '^clk:           ldb_pll_div7 ' "$t/out" || fail "no ldb_pll_div7 row"
grep -q '^clk:           lvds_ch1_gate ' "$t/out" || fail "no lvds_ch1_gate row"
grep -q '^clk:     uart1 ' "$t/out" && fail "a clock outside the list"
has 'display domain on: 1' "$t/out"
has '# reader python3: could not read 0x4b0c0000' "$t/out"
has '# reader devmem2: read 0x4b0c0000 = 0x00000014' "$t/out"
has 'reader: devmem2' "$t/out"
has '0x4b0c0000 0x00000014 lvds_csr+0x0' "$t/out"
has '0x4b0c0004 0x00082221 lvds_csr+0x4' "$t/out"
has '0x4b400000 0x21260400 dpu_COMCTRL_IPIDENTIFIER+0x0' "$t/out"
has '0x4b5d0008 ERR dpu_FetchLayer0+0x0' "$t/out"
has '# dw_dsi: gate camera does not hold, not read' "$t/out"
grep -q '^0x4acf' "$t/out" && fail "a DSI word read with no DSI connector"
for bad in 0x4b5d0014 0x4b5d004c 0x4b600024 0x4b510010 0x4b6b0090 0x4b6b00a0 0x4b6d0020; do
	grep -q "^$bad " "$t/out" && fail "$bad was read"
	grep -qi "devmem2 $bad " "$t/devmem2.log" && fail "devmem2 was asked for $bad"
done
grep -qv ' w$' "$t/devmem2.log" && fail "devmem2 was called with something else than ADDR w"
has '# end' "$t/out"

# 5. no reader can read: M4 says so and reads nothing
rm "$b/devmem2"
run --readlist "$allow" > "$t/out1" || fail "day_one.sh failed (no reader)"
has '# reader devmem2: not installed' "$t/out1"
has 'reader: none' "$t/out1"
has '# M4 skipped: no reader could read 0x4b0c0000' "$t/out1"
grep -qE '^0x4[ab]' "$t/out1" && fail "a register line without a reader"

# 6. a DSI panel, both domains on (the fixture reader)
make_drm disconnected connected
run --reader "file:$t/regs" --readlist "$allow" > "$t/out2" || fail "day_one.sh failed (DSI)"
has 'dsi connector connected and enabled: 1' "$t/out2"
has '# lvds_csr: gate lvds does not hold, not read' "$t/out2"
has '0x4acf0000 0x3133302a dw_dsi+0x0' "$t/out2"
has '0x4b400000 0x21260400 dpu_COMCTRL_IPIDENTIFIER+0x0' "$t/out2"
has "reader: file:$t/regs" "$t/out2"
grep -q '^0x4acf0070 ' "$t/out2" && fail "GEN_PLD_DATA was read"

# 7. CAMERA off: the camera side is not read, the display side is
make_genpd on off
run --reader "file:$t/regs" --readlist "$allow" > "$t/out3" || fail "day_one.sh failed (camera off)"
has '# dw_dsi: gate camera does not hold, not read' "$t/out3"
grep -q '^0x4acf' "$t/out3" && fail "a DSI word read with CAMERA off"
has '0x4b400000 0x21260400 dpu_COMCTRL_IPIDENTIFIER+0x0' "$t/out3"

# 8. DISPLAY off: nothing of M4 is read (U6), no reader is tried
make_drm connected disconnected
make_genpd off off
run --readlist "$allow" > "$t/out4" || fail "day_one.sh failed (display off)"
has '# dpu_COMCTRL_IPIDENTIFIER: gate display does not hold, not read' "$t/out4"
has '# reader: no gate holds, no reader tried' "$t/out4"
grep -qE '^0x4[ab]' "$t/out4" && fail "a register read with DISPLAY off"

# 9. no read list, and --no-m4
run > "$t/out5" || fail "day_one.sh failed (no list)"
has '# M4 skipped: no --readlist' "$t/out5"
run --readlist "$allow" --no-m4 > "$t/out6" || fail "day_one.sh failed (--no-m4)"
has '# M4 skipped (--no-m4)' "$t/out6"

# 10. run_day_one.sh: a fake ssh that runs each command in the fake board's
# /tmp, logs it, and refuses one longer than 2048 bytes (the board's Dropbear
# reset the connection when the whole script, about 12 KB in base64, was one
# argument; its limit is not known, the commands here are a few hundred
# bytes with this selftest's long paths)
mkdir -p "$t/board-tmp"
cat > "$b/ssh" <<F
#!/bin/sh
while [ \$# -gt 0 ]; do
	case \$1 in
	-o) shift 2 ;;
	-*) shift ;;
	*) break ;;
	esac
done
shift # the destination
cmd="\$*"
printf '%s\n' "\$cmd" >> "$t/ssh.log"
[ \${#cmd} -le 2048 ] || { echo "Connection reset by peer" >&2; exit 255; }
exec sh -c "\$cmd"
F
chmod +x "$b/ssh"
make_genpd on on
PATH="$b:/usr/bin:/bin" VI95_SSH=board VI95_REMOTE_TMP="$t/board-tmp" \
	sh "$here/run_day_one.sh" --root "$r" --reader "file:$t/regs" > "$t/out7" ||
	fail "run_day_one.sh failed"
has '# end' "$t/out7"
has '0x4b0c0000 0x00000014 lvds_csr+0x0' "$t/out7"
[ "$(wc -l < "$t/ssh.log")" -eq 5 ] || fail "run_day_one.sh sent $(wc -l < "$t/ssh.log") ssh commands, not 5"
awk 'length($0) > 1024 { bad = 1 } END { exit bad }' "$t/ssh.log" || fail "an ssh command longer than 1024 bytes"
grep -q 'base64\|sh -c' "$t/ssh.log" && fail "the script travelled as a command-line argument"
grep -q "^mktemp -d $t/board-tmp/vi95-day-one" "$t/ssh.log" || fail "no fresh directory under the board's /tmp"
grep -q "^ sh $t/board-tmp/vi95-day-one\.[A-Za-z0-9]*/day_one.sh --readlist " "$t/ssh.log" || fail "day_one.sh not run from the board's /tmp"
tail -n 1 "$t/ssh.log" | grep -q "^rm -rf $t/board-tmp/vi95-day-one" || fail "the copy was not removed last"
[ -z "$(ls -A "$t/board-tmp")" ] || fail "run_day_one.sh left files in the board's /tmp"
# an argument the remote shell would read is refused before any ssh
: > "$t/ssh.log"
PATH="$b:/usr/bin:/bin" VI95_SSH=board VI95_REMOTE_TMP="$t/board-tmp" \
	sh "$here/run_day_one.sh" --root "$r; reboot" > /dev/null 2>&1 && fail "a ';' argument was taken"
[ ! -s "$t/ssh.log" ] || fail "ssh ran for a refused argument"

echo "day_one selftest: ok"

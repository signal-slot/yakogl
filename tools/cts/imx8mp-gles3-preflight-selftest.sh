#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# imx8mp-gles3-preflight-selftest.sh - selftest of imx8mp-gles3-preflight.sh on a fake layout of empty stub files
# under $TMPDIR (removed at the end): all good, no stage 1, no image, wrong commit, BOARD_UART unset, and that
# BOARD_UART's value never appears in the output; the Armadillo-X2 mode (--board x2: the image under <out>/x2, the
# watchdog marker, the kexec tools, the board check through a stub ssh); and imx8mp-gles3-board.sh's board choice
# (dry runs of tools/board.sh imx8mp and x2, stubs record any board access).
#   TMPDIR=<dir> tools/cts/imx8mp-gles3-preflight-selftest.sh
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
: "${TMPDIR:?set TMPDIR to a directory}"
case $(cd "$TMPDIR" && pwd -P) in /tmp|/tmp/*|/var/tmp|/var/tmp/*) echo "TMPDIR must not be under /tmp" >&2; exit 2 ;; esac
W=$(mktemp -d "$TMPDIR/preflight.XXXXXX") || exit 2
trap 'rm -rf "$W"' EXIT
fails=0
SECRET=ttyUSB-selftest-secret-7
SHA=0123456789abcdef0123456789abcdef01234567
rc=0

mk() { # mk <dir>: a fake layout of empty stub files
	mkdir -p "$1/out/es3-01234567/zephyr" "$1/stage1/build/modules/gles3" "$1/ws" "$1/hal"
	: > "$1/out/es3-01234567/zephyr/zephyr.bin"
	: > "$1/stage1/build/modules/gles3/libdeqp-gles3-package.a"
}
run() { # run <dir> <uart> args...: output in $W/log, status in $rc
	local d=$1 uart=$2; shift 2
	local e=(PATH="$PATH" HOME="$W" IMX8MP_OUT="$d/out" DEQP_STAGE1="$d/stage1" ZEPHYR_WS="$d/ws" HAL_NXP_DIR="$d/hal")
	if [ -n "$uart" ]; then e+=(BOARD_UART="$uart"); fi
	env -i "${e[@]}" bash "$HERE/imx8mp-gles3-preflight.sh" "$@" > "$W/log" 2>&1
	rc=$?
}
expect() { # expect <name> <want rc> <pattern>
	local good=1
	[ "$rc" -eq "$2" ] || good=0
	grep -q -- "$3" "$W/log" || good=0
	if grep -q -- "$SECRET" "$W/log"; then good=0; echo "  FAIL BOARD_UART value in output"; fi
	if [ $good -eq 1 ]; then echo "  ok   $1"
	else echo "  FAIL $1 (rc=$rc)"; sed 's/^/       /' "$W/log"; fails=$((fails + 1)); fi
}

mk "$W/a"
run "$W/a" "$SECRET" --commit "$SHA"
expect "all good" 0 "preflight: all ok"
grep -q "ok   BOARD_UART is set" "$W/log" || { echo "  FAIL BOARD_UART reported as set"; fails=$((fails + 1)); }
run "$W/a" "$SECRET" --commit 01234567
expect "8-digit commit prefix" 0 "preflight: all ok"

mk "$W/b"; rm "$W/b/stage1/build/modules/gles3/libdeqp-gles3-package.a"
run "$W/b" "$SECRET" --commit "$SHA"
expect "missing stage 1" 1 "FAIL stage 1 package missing"

mk "$W/c"; rm -r "$W/c/out/es3-01234567"
run "$W/c" "$SECRET" --commit "$SHA"
expect "missing image" 1 "FAIL no table image"

mk "$W/d"
run "$W/d" "$SECRET" --commit fedcba9876543210
expect "wrong commit" 1 "FAIL no image is named after commit fedcba98"

mk "$W/e"
run "$W/e" "" --commit "$SHA"
expect "BOARD_UART unset" 1 "FAIL BOARD_UART is unset"

run "$W/a" "$SECRET" --commit "$SHA" --table "$W/nonexistent"
expect "missing table" 1 "FAIL table file not found"

# X2 images live in <out>/x2/es3-<hash8> (imx8mp-env.sh): a newer one next to the Verdin's is never picked up,
# neither by the preflight nor by the glob imx8mp-gles3-board.sh takes its default image from
mk "$W/f"; mkdir -p "$W/f/out/x2/es3-fedcba98/zephyr"; : > "$W/f/out/x2/es3-fedcba98/zephyr/zephyr.bin"
sleep 1; touch "$W/f/out/x2/es3-fedcba98/zephyr/zephyr.bin"
run "$W/f" "$SECRET" --commit "$SHA"
expect "newer X2 image beside the Verdin's" 0 "table image exists (es3-01234567)"
run "$W/f" "$SECRET" --commit fedcba98
expect "X2 image is not a Verdin image" 1 "FAIL no image is named after commit fedcba98"
pick=$(ls -t "$W"/f/out/es3-*/zephyr/zephyr.bin 2>/dev/null | head -1)
if [ "$pick" = "$W/f/out/es3-01234567/zephyr/zephyr.bin" ]; then echo "  ok   board script's glob picks the Verdin image"
else echo "  FAIL board script's glob picked '$pick'"; fails=$((fails + 1)); fi

# /boot medium: a stub ssh on PATH answers with fixed read-out lines (no board is used)
mkdir -p "$W/bin"
cat > "$W/bin/ssh" <<'STUB'
#!/bin/sh
cat "$STUB_PROBE"
STUB
chmod +x "$W/bin/ssh"
probe() { # probe <name> <want rc> <pattern> <lines...>: run with BOARD_HOST set and the stub's answer
	local name=$1 want=$2 pat=$3; shift 3
	printf '%s\n' "$@" probe=done > "$W/probe.txt"
	env -i PATH="$W/bin:$PATH" HOME="$W" IMX8MP_OUT="$W/a/out" DEQP_STAGE1="$W/a/stage1" ZEPHYR_WS="$W/a/ws" \
		HAL_NXP_DIR="$W/a/hal" BOARD_UART="$SECRET" BOARD_HOST=bootsecrethost STUB_PROBE="$W/probe.txt" \
		bash "$HERE/imx8mp-gles3-preflight.sh" --commit "$SHA" > "$W/log" 2>&1
	rc=$?
	if grep -q bootsecrethost "$W/log"; then echo "  FAIL BOARD_HOST in output"; fails=$((fails + 1)); fi
	expect "$name" "$want" "$pat"
}
probe "SD /boot passes" 0 "ok   /boot is /dev/mmcblk1p1.*SD card mmcblk1" \
	"mnt=/dev/mmcblk1p1 /boot" plabel=boot disk=mmcblk1 type=SD boot0=no fwenv=/dev/mmcblk2
grep -q "info U-Boot env device.*mmcblk2" "$W/log" || { echo "  FAIL env device info line"; fails=$((fails + 1)); }
probe "eMMC /boot fails" 1 "FAIL /boot (/dev/mmcblk2p1) is on the eMMC disk mmcblk2.*person's decision" \
	"mnt=/dev/mmcblk2p1 /boot" plabel=boot disk=mmcblk2 type=MMC boot0=yes fwenv=/dev/mmcblk2
probe "eMMC without type fails on boot0" 1 "FAIL .*on the eMMC disk" \
	"mnt=/dev/mmcblk2p1 /boot" plabel= disk=mmcblk2 type= boot0=yes
for pl in fsbla1 fsbla2 metadata1 metadata2 fip-a fip-b; do
	probe "protected partlabel $pl fails" 1 "FAIL .*protected microSD partition '$pl'" \
		"mnt=/dev/mmcblk1p3 /boot" plabel=$pl disk=mmcblk1 type=SD boot0=no
done
probe "no separate /boot fails" 1 "FAIL /boot is not a separate mount" mnt= fwenv=/dev/mmcblk2
run "$W/a" "$SECRET" --commit "$SHA"
expect "BOARD_HOST unset: check skipped" 0 "note /boot medium not checked: BOARD_HOST is unset"
printf '' > "$W/probe.txt"
env -i PATH="$W/bin:$PATH" HOME="$W" IMX8MP_OUT="$W/a/out" DEQP_STAGE1="$W/a/stage1" ZEPHYR_WS="$W/a/ws" \
	HAL_NXP_DIR="$W/a/hal" BOARD_UART="$SECRET" BOARD_HOST=x STUB_PROBE="$W/probe.txt" \
	bash "$HERE/imx8mp-gles3-preflight.sh" --commit "$SHA" > "$W/log" 2>&1; rc=$?
expect "unreachable board keeps the old behaviour" 0 "note /boot medium not checked: the board did not answer"

if [ "$(find "$W/a" -type f | wc -l)" -eq 2 ]; then echo "  ok   read-only"
else echo "  FAIL read-only"; fails=$((fails + 1)); fi

# ---- the Armadillo-X2 mode (--board x2 or IMX8MP_BOARD=x2): image under <out>/x2, the watchdog marker, the two
# kexec tools, BOARD_UART optional, the board check (model, /tmp tmpfs) instead of the /boot guard
mkx2() { # mkx2 <dir>: the fake layout plus an X2 image with the marker and the two tools
	mk "$1"
	mkdir -p "$1/out/x2/es3-01234567/zephyr" "$1/kz"
	printf 'xx\0gc_wdog_arm: armed\0' > "$1/out/x2/es3-01234567/zephyr/zephyr.bin"
	for t in kexec_zephyr snvs_lpgpr; do printf '#!/bin/sh\nexit 99\n' > "$1/kz/$t"; chmod +x "$1/kz/$t"; done
}
x2run() { # x2run <dir> <uart> <host or ''> args...: --board x2 unless args say otherwise; output in $W/log
	local d=$1 uart=$2 host=$3; shift 3
	local e=(PATH="$W/bin:$PATH" HOME="$W" IMX8MP_OUT="$d/out" DEQP_STAGE1="$d/stage1" ZEPHYR_WS="$d/ws"
		HAL_NXP_DIR="$d/hal" X2_TOOLS="$d/kz" STUB_PROBE="$W/probe.txt")
	if [ -n "$uart" ]; then e+=(BOARD_UART="$uart"); fi
	if [ -n "$host" ]; then e+=(BOARD_HOST="$host"); fi
	env -i "${e[@]}" ${X2ENV:-} bash "$HERE/imx8mp-gles3-preflight.sh" "$@" > "$W/log" 2>&1
	rc=$?
	if grep -q x2secrethost "$W/log"; then echo "  FAIL BOARD_HOST in output"; fails=$((fails + 1)); fi
}
mkx2 "$W/x"
x2run "$W/x" "" "" --board x2 --commit "$SHA"
expect "x2: all good without BOARD_UART and BOARD_HOST" 0 "preflight: all ok"
for p in "note BOARD_UART is unset: the X2 console is optional" "ok   x2 tool driver/tools/kexec-zephyr/kexec_zephyr is built" \
	"ok   x2 tool driver/tools/kexec-zephyr/snvs_lpgpr is built" "ok   table image exists (es3-01234567)" \
	"carries 'gc_wdog_arm: armed'" "note the board not checked: BOARD_HOST is unset" \
	"info board x2: measured on the Armadillo-X2 (i.MX 8M Plus, GC7000UL; tools/board.sh x2, crash-route kexec)"; do
	grep -qF "$p" "$W/log" || { echo "  FAIL x2 line missing: $p"; fails=$((fails + 1)); }
done
grep -q '/boot medium not checked' "$W/log" && { echo "  FAIL x2 ran the Verdin /boot guard"; fails=$((fails + 1)); }
X2ENV=IMX8MP_BOARD=x2 x2run "$W/x" "$SECRET" "" --commit "$SHA"
expect "x2: IMX8MP_BOARD=x2 is --board x2, BOARD_UART set" 0 "ok   BOARD_UART is set"
grep -q 'measured on the Armadillo-X2' "$W/log" || { echo "  FAIL IMX8MP_BOARD=x2: no X2 board line"; fails=$((fails + 1)); }
grep -q "carries 'gc_wdog_arm" "$W/log" || { echo "  FAIL IMX8MP_BOARD=x2 did not pick the x2 mode"; fails=$((fails + 1)); }
mkx2 "$W/x1"; rm "$W/x1/kz/snvs_lpgpr"
x2run "$W/x1" "" "" --board x2 --commit "$SHA"
expect "x2: snvs_lpgpr not built" 1 "FAIL x2 tool driver/tools/kexec-zephyr/snvs_lpgpr is not built"
mkx2 "$W/x2"; printf 'no marker here' > "$W/x2/out/x2/es3-01234567/zephyr/zephyr.bin"
x2run "$W/x2" "" "" --board x2 --commit "$SHA"
expect "x2: image without the watchdog marker" 1 "FAIL image es3-01234567 lacks 'gc_wdog_arm: armed'"
mkx2 "$W/x3"; rm -r "$W/x3/out/x2"
x2run "$W/x3" "" "" --board x2 --commit "$SHA"
expect "x2: a Verdin image is not an X2 image" 1 "FAIL no table image <out>/x2/es3-<hash8>/zephyr/zephyr.bin"
x2run "$W/x" "" "" --board x2 --commit fedcba98
expect "x2: wrong commit" 1 "FAIL no image is named after commit fedcba98"
x2run "$W/x" "" "" --board pi4
expect "unknown board: usage error" 2 "is imx8mp or x2"
x2probe() { # x2probe <name> <want rc> <pattern> <lines...>
	local name=$1 want=$2 pat=$3; shift 3
	printf '%s\n' "$@" probe=done > "$W/probe.txt"
	x2run "$W/x" "" x2secrethost --board x2 --commit "$SHA"
	expect "$name" "$want" "$pat"
}
x2probe "x2 board: the Armadillo-X2 with a tmpfs /tmp passes" 0 "ok   the board is the Armadillo-X2 (model 'Atmark Techno Armadillo X2'" \
	"model=Atmark Techno Armadillo X2" "compat=atmark,armadillo-x2 fsl,imx8mp " tmpfs=tmpfs "vartmp=ext4 /dev/mmcblk2p4 at /var/tmp"
for p in "ok   /tmp is a tmpfs" "note /boot medium guard does not apply on the X2" \
	"info /var/tmp (tools/board.sh's board lease file) is on: ext4 /dev/mmcblk2p4 at /var/tmp"; do
	grep -qF "$p" "$W/log" || { echo "  FAIL x2 probe line missing: $p"; fails=$((fails + 1)); }
done
grep -q 'FAIL.*/boot' "$W/log" && { echo "  FAIL x2: the /boot guard failed the X2"; fails=$((fails + 1)); }
x2probe "x2 board: the Verdin is not the X2" 1 "FAIL the board is 'Toradex Verdin iMX8M Plus.*', not the Armadillo-X2" \
	"model=Toradex Verdin iMX8M Plus WB on Verdin Development Board" "compat=toradex,verdin-imx8mp fsl,imx8mp" tmpfs=tmpfs
x2probe "x2 board: /tmp not a tmpfs" 1 "FAIL /tmp is 'ext4', not a tmpfs" \
	"model=Atmark Techno Armadillo X2" "compat=fsl,imx8mp" tmpfs=ext4
x2probe "x2 board: /tmp not a mount" 1 "FAIL /tmp is 'unknown', not a tmpfs" \
	"model=Atmark Techno Armadillo X2" "compat=fsl,imx8mp" tmpfs=
x2probe "x2 board: model unreadable" 1 "FAIL the board's device-tree model is unreadable" model= compat= tmpfs=tmpfs
x2probe "x2 board: an X2 model without fsl,imx8mp" 1 "FAIL model 'Armadillo-X2' but no fsl,imx8mp" \
	model=Armadillo-X2 "compat=atmark,armadillo-x2" tmpfs=tmpfs
printf '' > "$W/probe.txt"
x2run "$W/x" "" x2secrethost --board x2 --commit "$SHA"
expect "x2 board: no answer is a note" 0 "note the board not checked: the board did not answer"
if [ "$(find "$W/x" -type f | wc -l)" -eq 5 ]; then echo "  ok   x2: read-only"
else echo "  FAIL x2: read-only"; fails=$((fails + 1)); fi

# ---- imx8mp-gles3-board.sh: the board choice reaches tools/board.sh (dry run, one row, stubs record any access)
mkdir -p "$W/stub"
for c in ssh scp curl ping fuser flock stty sudo kexec; do
	printf '#!/bin/sh\necho "%s $*" >> "%s/touched"\nexit 99\n' "$c" "$W" > "$W/stub/$c"; chmod +x "$W/stub/$c"
done
mkimg() { # mkimg FILE: an AArch64 Image header (4 MiB footprint) and the watchdog marker (board_selftest.sh's form)
	{ head -c 16 /dev/zero; printf '\x00\x00\x40\x00\x00\x00\x00\x00'; head -c 32 /dev/zero; printf 'ARMd'
	  head -c 1000 /dev/zero; printf 'gc_wdog_arm: armed\0'; } > "$1"
}
bwrap() { # bwrap <env...> -- <args...>: imx8mp-gles3-board.sh's output in $W/log
	local e=()
	while [ "$1" != -- ]; do e+=("$1"); shift; done; shift
	env -i PATH="$W/stub:$PATH" HOME="$W" TMPDIR="$TMPDIR" TREE_ROWS=0 "${e[@]}" \
		bash "$HERE/imx8mp-gles3-board.sh" "$@" > "$W/log" 2>&1
	rc=$?
}
: > "$W/any.bin"
bwrap IMX8MP_BOARD=x2 TREE_IMAGE="$W/any.bin" -- --dry-run tree "$W/tree"
expect "board.sh wrapper: IMX8MP_BOARD=x2 runs tools/board.sh --dry-run x2 tree" 0 \
	"imx8mp-gles3-board: tools/board.sh --dry-run x2 tree tools/cts/imx8mp-gles3-tree.txt"
grep -q '^board.sh x2: tree: 133 rows from .*imx8mp-gles3-tree.txt' "$W/log" \
	&& grep -q '^DRY host\$ tools/board.sh x2 tally ' "$W/log" && echo "  ok   board.sh x2 read the 133-row table, tally named" \
	|| { echo "  FAIL board.sh x2 tree output"; sed 's/^/       /' "$W/log"; fails=$((fails + 1)); }
mkdir -p "$W/g/out/x2/es3-fedcba98/zephyr" "$W/g/out/es3-01234567/zephyr"
mkimg "$W/g/out/x2/es3-fedcba98/zephyr/zephyr.bin"; : > "$W/g/out/es3-01234567/zephyr/zephyr.bin"
touch "$W/g/out/es3-01234567/zephyr/zephyr.bin"   # the Verdin's is the newest: never the X2's default
bwrap IMX8MP_OUT="$W/g/out" -- --dry-run --board x2 tree "$W/tree"
expect "board.sh wrapper: --board x2 defaults to the newest <out>/x2/es3-* image" 0 "kexec_zephyr /tmp/yakogl-kexec/zephyr.bin 0x60000000 --crash"
grep -q 'no AArch64 Image header' "$W/log" && { echo "  FAIL --board x2 took the Verdin image"; fails=$((fails + 1)); }
bwrap TREE_IMAGE="$W/any.bin" -- --dry-run tree "$W/tree"
expect "board.sh wrapper: the default board stays imx8mp" 0 "imx8mp-gles3-board: tools/board.sh --dry-run imx8mp tree"
grep -q '^board.sh imx8mp: tree: 133 rows' "$W/log" || { echo "  FAIL default board is not imx8mp"; fails=$((fails + 1)); }
bwrap TREE_IMAGE="$W/any.bin" -- --board am62p tree "$W/tree"
expect "board.sh wrapper: another board is a usage error" 2 "the board is imx8mp or x2, not 'am62p'"
if [ ! -e "$W/tree" ] && [ ! -s "$W/touched" ]; then echo "  ok   board.sh wrapper dry runs: nothing written, no stub ran"
else echo "  FAIL board.sh wrapper dry runs touched something"; fails=$((fails + 1)); fi

if [ $fails -eq 0 ]; then echo "preflight selftest: all ok"; exit 0; fi
echo "preflight selftest: $fails FAILED"
exit 1

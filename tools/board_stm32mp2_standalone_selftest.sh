#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# board_stm32mp2_standalone_selftest.sh - tools/board-stm32mp2-standalone.sh
# without a board. ssh, scp and sudo are stubs on PATH that only record that
# they were called; none may run. Checks: every verb under --dry-run (exit 0,
# "DRY board" lines, the undo before any board line), the refusals (forever
# without a person, a bad count, a bad image, a live write while the payload
# is not wired, a missing BOARD_HOST), the write guard (guard path and guard
# name reject spaces, newlines, ; ' $ ` and a path outside the stage
# directory; dev_guard refuses the eMMC and every never-written label), and
# that every remote rm, mv and tee names the stage directory.
#
#   tools/board_stm32mp2_standalone_selftest.sh
#
# Prints "board_stm32mp2_standalone_selftest: N checks, F failed"; exit 0
# only when F is 0.
set -uo pipefail
export LC_ALL=C
HERE=$(cd "$(dirname "$0")" && pwd)
R="$HERE/board-stm32mp2-standalone.sh"
W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
mkdir "$W/bin"
for s in ssh scp sudo; do
    printf '#!/bin/sh\necho "%s $*" >> "%s/stub.log"\nexit 1\n' "$s" "$W" > "$W/bin/$s"
    chmod +x "$W/bin/$s"
done
export PATH="$W/bin:$PATH"
unset BOARD_HOST BOARD_PERSON_PRESENT MP2_NO_WATCHDOG MP2_SUDO MP2_KNOWN_HOSTS

N=0
F=0
ok() { N=$((N + 1)); echo "ok   $1"; }
bad() { N=$((N + 1)); F=$((F + 1)); echo "FAIL $1: $2"; }
# expect NAME RC CMD...: run, check the exit code; the output stays in $OUT
expect() {
    local name=$1 want=$2 rc
    shift 2
    OUT=$("$@" 2>&1)
    rc=$?
    if [ "$rc" = "$want" ]; then ok "$name"; else bad "$name" "rc $rc, wanted $want: $OUT"; fi
}
has() { if grep -qF -e "$2" <<<"$OUT"; then ok "$1"; else bad "$1" "no '$2' in: $OUT"; fi; }
hasnt() { if grep -qF -e "$2" <<<"$OUT"; then bad "$1" "'$2' in: $OUT"; else ok "$1"; fi; }

# fake images: AArch64 Image magic at byte 56, with and without the marker
IMG=$W/img.bin
{ head -c 56 /dev/zero; printf 'ARMd'; head -c 4000 /dev/zero; printf 'gc_wdog_arm: armed'; } > "$IMG"
NOWD=$W/nowd.bin
{ head -c 56 /dev/zero; printf 'ARMd'; head -c 4000 /dev/zero; } > "$NOWD"
BADHDR=$W/badhdr.bin
head -c 4096 /dev/zero > "$BADHDR"
BIG=$W/big.bin
{ head -c 56 /dev/zero; printf 'ARMd'; } > "$BIG"
truncate -s 300M "$BIG"

# 1. dry runs
for v in status install uninstall nodefault; do
    expect "dry-run $v" 0 "$R" --dry-run "$v"
    has "dry-run $v prints board lines" 'DRY board$'
done
expect "dry-run default 2" 0 "$R" --dry-run default "$IMG" 2
has "default prints the undo" 'UNDO, from Linux'
has "default 2: no undo needed" 'none needed'
undo_ln=$(grep -n UNDO <<<"$OUT" | head -1 | cut -d: -f1)
dry_ln=$(grep -n 'DRY board' <<<"$OUT" | head -1 | cut -d: -f1)
if [ "$undo_ln" -lt "$dry_ln" ]; then ok "the undo comes before the first board line"; else bad "undo first" "$OUT"; fi
has "default 2 says nothing was rebooted" 'nothing was rebooted'
hasnt "default 2 never names the eMMC" mmcblk0
hasnt "default never touches bootcmd" 'bootcmd'
hasnt "default never touches the bootcount" 'bootcount'
expect "dry-run forever with a person" 0 env BOARD_PERSON_PRESENT=1 "$R" --dry-run default "$IMG" forever
has "forever prints the person's undo" 'setenv mp2s_left 0; saveenv; boot'
expect "dry-run nodefault" 0 "$R" --dry-run nodefault
has "nodefault removes the fixed default.bin" 'default.bin'
expect "dry-run with MP2_NO_WATCHDOG" 0 env MP2_NO_WATCHDOG=1 "$R" --dry-run default "$NOWD" 1

# 2. refusals
expect "forever without a person" 2 "$R" --dry-run default "$IMG" forever
has "forever says why" 'BOARD_PERSON_PRESENT=1'
expect "forever with person=0" 2 env BOARD_PERSON_PRESENT=0 "$R" --dry-run default "$IMG" forever
for n in 0 10 -1 x ''; do expect "count '$n' refused" 2 "$R" --dry-run default "$IMG" "$n"; done
expect "image without the marker" 3 "$R" --dry-run default "$NOWD" 2
expect "image without the header" 3 "$R" --dry-run default "$BADHDR" 2
expect "image over 256 MiB" 3 "$R" --dry-run default "$BIG" 2
expect "image missing" 2 "$R" --dry-run default "$W/none" 2
expect "default without args" 2 "$R" --dry-run default
expect "unknown verb" 2 "$R" --dry-run frobnicate
expect "no verb" 2 "$R"
expect "live install without BOARD_HOST" 2 "$R" install
expect "live default refuses while the payload is not wired" 3 env BOARD_HOST=u@h "$R" default "$IMG" 2
has "the refusal says nothing was changed" 'nothing was changed'
has "the undo was printed before it" 'UNDO'
for v in install uninstall nodefault; do
    expect "live $v refuses" 3 env BOARD_HOST=u@h "$R" "$v"
done
# the live runs may send reads (fw_printenv, ls); nothing that changes
if ! grep -vE " (sudo -n )?(fw_printenv|ls) " "$W/stub.log" | grep -q .; then ok "only reads reached ssh; no scp or sudo"; else bad "stubs" "$(cat "$W/stub.log")"; fi

# 3. the write guard: paths and names
S=/boot/yakogl/standalone
for p in "$S" "$S/default.bin" "$S/default.bin.part" "$S/boot.scr.uimg"; do
    expect "guard path accepts $p" 0 "$R" guard path "$p"
done
BADNAMES=('a b' $'a\nb' 'x;reboot' "x'y" 'x$y' 'x`y' 'a /etc' '-rf' '.hid' 'a/b' '..' '.' '' 'x|y' 'x&y' 'x"y' 'x\y' 'x*y' $'ok\n')
for n in "${BADNAMES[@]}"; do
    label=${n//$'\n'/\\n}
    expect "guard path rejects '$label'" 3 "$R" guard path "$S/$n"
    expect "guard name rejects '$label'" 3 "$R" guard name "$n"
done
for p in /boot /boot/yakogl "$S/" "$S/../x" /etc/passwd /boot/boot.scr.uimg /boot/mmc0_extlinux/x "${S}x/y"; do
    expect "guard path rejects $p" 3 "$R" guard path "$p"
done
expect "guard name accepts default.bin" 0 "$R" guard name default.bin
expect "guard name accepts a-b_c.1" 0 "$R" guard name a-b_c.1

# 4. the write guard: devices
TAB=$'/dev/mmcblk1p7 u-boot-env\n/dev/mmcblk1p8 bootfs\n/dev/mmcblk1p3 rootfs\n/dev/mmcblk1p4 metadata1\n/dev/mmcblk1p5 fip-a\n/dev/mmcblk0p7 u-boot-env\n/dev/mmcblk1p9 userfs'
dg() { GUARD_TABLE=$TAB GUARD_CARD=/dev/mmcblk1 "$R" guard dev "$@"; }
expect "dev_guard accepts the card's u-boot-env" 0 dg /dev/mmcblk1p7 u-boot-env
expect "dev_guard accepts the card's bootfs" 0 dg /dev/mmcblk1p8 bootfs
expect "dev_guard refuses the eMMC partition" 3 dg /dev/mmcblk0p7 u-boot-env
expect "dev_guard refuses the eMMC disk" 3 dg /dev/mmcblk0 u-boot-env
expect "dev_guard refuses a wrong label" 3 dg /dev/mmcblk1p8 u-boot-env
expect "dev_guard refuses rootfs" 3 dg /dev/mmcblk1p3 rootfs
expect "dev_guard refuses metadata1" 3 dg /dev/mmcblk1p4 metadata1
expect "dev_guard refuses fip-a" 3 dg /dev/mmcblk1p5 fip-a
expect "dev_guard refuses userfs" 3 dg /dev/mmcblk1p9 userfs
expect "dev_guard refuses a non-partition" 3 dg /dev/sda1 bootfs
expect "dev_guard refuses a device of another card" 3 dg /dev/mmcblk2p1 bootfs
expect "dev_guard refuses an unlabelled partition" 3 dg /dev/mmcblk1p2 bootfs

# 5. every remote rm, mv and tee names the stage directory
OUT=$(BOARD_PERSON_PRESENT=1 "$R" --dry-run default "$IMG" forever; "$R" --dry-run nodefault; "$R" --dry-run uninstall)
if grep 'DRY board' <<<"$OUT" | grep -E 'rm |mv |tee ' | grep -vq '/boot/yakogl/standalone'; then
    bad "rm, mv and tee name the stage directory" "$OUT"
else ok "rm, mv and tee name the stage directory"; fi

echo "board_stm32mp2_standalone_selftest: $N checks, $F failed"
[ "$F" = 0 ]

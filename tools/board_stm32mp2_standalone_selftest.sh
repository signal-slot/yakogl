#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# board_stm32mp2_standalone_selftest.sh - tools/board-stm32mp2-standalone.sh
# without a board. Checks: every verb under --dry-run (exit 0, "DRY board"
# lines, the undo before any board line, tools/mp2s_undo_order.py on the
# transcripts), the refusals (forever without a person, also for an empty
# BOARD_PERSON_PRESENT and 'yes'; a bad count, a bad image, a LIFETIME_S=0
# product build under a count, a missing BOARD_HOST), the write guard (guard
# path and guard name reject spaces, newlines, ; ' $ ` and a path outside the
# stage directory; dev_guard refuses the eMMC and every never-written label),
# that every remote rm, mv and tee names the stage directory, and
# tools/mp2s_protect_scan.py on the runner.
#
# The live verbs run against a fake board: ssh, sudo, findmnt, lsblk,
# fw_printenv, fw_setenv and dd are stubs on PATH. The fake ssh logs each command
# and runs it on a fake root in this test's temp directory ($FK/boot, $FK/dev,
# $FK/sys, $FK/proc: every /boot, /dev, /sys and /proc of the command is
# rewritten there, and a command that still names a real one is not run).
# fw_printenv and fw_setenv fail as the DK's do (no /etc/fw_env.config) and
# log any call; the runner reads $FK/dev/mmcblk1p7, a 512 KiB u-boot-env
# partition made from tools/testdata/mp2s-env/ (copy 1 at 0x7e000, copy 2 at
# 0x7c000), with head and writes one copy of it with dd, which the dd stub
# logs. No network, no board. Proven there: fw_printenv and fw_setenv are
# never called; every write is one dd of one 8 KiB copy of the card's
# u-boot-env; bootcmd, bootcount and altbootcmd keep their values; no
# command names the eMMC (mmcblk0) or a partition
# labelled fsbla1, fsbla2, metadata1, metadata2, fip-a or fip-b, and their
# fake bytes never change; a refused verb changes nothing (the fake root's
# sha256 before and after); a write that does not read back is undone.
#
# install and uninstall need tools/mp2s_boot_manifest.py (MP2S_BOOT_MANIFEST
# names another copy). While it is missing their live cases print
# "skip: mp2s_boot_manifest.py missing" and are not counted; the refusal of a
# live install and uninstall without it is checked always.
#
#   tools/board_stm32mp2_standalone_selftest.sh
#
# Prints "board_stm32mp2_standalone_selftest: N checks, F failed, S skipped";
# exit 0 only when F is 0.
set -uo pipefail
export LC_ALL=C
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
R="$HERE/board-stm32mp2-standalone.sh"
FIX="$HERE/testdata/mp2s-env"
W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
FK=$W/fk
export FK REPO
MANIFEST=${MP2S_BOOT_MANIFEST:-$HERE/mp2s_boot_manifest.py}
unset BOARD_HOST BOARD_PERSON_PRESENT MP2_NO_WATCHDOG MP2_SUDO MP2_KNOWN_HOSTS MP2S_BOOT_MANIFEST
export MP2S_STATE_DIR=$W/state BOARD_LOCK=$W/board.lock LOCK_WAIT=5

N=0
F=0
S=0
ok() { N=$((N + 1)); echo "ok   $1"; }
bad() { N=$((N + 1)); F=$((F + 1)); echo "FAIL $1: $2"; }
skip() { S=$((S + 1)); echo "skip: $1"; }
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

# ---- the stubs ------------------------------------------------------------------------
mkdir "$W/bin"
cat >"$W/bin/ssh" <<'EOF'
#!/bin/bash
# the fake board's ssh: options, the host, then the remote command
while [ $# -gt 0 ]; do case $1 in -o) shift 2 ;; -*) shift ;; *) break ;; esac; done
shift
cmd="$*"
printf '%s\n' "$cmd" >>"$FK/ssh.log"
[ -e "$FK/down" ] && exit 255
c=$(printf '%s' "$cmd" | perl -pe 's#(?<![\w.-])/(boot|dev|sys|proc)(?=/|\s|$|[;\x27"])#$ENV{FK}/$1#g')
if printf '%s' "$c" | grep -Eq "(^|[^A-Za-z0-9_.-])/(boot|dev|sys|proc|etc)(/|\$|[ ;'\"])"; then
    echo "UNSAFE: $cmd" >>"$FK/ssh.log"
    exit 99
fi
MP2S_ON_BOARD=1 exec bash -c "$c"
EOF
cat >"$W/bin/sudo" <<'EOF'
#!/bin/bash
[ "${1:-}" = -n ] && shift
exec "$@"
EOF
cat >"$W/bin/scp" <<'EOF'
#!/bin/sh
echo "scp $*" >>"$FK/ssh.log"
exit 1
EOF
printf '#!/bin/sh\ncat "$FK/findmnt.out"\n' >"$W/bin/findmnt"
printf '#!/bin/sh\ncat "$FK/lsblk.txt"\n' >"$W/bin/lsblk"
cat >"$W/bin/dd" <<'EOF'
#!/bin/bash
# dd on the fake board (run through the fake ssh): a write to the fake root
# (of=$FK/...) is logged in dd.log with its arguments, its bytes kept in
# dd-N.bin; with $FK/dd.lie it is swallowed. The test's own dd is plain dd.
of=
for a in "$@"; do case $a in of=*) of=${a#of=} ;; esac; done
case ${MP2S_ON_BOARD:-}:$of in
1:"$FK"/*)
    n=$(($(wc -l <"$FK/dd.log") + 1))
    printf '%s\n' "${*//"$FK"/}" >>"$FK/dd.log"
    cat >"$FK/dd-$n.bin"
    [ -e "$FK/dd.lie" ] && exit 0
    exec /bin/dd "$@" if="$FK/dd-$n.bin"
    ;;
esac
exec /bin/dd "$@"
EOF
for t in fw_printenv fw_setenv; do
    printf '#!/bin/sh\necho "%s: Cannot initialize environment" >&2\necho "%s $*" >>"$FK/fw.log"\nexit 1\n' "$t" "$t" >"$W/bin/$t"
done
chmod +x "$W/bin"/*
export PATH="$W/bin:$PATH"

# ---- the images -------------------------------------------------------------------------
# fake images: AArch64 Image magic at byte 56, with and without a marker
IMG=$W/img.bin
{ head -c 56 /dev/zero; printf 'ARMd'; head -c 4000 /dev/zero; printf 'gc_wdog_arm: armed'; } >"$IMG"
NOWD=$W/nowd.bin
{ head -c 56 /dev/zero; printf 'ARMd'; head -c 4000 /dev/zero; } >"$NOWD"
PROD=$W/prod.bin
{ head -c 56 /dev/zero; printf 'ARMd'; head -c 4000 /dev/zero; printf 'lifetime %%u s (PSCI SYSTEM_RESET)\0mp2_wdtr: product refresh armed'; } >"$PROD"
NOLIFE=$W/nolife.bin
{ head -c 56 /dev/zero; printf 'ARMd'; head -c 4000 /dev/zero; printf 'no lifetime of its own (product build)\0mp2_wdtr: product refresh armed'; } >"$NOLIFE"
BADHDR=$W/badhdr.bin
head -c 4096 /dev/zero >"$BADHDR"
BIG=$W/big.bin
{ head -c 56 /dev/zero; printf 'ARMd'; } >"$BIG"
truncate -s 300M "$BIG"
"$REPO/driver/tools/mp2-standalone/build.sh" "$W/build" >/dev/null || { echo "build.sh failed"; exit 1; }
SCR=$W/build/boot.scr.uimg

# ---- the fake board -------------------------------------------------------------------
ENVL=(--redundant --size 0x2000 --offset 0x7e000 --offset2 0x7c000)
# mkpart FIXTURE OUT: a 512 KiB u-boot-env partition, copy 1 at 0x7e000, copy 2 at 0x7c000
mkpart() {
    head -c 524288 /dev/zero >"$2"
    dd if="$1" of="$2" bs=8192 skip=0 seek=63 count=1 conv=notrunc status=none
    dd if="$1" of="$2" bs=8192 skip=1 seek=62 count=1 conv=notrunc status=none
}
# mkfwu OUT STATES: FWU metadata v2, active bank 0, bank_state from hex
mkfwu() {
    python3 -I -c 'import struct, sys, zlib
body = struct.pack("<IIIIHH4sI", 2, 0, 0, 32, 0, 0, bytes.fromhex(sys.argv[2]), 0)
open(sys.argv[1], "wb").write(struct.pack("<I", zlib.crc32(body)) + body + b"\xff" * 4064)' "$1" "$2"
}
# fake FIXTURE [installed]: a fresh fake board with that environment
fake() {
    rm -rf "$FK"
    mkdir -p "$FK/boot/mmc0_extlinux" "$FK/dev" "$FK/sys/block/mmcblk1/device" "$FK/sys/block/mmcblk0/device" "$FK/proc/sys/vm"
    echo "stock kernel" >"$FK/boot/Image.gz"
    echo "stock script" >"$FK/boot/boot.scr.uimg"
    echo "stock dtb" >"$FK/boot/stm32mp257f-dk.dtb"
    echo "label stock" >"$FK/boot/mmc0_extlinux/stm32mp257f-dk_extlinux.conf"
    : >"$FK/dev/null"
    : >"$FK/proc/sys/vm/drop_caches"
    echo SD >"$FK/sys/block/mmcblk1/device/type"
    echo MMC >"$FK/sys/block/mmcblk0/device/type"
    for p in 1 2 5 6; do head -c 4096 /dev/urandom >"$FK/dev/mmcblk1p$p"; done
    mkfwu "$FK/dev/mmcblk1p3" fcfcffff
    cp "$FK/dev/mmcblk1p3" "$FK/dev/mmcblk1p4"
    mkpart "$1" "$FK/dev/mmcblk1p7"
    cp "$FK/dev/mmcblk1p7" "$FK/dev/mmcblk0p7"
    printf '%s\n' "/dev/mmcblk1p1 fsbla1" "/dev/mmcblk1p2 fsbla2" "/dev/mmcblk1p3 metadata1" \
        "/dev/mmcblk1p4 metadata2" "/dev/mmcblk1p5 fip-a" "/dev/mmcblk1p6 fip-b" \
        "/dev/mmcblk1p7 u-boot-env" "/dev/mmcblk1p8 bootfs" "/dev/mmcblk1p9 vendorfs" >"$FK/lsblk.txt"
    echo /dev/mmcblk1p8 >"$FK/findmnt.out"
    if [ "${2:-}" = installed ]; then
        mkdir -p "$FK/boot/yakogl/standalone"
        cp "$SCR" "$FK/boot/yakogl/standalone/boot.scr.uimg"
    fi
    : >"$FK/ssh.log"
    : >"$FK/dd.log"
    : >"$FK/fw.log"
    rm -rf "$MP2S_STATE_DIR"
    protect_record
    stock_sum >"$W/stock.sum"
    BC0=$(env_print -n bootcmd)
    ABC0=$(env_print -n altbootcmd)
}
protect_record() { (cd "$FK" && sha256sum dev/mmcblk1p1 dev/mmcblk1p2 dev/mmcblk1p3 dev/mmcblk1p4 dev/mmcblk1p5 dev/mmcblk1p6 dev/mmcblk0p7) >"$W/protected.sum"; }
stock_sum() { (cd "$FK/boot" && sha256sum Image.gz boot.scr.uimg stm32mp257f-dk.dtb mmc0_extlinux/stm32mp257f-dk_extlinux.conf); }
# snap: every byte of the fake board but its logs
snap() { (cd "$FK" && find boot dev -type f ! -path dev/null -print0 | sort -z | xargs -0 sha256sum) }
env_print() { python3 -I "$REPO/tools/uboot_env.py" "${ENVL[@]}" print "$FK/dev/mmcblk1p7" "$@" 2>/dev/null; }
# others: the environment without the four names the runner may write
others() { env_print | grep -Ev '^(mp2s_ver|mp2s_left|mp2s_prefixes|boot_prefixes)='; }
live() { env BOARD_HOST=u@fake "$R" "$@"; }

# renumber: the card becomes mmcblk0 and the eMMC mmcblk1, as the DK's Linux
# numbered them on 2026-11-22 (the card was mmcblk2 on the boot before)
renumber() {
    local p
    mv "$FK/dev/mmcblk0p7" "$W/emmc-env.bin"
    for p in 1 2 3 4 5 6 7; do mv "$FK/dev/mmcblk1p$p" "$FK/dev/mmcblk0p$p"; done
    mv "$W/emmc-env.bin" "$FK/dev/mmcblk1p7"
    echo SD >"$FK/sys/block/mmcblk0/device/type"
    echo MMC >"$FK/sys/block/mmcblk1/device/type"
    sed -i 's#/dev/mmcblk1p#/dev/mmcblk0p#' "$FK/lsblk.txt"
    echo /dev/mmcblk0p8 >"$FK/findmnt.out"
}
# board_cmds: the commands sent, without the reads of a disk's sysfs type
# (reading the eMMC's type is how it is told apart, never a write)
board_cmds() { grep -Ev '^cat -- /sys/block/mmcblk[0-9]+/device/type$' "$FK/ssh.log"; }

# the invariants of every live run: protected names and partitions
protected() {
    local what=$1
    if (cd "$FK" && sha256sum -c --quiet "$W/protected.sum" >/dev/null 2>&1); then ok "$what: fsbla1/2, metadata1/2, fip-a/b and the eMMC unchanged"; else bad "$what: protected bytes" "changed"; fi
    if board_cmds | grep -Eq 'mmcblk0|fsbla|metadata|fip-'; then bad "$what: no command names the eMMC or a protected partition" "$(board_cmds | grep -E 'mmcblk0|fsbla|metadata|fip-')"; else ok "$what: no command names the eMMC or a protected partition"; fi
    if grep -q UNSAFE "$FK/ssh.log"; then bad "$what: every command stayed on the fake root" "$(grep UNSAFE "$FK/ssh.log")"; else ok "$what: every command stayed on the fake root"; fi
    if [ -s "$FK/fw.log" ]; then bad "$what: fw_printenv and fw_setenv are never called" "$(cat "$FK/fw.log")"; else ok "$what: fw_printenv and fw_setenv are never called"; fi
    if grep -Evq '^of=/dev/mmcblk1p7 bs=8192 seek=6[23] count=1 iflag=fullblock conv=notrunc,fsync status=none$' "$FK/dd.log"; then bad "$what: every write is one 8 KiB copy of the card's u-boot-env" "$(cat "$FK/dd.log")"; else ok "$what: every write is one 8 KiB copy of the card's u-boot-env"; fi
    if [ "$(env_print -n bootcmd)" = "$BC0" ] && [ "$(env_print -n altbootcmd)" = "$ABC0" ] && ! env_print bootcount >/dev/null; then ok "$what: bootcmd, altbootcmd as before, no bootcount"; else bad "$what: bootcmd, bootcount, altbootcmd" "changed"; fi
}

# 1. dry runs
for v in status printenv install uninstall nodefault; do
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
has "default 2 writes one copy of the environment with dd" 'dd of=/dev/mmcblk1p7 bs=8192 seek='
hasnt "default 2 never calls fw_setenv" 'fw_setenv'
hasnt "default 2 never calls fw_printenv" 'fw_printenv'
has "default 2 writes mp2s_left=2 in that one write" 'mp2s_left=2'
hasnt "default 2 never names the eMMC" mmcblk0
hasnt "default never touches bootcmd" 'bootcmd'
hasnt "default never touches the bootcount" 'bootcount'
hasnt "default never touches altbootcmd" 'altbootcmd'
for l in fsbla1 fsbla2 metadata1 metadata2 fip-a fip-b; do hasnt "default 2 never names $l" "$l"; done
printf '%s\n' "$OUT" >"$W/dry-default.txt"
if python3 -I "$HERE/mp2s_undo_order.py" "$W/dry-default.txt" >"$W/undo.out" 2>&1; then ok "default 2's dry run passes mp2s_undo_order.py"; else bad "undo order of default 2" "$(cat "$W/undo.out")"; fi
expect "dry-run forever with a person" 0 env BOARD_PERSON_PRESENT=1 "$R" --dry-run default "$PROD" forever
has "forever prints the person's undo" 'setenv mp2s_left 0; saveenv; boot'
has "forever names the card undo" 'tools/mp2s_card_undo.sh'
printf '%s\n' "$OUT" >"$W/dry-forever.txt"
if python3 -I "$HERE/mp2s_undo_order.py" "$W/dry-forever.txt" >"$W/undo.out" 2>&1; then ok "forever's dry run passes mp2s_undo_order.py"; else bad "undo order of forever" "$(cat "$W/undo.out")"; fi
for v in install nodefault uninstall; do
    OUT=$("$R" --dry-run "$v" 2>&1)
    printf '%s\n' "$OUT" >"$W/dry-$v.txt"
    if python3 -I "$HERE/mp2s_undo_order.py" "$W/dry-$v.txt" >"$W/undo.out" 2>&1; then ok "$v's dry run passes mp2s_undo_order.py"; else bad "undo order of $v" "$(cat "$W/undo.out")"; fi
done
if [ -f "$MANIFEST" ]; then
    expect "dry-run install with the manifest tool" 0 env MP2S_BOOT_MANIFEST="$MANIFEST" "$R" --dry-run install
else
    expect "dry-run install without the manifest tool still prints the plan" 0 "$R" --dry-run install
    has "it says the tool is missing" 'skip: mp2s_boot_manifest.py missing'
    has "the plan reaches the staged script" 'boot.scr.uimg'
fi
expect "dry-run nodefault" 0 "$R" --dry-run nodefault
has "nodefault removes the fixed default.bin" 'default.bin'
expect "dry-run with MP2_NO_WATCHDOG" 0 env MP2_NO_WATCHDOG=1 "$R" --dry-run default "$NOWD" 1
expect "dry-run forever of a LIFETIME_S=0 product build with a person" 0 env BOARD_PERSON_PRESENT=1 "$R" --dry-run default "$NOLIFE" forever
expect "dry-run count of a product build with a lifetime" 0 "$R" --dry-run default "$PROD" 3
if grep -qx 'PAYLOAD_READY=1' "$R"; then ok "the payload is wired (PAYLOAD_READY=1)"; else bad "PAYLOAD_READY" "not 1"; fi

# 2. refusals before the board
expect "forever without a person" 2 env -u BOARD_PERSON_PRESENT "$R" --dry-run default "$IMG" forever
has "forever says why" 'BOARD_PERSON_PRESENT=1'
for p in 0 '' yes YES true 2; do
    expect "forever with BOARD_PERSON_PRESENT='$p'" 2 env BOARD_PERSON_PRESENT="$p" "$R" --dry-run default "$IMG" forever
    has "it names BOARD_PERSON_PRESENT ('$p')" 'BOARD_PERSON_PRESENT'
done
for n in 0 10 -1 x '' 01 ' 2'; do expect "count '$n' refused" 2 "$R" --dry-run default "$IMG" "$n"; done
expect "image without the marker" 3 "$R" --dry-run default "$NOWD" 2
expect "image without the header" 3 "$R" --dry-run default "$BADHDR" 2
expect "image over 256 MiB" 3 "$R" --dry-run default "$BIG" 2
expect "image missing" 2 "$R" --dry-run default "$W/none" 2
expect "forever: an image without the product marker" 3 env BOARD_PERSON_PRESENT=1 "$R" --dry-run default "$IMG" forever
has "it names the product marker" 'mp2_wdtr: product refresh armed'
expect "a LIFETIME_S=0 build under a count" 3 "$R" --dry-run default "$NOLIFE" 2
has "it says why" 'LIFETIME_S=0'
expect "a LIFETIME_S=0 build under a count, MP2_NO_WATCHDOG=1" 3 env MP2_NO_WATCHDOG=1 "$R" --dry-run default "$NOLIFE" 9
expect "default without args" 2 "$R" --dry-run default
expect "unknown verb" 2 "$R" --dry-run frobnicate
expect "no verb" 2 "$R"
expect "live install without BOARD_HOST" 2 "$R" install

# 3. the write guard: paths and names
S_=/boot/yakogl/standalone
for p in "$S_" "$S_/default.bin" "$S_/default.bin.part" "$S_/boot.scr.uimg"; do
    expect "guard path accepts $p" 0 "$R" guard path "$p"
done
BADNAMES=('a b' $'a\nb' 'x;reboot' "x'y" 'x$y' 'x`y' 'a /etc' '-rf' '.hid' 'a/b' '..' '.' '' 'x|y' 'x&y' 'x"y' 'x\y' 'x*y' $'ok\n')
for n in "${BADNAMES[@]}"; do
    label=${n//$'\n'/\\n}
    expect "guard path rejects '$label'" 3 "$R" guard path "$S_/$n"
    expect "guard name rejects '$label'" 3 "$R" guard name "$n"
done
for p in /boot /boot/yakogl "$S_/" "$S_/../x" /etc/passwd /boot/boot.scr.uimg /boot/mmc0_extlinux/x "${S_}x/y"; do
    expect "guard path rejects $p" 3 "$R" guard path "$p"
done
expect "guard name accepts default.bin" 0 "$R" guard name default.bin
expect "guard name accepts a-b_c.1" 0 "$R" guard name a-b_c.1

# 4. the write guard: devices
TAB=$'/dev/mmcblk1p7 u-boot-env\n/dev/mmcblk1p8 bootfs\n/dev/mmcblk1p3 rootfs\n/dev/mmcblk1p4 metadata1\n/dev/mmcblk1p5 fip-a\n/dev/mmcblk0p7 u-boot-env\n/dev/mmcblk1p9 userfs\n/dev/mmcblk1p1 fsbla1\n/dev/mmcblk1p2 fsbla2\n/dev/mmcblk1p6 fip-b\n/dev/mmcblk1p10 metadata2'
dg() { GUARD_TABLE=$TAB GUARD_CARD=/dev/mmcblk1 "$R" guard dev "$@"; }
expect "dev_guard accepts the card's u-boot-env" 0 dg /dev/mmcblk1p7 u-boot-env
expect "dev_guard accepts the card's bootfs" 0 dg /dev/mmcblk1p8 bootfs
expect "dev_guard refuses the eMMC partition" 3 dg /dev/mmcblk0p7 u-boot-env
expect "dev_guard refuses the eMMC disk" 3 dg /dev/mmcblk0 u-boot-env
expect "dev_guard refuses a wrong label" 3 dg /dev/mmcblk1p8 u-boot-env
expect "dev_guard refuses rootfs" 3 dg /dev/mmcblk1p3 rootfs
for d in "p1 fsbla1" "p2 fsbla2" "p4 metadata1" "p10 metadata2" "p5 fip-a" "p6 fip-b"; do
    set -- $d
    expect "dev_guard refuses $2" 3 dg "/dev/mmcblk1$1" "$2"
done
expect "dev_guard refuses userfs" 3 dg /dev/mmcblk1p9 userfs
expect "dev_guard refuses a non-partition" 3 dg /dev/sda1 bootfs
expect "dev_guard refuses a device of another card" 3 dg /dev/mmcblk2p1 bootfs
expect "dev_guard refuses an unlabelled partition" 3 dg /dev/mmcblk1p2 bootfs

# 5. every remote rm, mv and tee names the stage directory
OUT=$(BOARD_PERSON_PRESENT=1 "$R" --dry-run default "$PROD" forever; "$R" --dry-run nodefault; "$R" --dry-run uninstall; "$R" --dry-run install)
if grep 'DRY board' <<<"$OUT" | grep -E 'rm |mv |tee ' | grep -vq '/boot/yakogl/standalone'; then
    bad "rm, mv and tee name the stage directory" "$OUT"
else ok "rm, mv and tee name the stage directory"; fi

# 6. the static scan
if python3 -I "$HERE/mp2s_protect_scan.py" "$R" >"$W/scan.out" 2>&1; then ok "mp2s_protect_scan.py: the runner writes no protected variable, partition or the eMMC"; else bad "protect scan" "$(cat "$W/scan.out")"; fi

# 7. the fake board: default, nodefault, refusals
fake "$FIX/pristine.bin" installed
BEFORE_ALL=$(env_print)
BEFORE_OTHERS=$(others)
ACTIVE0=$(python3 -I "$REPO/tools/uboot_env.py" "${ENVL[@]}" check "$FK/dev/mmcblk1p7" | sed -n 's/^active: copy //p')
if [ "$ACTIVE0" = 1 ]; then INACTIVE=2 ISEEK=62; else INACTIVE=1 ISEEK=63; fi
BC=$(env_print -n bootcmd)
ABC=$(env_print -n altbootcmd)
if [ "$(live printenv 2>/dev/null)" = "$BEFORE_ALL" ]; then ok "printenv prints the active copy of the card's u-boot-env"; else bad "printenv" "$(live printenv 2>&1 | head -5)"; fi
[ ! -s "$FK/dd.log" ] && [ ! -s "$FK/fw.log" ] && ok "printenv writes nothing and calls no fw_printenv" || bad "printenv" "$(cat "$FK/dd.log" "$FK/fw.log")"
expect "live default 2" 0 live default "$IMG" 2
has "it prints the undo" 'UNDO, from Linux'
has "it says nothing was rebooted" 'default set for 2 boots; nothing was rebooted.'
if [ "$(env_print -n mp2s_left)" = 2 ] && [ "$(env_print -n mp2s_ver)" = 1 ] &&
    [ "$(env_print -n mp2s_prefixes)" = '/ /boot/' ] && [ "$(env_print -n boot_prefixes)" = '/yakogl/standalone/ / /boot/' ]; then
    ok "the environment holds the default (mp2s_ver, mp2s_left, mp2s_prefixes, boot_prefixes)"
else bad "the default in the environment" "$(env_print | grep -E '^(mp2s_|boot_prefixes)')"; fi
if cmp -s "$IMG" "$FK/boot/yakogl/standalone/default.bin"; then ok "default.bin is the image"; else bad "default.bin" "not the image"; fi
[ ! -e "$FK/boot/yakogl/standalone/default.bin.part" ] && ok "no default.bin.part is left" || bad "default.bin.part" "left"
if [ "$(others)" = "$BEFORE_OTHERS" ]; then ok "every other variable kept its value"; else bad "other variables" "$(diff <(echo "$BEFORE_OTHERS") <(others))"; fi
[ "$(env_print -n bootcmd)" = "$BC" ] && [ "$(env_print -n altbootcmd)" = "$ABC" ] && ! env_print bootcount >/dev/null &&
    ok "bootcmd and altbootcmd as before, no bootcount" || bad "bootcmd/altbootcmd/bootcount" "changed"
[ "$(wc -l <"$FK/dd.log")" = 1 ] && ok "one environment write" || bad "one write" "$(cat "$FK/dd.log")"
grep -q "seek=$ISEEK " "$FK/dd.log" && ok "the write is copy $INACTIVE, the inactive one" || bad "the inactive copy" "$(cat "$FK/dd.log")"
python3 -I "$REPO/tools/uboot_env.py" "${ENVL[@]}" check "$FK/dev/mmcblk1p7" | grep -q "^active: copy $INACTIVE" && ok "copy $INACTIVE is active after the write" || bad "active copy" "$(python3 -I "$REPO/tools/uboot_env.py" "${ENVL[@]}" check "$FK/dev/mmcblk1p7")"
if stock_sum | cmp -s - "$W/stock.sum"; then ok "ST's files under /boot unchanged"; else bad "stock files" "changed"; fi
if grep -q '/boot/yakogl/standalone/default-prefixes.txt\|default-prefixes' "$FK/ssh.log"; then bad "the default record stays on the host" "sent"; else ok "the default record stays on the host"; fi
[ "$(cat "$MP2S_STATE_DIR/default-prefixes.txt")" = '/ /boot/' ] && ok "the default record holds the live boot_prefixes" || bad "default record" "$(cat "$MP2S_STATE_DIR/default-prefixes.txt" 2>&1)"
protected "default 2"

# a second default refuses and changes nothing
snap >"$W/snap1"
: >"$FK/dd.log"
expect "a second default refuses" 3 live default "$IMG" 3
has "it says nodefault first" 'nodefault first'
snap | cmp -s - "$W/snap1" && [ ! -s "$FK/dd.log" ] && ok "the refused default changed nothing" || bad "refused default" "changed the fake board"
expect "a forever default while counting refuses too" 3 env BOARD_PERSON_PRESENT=1 BOARD_HOST=u@fake "$R" default "$PROD" forever

expect "live nodefault" 0 live nodefault
has "nodefault prints its undo line" 'nothing to undo: nodefault only removes'
has "nodefault ends with Linux" "no default: ST's extlinux boots Linux again"
if [ -f "$MANIFEST" ]; then ok "nodefault compared /boot (the manifest tool is there)"; else has "nodefault says /boot was not compared" 'skip: mp2s_boot_manifest.py missing'; fi
if [ "$(env_print)" = "$BEFORE_ALL" ]; then ok "the environment's variables are as before default"; else bad "environment after nodefault" "$(diff <(echo "$BEFORE_ALL") <(env_print))"; fi
[ ! -e "$FK/boot/yakogl/standalone/default.bin" ] && ok "default.bin removed" || bad "default.bin" "still there"
[ -e "$FK/boot/yakogl/standalone/boot.scr.uimg" ] && ok "the installed script stays" || bad "boot.scr.uimg" "removed by nodefault"
[ ! -e "$MP2S_STATE_DIR/default-prefixes.txt" ] && ok "the default record is dropped" || bad "default record" "left"
protected "nodefault"
: >"$FK/dd.log"
expect "nodefault again (safe to repeat)" 0 live nodefault
has "it writes no environment" 'no default is set in the environment'
[ ! -s "$FK/dd.log" ] && ok "the repeated nodefault wrote no environment" || bad "repeated nodefault" "$(cat "$FK/dd.log")"

# the count ran out on the board (the script put the environment back):
# nodefault writes nothing, removes default.bin and drops the stale record
fake "$FIX/pristine.bin" installed
expect "live default 2 (to run out)" 0 live default "$IMG" 2
mkpart "$FIX/pristine.bin" "$FK/dev/mmcblk1p7"
: >"$FK/dd.log"
expect "nodefault after the count ran out" 0 live nodefault
[ ! -s "$FK/dd.log" ] && ok "it wrote no environment" || bad "ran-out nodefault" "$(cat "$FK/dd.log")"
[ ! -e "$MP2S_STATE_DIR/default-prefixes.txt" ] && ok "the stale default record is dropped" || bad "stale record" "left"
[ ! -e "$FK/boot/yakogl/standalone/default.bin" ] && ok "default.bin removed after the count ran out" || bad "default.bin" "left"

# forever with a person, then nodefault without the host's record (the pristine image is the record)
fake "$FIX/pristine.bin" installed
cp "$FK/dev/mmcblk1p7" "$FK/boot/yakogl/standalone/u-boot-env.bin"
expect "live forever with a person" 0 env BOARD_PERSON_PRESENT=1 BOARD_HOST=u@fake "$R" default "$PROD" forever
[ "$(env_print -n mp2s_left)" = forever ] && ok "mp2s_left=forever" || bad "forever" "$(env_print -n mp2s_left)"
protected "forever"
rm -f "$MP2S_STATE_DIR/default-prefixes.txt"
expect "nodefault without the host's default record" 0 live nodefault
has "it takes the pristine u-boot-env.bin's boot_prefixes" 'pristine u-boot-env.bin is the record'
[ "$(env_print -n boot_prefixes)" = '/ /boot/' ] && ! env_print mp2s_left >/dev/null && ok "boot_prefixes back, mp2s_* gone" || bad "nodefault after forever" "$(env_print | grep -E '^(mp2s_|boot_prefixes)')"

# a mismatching default record refuses
fake "$FIX/pristine.bin" installed
expect "live default 1" 0 live default "$IMG" 1
printf '/mmc0_' >"$MP2S_STATE_DIR/default-prefixes.txt"
snap >"$W/snap1"
: >"$FK/dd.log"
expect "nodefault refuses a record that differs from mp2s_prefixes" 3 live nodefault
snap | cmp -s - "$W/snap1" && [ ! -s "$FK/dd.log" ] && ok "the refused nodefault changed nothing" || bad "refused nodefault" "changed the fake board"

# refusals on the board: each changes nothing
refuses() { # NAME SETUP...: the setup on a fresh installed fake, then default 2 must refuse and change nothing
    local name=$1
    shift
    fake "$FIX/pristine.bin" installed
    "$@"
    protect_record
    BC0=$(env_print -n bootcmd)
    ABC0=$(env_print -n altbootcmd)
    snap >"$W/snap1"
    expect "default refuses: $name" 3 live default "$IMG" 2
    if snap | cmp -s - "$W/snap1" && [ ! -s "$FK/dd.log" ]; then ok "$name: nothing changed"; else bad "$name: nothing changed" "$(diff <(cat "$W/snap1") <(snap))"; fi
    protected "$name"
}
refuses "one environment copy bad" mkpart "$FIX/one-copy-bad.bin" "$FK/dev/mmcblk1p7"
refuses "both environment copies bad" mkpart "$FIX/both-bad.bin" "$FK/dev/mmcblk1p7"
refuses "a default already counting" mkpart "$FIX/counting2.bin" "$FK/dev/mmcblk1p7"
python3 -I "$REPO/tools/uboot_env.py" --redundant --size 0x2000 set "$FIX/pristine.bin" "$W/yako.bin" yako_once=1 >/dev/null
refuses "a one-shot (yako_*) armed" mkpart "$W/yako.bin" "$FK/dev/mmcblk1p7"
python3 -I "$REPO/tools/uboot_env.py" --redundant --size 0x2000 set "$FIX/pristine.bin" "$W/prefix.bin" "boot_prefixes=/yakogl/standalone/ / /boot/" >/dev/null
refuses "our prefix left in boot_prefixes" mkpart "$W/prefix.bin" "$FK/dev/mmcblk1p7"
refuses "ST's firmware update in trial" eval 'mkfwu "$FK/dev/mmcblk1p3" fefcffff; cp "$FK/dev/mmcblk1p3" "$FK/dev/mmcblk1p4"'
refuses "no u-boot-env on the card (only the eMMC has one)" eval 'grep -v u-boot-env "$FK/lsblk.txt" >"$FK/l"; mv "$FK/l" "$FK/lsblk.txt"'
refuses "/boot on the eMMC" eval 'echo /dev/mmcblk0p8 >"$FK/findmnt.out"'
refuses "/boot on a disk that says MMC" eval 'echo MMC >"$FK/sys/block/mmcblk1/device/type"'
refuses "/boot on the eMMC, numbered mmcblk1" eval 'renumber; echo /dev/mmcblk1p8 >"$FK/findmnt.out"'
refuses "not installed" rm -rf "$FK/boot/yakogl"
refuses "a staged script that is not this commit's" eval 'echo old >"$FK/boot/yakogl/standalone/boot.scr.uimg"'
refuses "a foreign file in the stage directory" mkdir "$FK/boot/yakogl/standalone/extlinux"
refuses "the card undo applied" mv "$FK/boot/yakogl/standalone/boot.scr.uimg" "$FK/boot/yakogl/standalone/boot.scr.uimg.off"
refuses "the board does not answer" touch "$FK/down"

# the card as mmcblk0, the eMMC as mmcblk1: the runner goes by the disk's
# type, so printenv and status read the card's u-boot-env (board, 2026-11-22:
# an index rule refused this card as "the eMMC (mmcblk0)")
fake "$FIX/pristine.bin" installed
renumber
expect "printenv with the card as mmcblk0" 0 live printenv
has "printenv with the card as mmcblk0: the card's partition" "/dev/mmcblk0p7: active"
has "printenv with the card as mmcblk0: its variables" "boot_prefixes="
hasnt "printenv with the card as mmcblk0: never the eMMC's partition" "mmcblk1p7"
expect "status with the card as mmcblk0" 0 live status
if grep -q 'mmcblk1p' "$FK/ssh.log"; then bad "the card as mmcblk0: no command names the eMMC's partitions" "$(grep 'mmcblk1p' "$FK/ssh.log")"; else ok "the card as mmcblk0: no command names the eMMC's partitions"; fi

# no board line at all for a refusal made on the host
fake "$FIX/pristine.bin" installed
expect "live forever without a person" 2 env -u BOARD_PERSON_PRESENT BOARD_HOST=u@fake "$R" default "$PROD" forever
expect "live LIFETIME_S=0 under a count" 3 live default "$NOLIFE" 2
[ ! -s "$FK/ssh.log" ] && ok "host-side refusals sent nothing to the board" || bad "host-side refusals" "$(cat "$FK/ssh.log")"

# a write that does not read back is undone
fake "$FIX/pristine.bin" installed
touch "$FK/dd.lie"
BEFORE_ALL=$(env_print)
expect "a default whose write does not read back" 4 live default "$IMG" 2
has "it says the environment is as before" 'nothing of it is left changed'
[ "$(env_print)" = "$BEFORE_ALL" ] && ok "the environment is as before" || bad "after a failed read-back" "changed"
[ "$(wc -l <"$FK/dd.log")" = 2 ] && cmp -s "$FK/dd-2.bin" <(dd if="$FK/dev/mmcblk1p7" bs=8192 skip=63 count=1 status=none) && ok "the restore wrote the copy's old bytes back" || bad "restore" "$(cat "$FK/dd.log")"
protected "failed read-back"

# 8. install and uninstall: the manifest tool
fake "$FIX/pristine.bin"
expect "live install without mp2s_boot_manifest.py" 3 env MP2S_BOOT_MANIFEST="$W/none.py" BOARD_HOST=u@fake "$R" install
has "it names the missing tool" 'mp2s_boot_manifest.py missing'
expect "live uninstall without mp2s_boot_manifest.py" 3 env MP2S_BOOT_MANIFEST="$W/none.py" BOARD_HOST=u@fake "$R" uninstall
has "it names the missing tool" 'mp2s_boot_manifest.py missing'
[ ! -s "$FK/ssh.log" ] && ok "without the tool nothing was sent to the board" || bad "without the tool" "$(cat "$FK/ssh.log")"
[ ! -e "$FK/boot/yakogl" ] && ok "without the tool nothing was staged" || bad "without the tool" "staged"

if [ ! -f "$MANIFEST" ]; then
    skip "mp2s_boot_manifest.py missing: live install, default, nodefault, uninstall cycle"
    skip "mp2s_boot_manifest.py missing: uninstall finds a changed stock file"
else
    export MP2S_BOOT_MANIFEST=$MANIFEST
    fake "$FIX/pristine.bin"
    BEFORE_ALL=$(env_print)
    snap >"$W/snap0"
    expect "live install" 0 live install
    has "install prints its undo" 'UNDO: tools/board-stm32mp2-standalone.sh uninstall'
    for f in boot.scr.uimg boot.scr.uimg.sha256 boot-sha256.txt u-boot-env.bin; do
        [ -f "$FK/boot/yakogl/standalone/$f" ] && ok "install staged $f" || bad "install" "no $f"
    done
    cmp -s "$SCR" "$FK/boot/yakogl/standalone/boot.scr.uimg" && ok "the staged script is this commit's build" || bad "staged script" "differs"
    cmp -s "$FK/dev/mmcblk1p7" "$FK/boot/yakogl/standalone/u-boot-env.bin" && ok "u-boot-env.bin is the pristine partition" || bad "u-boot-env.bin" "differs"
    [ "$(env_print)" = "$BEFORE_ALL" ] && [ ! -s "$FK/dd.log" ] && ok "install wrote no environment" || bad "install" "wrote the environment"
    ls "$FK/boot/yakogl/standalone" | grep -q '\.part$' && bad "install" "a .part is left" || ok "install left no .part"
    stock_sum | cmp -s - "$W/stock.sum" && ok "install left ST's files alone" || bad "install" "stock files changed"
    expect "install again: nothing to do" 0 live install
    has "it says so" 'nothing to do'
    expect "live default 2 after install" 0 live default "$IMG" 2
    expect "live uninstall (nodefault first)" 0 live uninstall
    has "uninstall prints its undo" 'UNDO: tools/board-stm32mp2-standalone.sh install'
    has "uninstall requires mp2s-boot: same" 'mp2s-boot: same'
    has "uninstall names the card undo" 'tools/mp2s_card_undo.sh'
    [ ! -e "$FK/boot/yakogl/standalone" ] && ok "the stage directory is gone" || bad "uninstall" "stage directory left"
    [ "$(env_print)" = "$BEFORE_ALL" ] && ok "the environment's variables are as before install" || bad "uninstall" "environment differs"
    protected "install-default-uninstall"
    fake "$FIX/pristine.bin"
    expect "live install (2)" 0 live install
    echo "changed" >>"$FK/boot/Image.gz"
    expect "uninstall with a changed stock file needs a person" 5 live uninstall
    has "it says NEEDS A PERSON" 'NEEDS A PERSON'
    unset MP2S_BOOT_MANIFEST
fi

echo "board_stm32mp2_standalone_selftest: $N checks, $F failed, $S skipped"
[ "$F" = 0 ]

#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# selftest.sh - collect.sh and summarize.py without a board.
#
#   tools/probes/stm32mp257f_dk/env_read/selftest.sh
#
# Builds in $TMPDIR a redundant environment in a 512 KiB image (copies of
# 0x2000 at 0x7e000 and 0x7c000, made with tools/uboot_env.py set), a fake
# fw_printenv and fake /boot listings, and runs summarize.py on identical,
# env-changed, bootcmd-changed, /boot-file-changed, yakogl-only-changed and
# corrupt-CRC cases. A fake ssh on PATH answers collect.sh from the same
# fixture, and logs every command it is sent: none may write. Ends with
# "env_read selftest: N ok, M failed"; exit 0 only when M is 0.
set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
tools=$(cd "$here/../../.." && pwd)
envtool=$tools/uboot_env.py
sum=$here/summarize.py
col=$here/collect.sh
T=$(mktemp -d "${TMPDIR:-/tmp}/env_read.XXXXXX") || exit 1
trap 'rm -rf "$T"' EXIT
L=(--redundant --size 0x2000 --offset 0x7e000 --offset2 0x7c000)

n=0
failed=0
ok() { n=$((n + 1)); echo "ok   $1"; }
bad() { n=$((n + 1)); failed=$((failed + 1)); echo "FAIL $1"; }
# expect NAME WANT_RC CMD...: the exit code of CMD (output kept in $T/out)
expect() {
    local name=$1 want=$2 rc
    shift 2
    "$@" > "$T/out" 2>&1
    rc=$?
    if [ "$rc" = "$want" ]; then ok "$name"; else bad "$name (exit $rc, want $want)"; sed 's/^/     | /' "$T/out" | head -20; fi
}
has() { if grep -q -- "$2" "$T/out"; then ok "$1"; else bad "$1 (no '$2')"; sed 's/^/     | /' "$T/out" | head -20; fi; }

# ---- the fixture -----------------------------------------------------------
python3 -I - "$T/blank.bin" <<'PY'
import struct, sys, zlib
b = bytearray(b"\xff" * 524288)
data = bytearray(b"\xff" * (0x2000 - 5))
s = (b"scriptaddr=0x84000000\0kernel_addr_r=0x84000000\0fdt_addr_r=0x8c000000\0"
     b"ramdisk_addr_r=0x8d000000\0boot_targets=mmc1 mmc0 usb0\0boot_prefixes=/ /boot/\0"
     b"boot_scripts=boot.scr.uimg boot.scr\0boot_syslinux_conf=extlinux/extlinux.conf\0"
     b"bootdelay=1\0bootcmd=run bootcmd_stm32mp\0bootcount=0\0\0")
data[:len(s)] = s
for off, flags in ((0x7e000, 1), (0x7c000, 0)):
    b[off:off + 4] = struct.pack("<I", zlib.crc32(bytes(data)))
    b[off + 4] = flags
    b[off + 5:off + 0x2000] = data
open(sys.argv[1], "wb").write(b)
PY
base=$T/base.bin
mv "$T/blank.bin" "$base"
python3 -I "$envtool" "${L[@]}" set "$base" "$T/env-changed.bin" bootdelay=3 > /dev/null
python3 -I "$envtool" "${L[@]}" set "$base" "$T/boot-changed.bin" bootcmd='run yakogl' > /dev/null
# the corrupt one: a flipped data byte in the active copy
cp "$base" "$T/corrupt.bin"
printf 'Z' | dd of="$T/corrupt.bin" bs=1 seek=$((0x7e000 + 12)) conv=notrunc status=none

# mk DIR IMAGE: a collect.sh-shaped directory (fw_printenv from the image)
mk() {
    mkdir -p "$1/boot"
    cp "$2" "$1/u-boot-env.bin"
    python3 -I "$envtool" "${L[@]}" print "$2" > "$1/fw_printenv.txt" 2> /dev/null
    echo /dev/mmcblk1p7 > "$1/u-boot-env.dev"
    echo "/dev/mmcblk1p8 ext4" > "$1/boot-mount.txt"
    printf 'Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/mmcblk1p8 120000 20000 100000 17%% /boot\n' > "$1/boot-df.txt"
    printf '%s  ./Image\n%s  ./stm32mp257f-dk.dtb\n%s  ./extlinux/extlinux.conf\n%s  ./yakogl/mp2_hello.bin\n' \
        $(printf 'a%.0s' $(seq 64)) $(printf 'b%.0s' $(seq 64)) $(printf 'c%.0s' $(seq 64)) $(printf 'd%.0s' $(seq 64)) > "$1/boot-sha256.txt"
    printf '1000 ./Image\n2000 ./stm32mp257f-dk.dtb\n300 ./extlinux/extlinux.conf\n400 ./yakogl/mp2_hello.bin\n' > "$1/boot-sizes.txt"
}
# setsha DIR NAME CHAR: that file's sha256 becomes 64 CHARs
setsha() {
    local h
    h=$(head -c 64 /dev/zero | tr '\0' "$3")
    awk -v n="$2" -v h="$h" '{ if ($2 == n) $1 = h; print $1 "  " $2 }' "$1/boot-sha256.txt" > "$1/x" && mv "$1/x" "$1/boot-sha256.txt"
}
mk "$T/a" "$base"
mk "$T/same" "$base"
mk "$T/envchg" "$T/env-changed.bin"
mk "$T/bootcmd" "$T/boot-changed.bin"
mk "$T/filechg" "$base"
setsha "$T/filechg" ./extlinux/extlinux.conf e
mk "$T/yako" "$base"
setsha "$T/yako" ./yakogl/mp2_hello.bin f
echo "1 ./yakogl/extra.bin" >> "$T/yako/boot-sizes.txt"
echo "$(head -c 64 /dev/zero | tr '\0' 9)  ./yakogl/extra.bin" >> "$T/yako/boot-sha256.txt"
mk "$T/corrupt" "$T/corrupt.bin"

# ---- summarize.py ----------------------------------------------------------
S="python3 -I $sum"
expect "single: a good directory renders" 0 $S "$T/a"
has "single: section-11 variable row" '^| bootcmd | run bootcmd_stm32mp |'
has "single: an unset variable is shown" '^| altbootcmd | (unset) |'
has "single: the partition size" '524288 bytes'
has "single: both copies checked" 'active copy'
has "single: bootfs partition number" '^| bootfs partition number | 8 |'
has "single: free space" '100000 KiB free'
has "single: /boot file row" '^| ./Image | 1000 |'
expect "identical directories" 0 $S "$T/a" "$T/same"
has "identical: result" 'RESULT: IDENTICAL'
has "identical: fw_printenv diff empty" 'fw_printenv diff: empty'
expect "env changed" 1 $S "$T/a" "$T/envchg"
has "env changed: the variable is named" '^| bootdelay | 1 | 3 |'
expect "bootcmd changed" 1 $S "$T/a" "$T/bootcmd"
has "bootcmd changed: shown" '^| bootcmd '
expect "stock /boot file changed" 1 $S "$T/a" "$T/filechg"
has "/boot file changed: named" 'extlinux/extlinux.conf'
expect "yakogl/ only changed" 0 $S "$T/a" "$T/yako"
has "yakogl/ only: ignored" 'ignored: 2 file'
expect "corrupt CRC, single" 1 $S "$T/corrupt"
has "corrupt CRC: reported" 'FAILED'
expect "corrupt CRC, compare" 1 $S "$T/a" "$T/corrupt"
expect "a missing directory is a usage error" 2 $S "$T/nope"
# an image that differs only in the inactive copy is still a difference
python3 -I "$envtool" "${L[@]}" set "$base" "$T/inactive.bin" zzz=1 > /dev/null
mk "$T/inact" "$T/inactive.bin"
cp "$base" "$T/inact/u-boot-env.bin"
dd if="$T/inactive.bin" of="$T/inact/u-boot-env.bin" bs=1 skip=$((0x7c000)) seek=$((0x7c000)) count=$((0x2000)) conv=notrunc status=none
cp "$T/a/fw_printenv.txt" "$T/inact/fw_printenv.txt"
expect "an image byte differs, fw_printenv does not" 1 $S "$T/a" "$T/inact"

# ---- collect.sh against a fake ssh -----------------------------------------
mkdir -p "$T/bin"
cat > "$T/bin/ssh" <<'EOF'
#!/bin/bash
# fake ssh: the last argument is the remote command; log it, answer from $FAKE
cmd=${*: -1}
echo "$cmd" >> "$FAKE/ssh.log"
case $cmd in
    findmnt*) echo "/dev/mmcblk1p8 ext4" ;;
    "lsblk -rno"*) printf '/dev/mmcblk1 \n/dev/mmcblk1p7 u-boot-env\n/dev/mmcblk1p8 bootfs\n' ;;
    "lsblk -o"*) echo "NAME SIZE TYPE PARTLABEL MOUNTPOINT"; echo "mmcblk1p8 1G part bootfs /boot" ;;
    *fw_printenv*) cat "$FAKE/a/fw_printenv.txt" ;;
    *"dd if="*) cat "$FAKE/a/u-boot-env.bin" ;;
    "ls -lR"*) echo "total 0" ;;
    *printf*) cat "$FAKE/a/boot-sizes.txt" ;;
    *sha256sum*) cat "$FAKE/a/boot-sha256.txt" ;;
    "df -P"*) cat "$FAKE/a/boot-df.txt" ;;
    *) echo "fake ssh: unexpected command: $cmd" >&2; exit 9 ;;
esac
EOF
chmod +x "$T/bin/ssh"
export FAKE=$T
PATH=$T/bin:$PATH BOARD_HOST=user@board MP2_SUDO= expect "collect.sh reads a fake board" 0 bash "$col" "$T/got"
expect "collected directory equals the fixture" 0 python3 -I "$sum" "$T/a" "$T/got"
expect "collect.sh without BOARD_HOST refuses" 2 env -u BOARD_HOST bash "$col" "$T/got2"
expect "collect.sh without an argument refuses" 2 env BOARD_HOST=x bash "$col"
# every command sent is one of the read-only ones
if grep -Eq 'fw_setenv|dd of=|mount|modprobe|rmmod|insmod|tailscale|sshd|systemctl| > /dev|tee ' "$T/ssh.log"; then
    bad "no write command was sent to the board"; cat "$T/ssh.log"
else
    ok "no write command was sent to the board"
fi

# ---- the script itself: forbidden commands (comment lines excluded) --------
code=$(grep -v '^[[:space:]]*#' "$col")
for pat in 'fw_setenv' 'dd of=' 'mount ' 'umount' 'modprobe' 'rmmod' 'insmod' 'tailscale' 'sshd' 'systemctl' 'mkfs' 'tee '; do
    if grep -q -- "$pat" <<<"$code"; then bad "collect.sh has no '$pat'"; else ok "collect.sh has no '$pat'"; fi
done
# no board identity in the new files
if grep -nE '([0-9]{1,3}\.){3}[0-9]{1,3}|/dev/tty|\.ts\.net' "$here"/collect.sh "$here"/summarize.py "$here"/README.md; then
    bad "no address, host name or tty path in the files"
else
    ok "no address, host name or tty path in the files"
fi
command -v shellcheck > /dev/null && { if shellcheck "$col" "$here/selftest.sh" > "$T/out" 2>&1; then ok "shellcheck"; else bad "shellcheck"; head -20 "$T/out"; fi; }

echo "env_read selftest: $((n - failed)) ok, $failed failed"
[ "$failed" = 0 ]

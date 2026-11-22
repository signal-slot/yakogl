#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Selftest of tools/mp2s_comeback.sh. No board, no network: a fake ssh on
# PATH answers the check's one remote command with canned key=value readings
# (or refuses, exit 255, when no answer is canned) and records the command
# line it was given. Checks: every verdict line with its exit code, the
# --wait rules, the target coming from BOARD_HOST (BOARD_SSH as fallback, a
# refusal with neither), MP2_SUDO's default, that only reads are ever asked
# of the board (no write verb, no service call, no sshd or tailscale
# question, nothing naming the eMMC), and that no verdict leaks the fake
# target, its user or a fake address.
#
# Every host and address fixture is assembled at run time (the idiom of
# tests/redaction_scan.rs and tools/mp2_uboot_record_check.py): no host name
# and no address literal sits in this file, so the tree's redaction scan
# stays clean. The host fixture uses the reserved .invalid name (RFC 2606).
#
#   tools/mp2s_comeback_selftest.sh
#
# Prints "mp2s_comeback_selftest: N checks, F failed"; exit 0 only when F=0.
set -u
export LC_ALL=C
HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/mp2s_comeback.sh
[ -x "$TOOL" ] || { echo "mp2s_comeback_selftest: cannot run $TOOL" >&2; exit 2; }
unset BOARD_HOST BOARD_SSH MP2_SUDO MP2_KNOWN_HOSTS

W=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_comeback_selftest.XXXXXX") || exit 2
trap 'rm -rf "$W"' EXIT
mkdir "$W/bin"

# fixtures, assembled at run time so no literal of them is in this file
DOT=. C=:
FHOST=dkfake${DOT}box${DOT}invalid
FA4=192${DOT}168${DOT}7${DOT}9
FB4=10${DOT}9${DOT}8${DOT}7
FA6=fd7a${C}115c${C}a1e0${C}${C}1
TARGET=tester@${FHOST}
BOOTID=11111111-2222-3333-4444-555555555555
OLDID=99998888-7777-6666-5555-444433332222
FAKE_OUT=$W/answers FAKE_LOG=$W/ssh.log
FIX=$HERE/testdata/mp2s-env/pristine.bin

cat >"$W/bin/ssh" <<'EOF'
#!/bin/sh
# fake ssh: record the command line as one line, answer with the canned
# reading, or refuse (255: the board does not answer) when none is canned
if [ -n "${FAKE_LOG:-}" ]; then { printf '%s' "$*" | tr '\n' ' '; echo; } >>"$FAKE_LOG"; fi
if [ -f "${FAKE_OUT:-}" ]; then cat "$FAKE_OUT"; exit 0; fi
exit 255
EOF
chmod +x "$W/bin/ssh"
export PATH="$W/bin:$PATH"

# the canned readings, as the key=value fields the tool's own remote prints
reset() {
    B_ID=$BOOTID B_UP=42 B_KERNEL=6.6.78 B_MOUNT=/dev/mmcblk1p8 B_FSTYPE=ext4
    B_TYPE=SD B_ROOT=/dev/mmcblk1p9 B_RC=0 B_MP2S= B_PFX='/ /boot/' B_BADCRC=0
}
mk() { # write $FAKE_OUT from the current B_ fields
    local m=$B_MOUNT r=$B_ROOT
    [ -n "$m" ] && m="$m $B_FSTYPE"
    [ -n "$r" ] && r="$r $B_FSTYPE"
    {
        echo "boot_id=$B_ID"
        echo "uptime=$B_UP"
        echo "kernel=$B_KERNEL"
        echo "boot_mount=$m"
        echo "boot_type=$B_TYPE"
        echo "root_mount=$r"
        echo "env_dev=/dev/mmcblk1p7"
        [ "$B_RC" = 0 ] && echo "env_b64=$(mkenv | base64 -w 0)"
        echo end=1
    } >"$FAKE_OUT"
}
# mkenv: the card's u-boot-env partition as the board would send it: 512 KiB,
# copy 1 at 0x7e000 and copy 2 at 0x7c000, made from tools/testdata/mp2s-env/
# pristine.bin with boot_prefixes=$B_PFX and each name of $B_MP2S set; with
# B_BADCRC=1 both copies' CRCs are broken
mkenv() {
    local sets=() n
    [ -n "$B_PFX" ] && sets+=("boot_prefixes=$B_PFX")
    for n in $B_MP2S; do sets+=("$n=1"); done
    if [ ${#sets[@]} -gt 0 ]; then
        python3 -I "$HERE/uboot_env.py" --redundant --size 0x2000 set "$FIX" "$W/env.bin" "${sets[@]}" >/dev/null || return 1
    else
        cp "$FIX" "$W/env.bin"
    fi
    head -c 524288 /dev/zero >"$W/part.bin"
    dd if="$W/env.bin" of="$W/part.bin" bs=8192 skip=0 seek=63 count=1 conv=notrunc status=none
    dd if="$W/env.bin" of="$W/part.bin" bs=8192 skip=1 seek=62 count=1 conv=notrunc status=none
    if [ "${B_BADCRC:-0}" = 1 ]; then
        printf 'XXXX' | dd of="$W/part.bin" bs=1 seek=$((0x7e000)) conv=notrunc status=none
        printf 'XXXX' | dd of="$W/part.bin" bs=1 seek=$((0x7c000)) conv=notrunc status=none
    fi
    cat "$W/part.bin"
}

N=0 F=0
ok() { N=$((N + 1)); echo "ok   $1"; }
bad() { N=$((N + 1)); F=$((F + 1)); echo "FAIL $1: $2"; }
run_tool() { OUT=$(env BOARD_HOST=$TARGET FAKE_OUT=$FAKE_OUT FAKE_LOG=$FAKE_LOG "$TOOL" "$@" 2>&1); RC=$?; }
run() { reset; mk; run_tool "$@"; }
env_run() { OUT=$(env "$@" 2>&1); RC=$?; }
expect() { [ "$RC" = "$1" ] && ok "$2" || bad "$2" "rc $RC, wanted $1: $OUT"; }
has() { grep -qF -- "$2" <<<"$OUT" && ok "$1" || bad "$1" "no '$2' in: $OUT"; }
hasnt() { grep -qF -- "$2" <<<"$OUT" && bad "$1" "'$2' leaked into: $OUT" || ok "$1"; }

# 1. the ok verdict, boot id compared against a different one
run --before-boot-id "$OLDID"
expect 0 'ok exits 0'
has 'the verdict line' 'comeback: ok OpenSTLinux'
has 'kernel, mount and disk type named' '/boot on /dev/mmcblk1p8 (ext4, disk type SD), / on /dev/mmcblk1p9'
has 'the environment is called clean' 'no mp2s_* and no /yakogl/standalone/ prefix'
has 'the boot id comparison is reported' 'new boot, the boot id differs'
has 'the uptime is printed' 'comeback: uptime 42 s'
hasnt 'the fake target never comes back' "$TARGET"
hasnt 'no fake host in the verdict' dkfake
hasnt 'no fake user in the verdict' tester

# 2. ok without a comparison, and the tool says so
run
expect 0 'ok without --before-boot-id'
has 'says the id was not compared' 'boot id not compared (no --before-boot-id)'

# 3. every failed reading: rc 1 and the reason named
run --before-boot-id "$BOOTID"
expect 1 'same boot id FAILs'
has 'the reason' 'same boot id: no reboot happened'
reset; B_ID=; mk; run_tool --before-boot-id "$OLDID"
expect 1 'no boot id FAILs'
has 'the reason' 'no boot id'
reset; B_TYPE=MMC; mk; run_tool
expect 1 'an MMC /boot FAILs'
has 'the reason names the type' 'reads '\''MMC'\'', not SD'
has 'the reason names the eMMC' 'MMC is the eMMC'
reset; B_FSTYPE=vfat; mk; run_tool
expect 1 'a vfat /boot FAILs'
has 'the reason' 'not ext4'
reset; B_MOUNT=/dev/mmcblk1; mk; run_tool
expect 1 'a whole disk as /boot FAILs'
has 'the reason' 'is not a partition'
reset; B_MOUNT=; mk; run_tool
expect 1 'an unmounted /boot FAILs'
has 'the reason' '/boot not mounted'
hasnt 'the fstype is not reported on top of it' 'not ext4'
reset; B_ROOT=; mk; run_tool
expect 1 'an unmounted / FAILs'
has 'the reason' '/ not mounted'
reset; B_RC=1; mk; run_tool
expect 1 'an unreadable u-boot-env partition FAILs'
has 'the variables are not taken as unset' 'not taken as unset'
has 'the reason' 'could not be read'
reset; B_BADCRC=1; mk; run_tool
expect 1 'a u-boot-env whose CRCs fail FAILs'
has 'the reason' 'CRC fails'
reset; B_MP2S='mp2s_left mp2s_prefixes mp2s_ver'; mk; run_tool
expect 1 'left mp2s_* FAILs'
has 'the names are listed' 'mp2s_* left in the environment: mp2s_left mp2s_prefixes mp2s_ver'
reset; B_PFX='/ /boot/ /yakogl/standalone/'; mk; run_tool
expect 1 'a standalone prefix in boot_prefixes FAILs'
has 'the reason' 'boot_prefixes still carries /yakogl/standalone/'

# 4. no answer, or an answer that does not end: NEEDS A PERSON, nothing retried
reset; mk; sed -i '/^end=1$/d' "$FAKE_OUT"; run_tool
expect 4 'a reading without its end line is NEEDS A PERSON'
has 'it says a person is needed' 'NEEDS A PERSON'
reset; rm -f "$FAKE_OUT"; run_tool
expect 4 'no ssh answer is NEEDS A PERSON'
has 'it points at the supply' 'USB-C supply first'
reset; rm -f "$FAKE_OUT"; run_tool --wait 5
expect 4 '--wait 5 keeps trying, then NEEDS A PERSON'
has 'it names the waited seconds' 'did not answer ssh within 5 s'
reset; mk; run_tool --wait 901
expect 2 '--wait over 900 refused'
has 'the cap is named' 'capped at 900 s'
run_tool --wait x
expect 2 '--wait not whole seconds refused'
has 'the reason' 'takes whole seconds'
run_tool --frobnicate
expect 2 'an unknown option refused'
run_tool --help
expect 2 '--help prints usage and exits 2'
has 'the usage line' 'usage: mp2s_comeback.sh'

# 5. the target: BOARD_HOST, the BOARD_SSH fallback, and a refusal with neither
env_run -u BOARD_HOST -u BOARD_SSH FAKE_OUT="$FAKE_OUT" FAKE_LOG="$FAKE_LOG" "$TOOL"
expect 2 'no BOARD_HOST and no BOARD_SSH: refused'
has 'it says there is no default' 'there is no default'
hasnt 'no fake host in the refusal' dkfake
env_run BOARD_SSH="$TARGET" FAKE_OUT="$FAKE_OUT" FAKE_LOG="$FAKE_LOG" "$TOOL"
expect 0 'BOARD_SSH answers when BOARD_HOST is unset'
has 'the verdict line' 'comeback: ok OpenSTLinux'

# 6. what reaches the board: one ssh call, the runners' ssh options, the
# sudo prefix from MP2_SUDO, and only reads (the note's section 8: no module,
# no service call, nothing about sshd or tailscale, never the eMMC)
reset; mk; : >"$FAKE_LOG"
run_tool --before-boot-id "$OLDID"
expect 0 'run for the ssh-log checks'
if [ "$(wc -l <"$FAKE_LOG")" = 1 ]; then ok 'exactly one ssh call per check'; else bad 'one ssh call' "$(cat "$FAKE_LOG")"; fi
for o in StrictHostKeyChecking=yes UpdateHostKeys=no BatchMode=yes; do
    if grep -qF -- "$o" "$FAKE_LOG"; then ok "the ssh options carry $o"; else bad "ssh option $o" "$(cat "$FAKE_LOG")"; fi
done
if grep -qF -- 'sudo -n head -c 1048576' "$FAKE_LOG"; then ok 'MP2_SUDO defaults to sudo -n as the standalone runner does'; else bad 'MP2_SUDO default' "$(cat "$FAKE_LOG")"; fi
if grep -qF -- 'by-partlabel' "$FAKE_LOG"; then bad 'the partition comes from lsblk of the disk' "$(cat "$FAKE_LOG")"; else ok 'the partition comes from lsblk of the disk, never by-partlabel'; fi
if grep -qE '(^|[^A-Za-z0-9_])(fw_printenv|fw_setenv|saveenv|reboot|poweroff|shutdown|modprobe|insmod|systemctl|tailscale|sshd|mount|umount|rm|mv|dd|tee|chmod|chown|mkdir|truncate|unlink|mmcblk0)([^A-Za-z0-9_]|$)' "$FAKE_LOG"; then
    bad 'only reads reach the board' "$(cat "$FAKE_LOG")"
else
    ok 'only reads reach the board: no write verb, no service call, no sshd or tailscale question, no mmcblk0'
fi
reset; mk; : >"$W/log2"
env_run BOARD_HOST="$TARGET" MP2_SUDO= FAKE_OUT="$FAKE_OUT" FAKE_LOG="$W/log2" "$TOOL"
expect 0 'MP2_SUDO= runs'
if grep -qF -- 'head -c 1048576' "$W/log2" && ! grep -qF -- 'sudo' "$W/log2"; then
    ok 'MP2_SUDO= sends the partition read bare'
else
    bad 'MP2_SUDO empty' "$(cat "$W/log2")"
fi
reset; mk; : >"$W/log3"
env_run BOARD_HOST="$TARGET" 'MP2_SUDO=doas ' FAKE_OUT="$FAKE_OUT" FAKE_LOG="$W/log3" "$TOOL"
expect 0 'MP2_SUDO with another prefix runs'
if grep -qF -- 'doas head -c 1048576' "$W/log3"; then ok 'MP2_SUDO travels into the one remote command'; else bad 'MP2_SUDO prefix' "$(cat "$W/log3")"; fi

# 7. a board reading that itself carries a host name and addresses: the
# verdict must mask them (the fake target, its user, and the fixture
# addresses may not come back)
reset; B_KERNEL="${FHOST}-${FA4}-${FB4}-${FA6}"; mk; run_tool
expect 0 'a dirty kernel line still passes the verdict'
has 'the target is masked' '<board>'
has 'the addresses are masked' '<address>'
hasnt 'no fake host' dkfake
hasnt 'no fake user' tester
hasnt 'no fake IPv4' "$FA4"
hasnt 'no other fake IPv4' "$FB4"
hasnt 'no fake IPv6' "fd7a${C}"

echo "mp2s_comeback_selftest: $N checks, $F failed"
[ "$F" = 0 ]

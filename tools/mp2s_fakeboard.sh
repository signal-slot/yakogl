#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# A fake STM32MP257F-DK for the no-board selftests of the DK standalone runner
# and session driver (docs/notes/stm32mp257f-dk-standalone.md). No ssh, no board.
#
#   tools/mp2s_fakeboard.sh init DIR      create the fake root in DIR
#   tools/mp2s_fakeboard.sh shell DIR     print PATH/env settings (eval them)
#   tools/mp2s_fakeboard.sh writes DIR    print DIR/writes.log
#   tools/mp2s_fakeboard.sh put DIR PATH  stdin -> PATH (a path under /boot)
#
# DIR holds:
#   boot/       the microSD's bootfs as ST ships it: extlinux/extlinux.conf,
#               mmc0_extlinux/extlinux.conf (the file the saved boot_prefixes
#               reaches, section 2), boot.scr.uimg, Image, stm32mp257f-dk.dtb;
#               placeholder bytes
#   env/u-boot-env.bin  the top 0x4000 bytes of the u-boot-env partition: a
#               redundant pair of CONFIG_ENV_SIZE 0x2000, copy 1 (0x7e000) at
#               0x2000, copy 2 (0x7c000) at 0; both hold ST's default env
#               (driver/tools/mp2-standalone/st-env/) as the stock boot.scr.uimg
#               saved it (boot_prefixes=/mmc0_, boot_targets=mmc0, no bootfile)
#               plus bootcount=0. Built and edited with tools/uboot_env.py
#               (layout in env/layout)
#   captured/{findmnt,lsblk,blkid}.txt  /boot on the microSD's bootfs
#               (mmcblk0p8; the eMMC is mmcblk1), generic UUIDs only
#   sys/block/mmcblk{0,1}/device/type   SD and MMC, for a dev_guard
#   bin/        shims, first in PATH after `shell`: fw_printenv fw_setenv
#               findmnt lsblk blkid ssh sudo reboot systemctl
#
# `ssh [opts] HOST CMD...` runs CMD here (cwd DIR), with /boot and /sys/block
# mapped into DIR; a redirect, tee or dd of= to an absolute path outside DIR
# (other than /dev/null) is refused before it runs. `sudo [-n] CMD` runs CMD.
# `reboot` and `systemctl reboot|poweroff` only log. fw_setenv takes NAME
# [VALUE] or -s FILE (- is stdin; "NAME VALUE" per line, a bare NAME deletes;
# one write for all names, as on the board).
#
# Every write is appended to DIR/writes.log, one line each:
#   setenv NAME VALUE | file PATH SHA256 | file PATH removed | reboot | VIOLATION ...
# A write of a variable the design never touches (section 3: bootcmd,
# bootcmd_stm32mp, altbootcmd, bootcount, bootlimit, upgrade_available,
# boot_targets, boot_syslinux_conf, boot_extlinux, boot_scripts, preboot,
# mp2z_*, yako_*), or of a file outside DIR/boot, is logged VIOLATION and
# exits non-zero. A refused fw_setenv writes nothing; an ssh command's files
# are judged after it ran (sha256 of DIR before and after).
set -u
SELF=$(readlink -f "${BASH_SOURCE[0]}")
TOOLS=$(dirname "$SELF")
REPO=$(dirname "$TOOLS")
ST_ENV=$REPO/driver/tools/mp2-standalone/st-env/stm32mp25-dk-initial-env.txt
PROTECTED="bootcmd bootcmd_stm32mp altbootcmd bootcount bootlimit upgrade_available
boot_targets boot_syslinux_conf boot_extlinux boot_scripts preboot"
LAYOUT=(--size 0x2000 --offset 0x2000 --offset2 0)

die() { echo "mp2s_fakeboard: $*" >&2; exit 2; }
log() { printf '%s\n' "$*" >> "$ROOT/writes.log"; }
envtool() { python3 -I "$TOOLS/uboot_env.py" "${LAYOUT[@]}" "$@"; }

# sha256 of every file a command may change, minus the fake's own state
manifest() {
    (cd "$ROOT" && find . -type f ! -path ./writes.log ! -path ./.manifest \
        ! -path './env/*' -print0 | sort -z | xargs -0 -r sha256sum | sed 's/  \.\//  /')
}

fb_init() {
    ROOT=$1
    [ -f "$ST_ENV" ] || die "no $ST_ENV"
    mkdir -p "$ROOT"/boot/{extlinux,mmc0_extlinux} "$ROOT"/{env,captured,bin} \
        "$ROOT"/sys/block/mmcblk{0,1}/device || die "mkdir"
    printf 'placeholder kernel Image\n' > "$ROOT/boot/Image"
    printf 'placeholder device tree\n' > "$ROOT/boot/stm32mp257f-dk.dtb"
    printf 'placeholder stock boot.scr.uimg\n' > "$ROOT/boot/boot.scr.uimg"
    local c
    for c in extlinux mmc0_extlinux; do
        cat > "$ROOT/boot/$c/extlinux.conf" <<'X'
menu title Select the boot mode
DEFAULT OpenSTLinux
TIMEOUT 1
LABEL OpenSTLinux
	KERNEL /Image
	FDT /stm32mp257f-dk.dtb
	APPEND root=PARTLABEL=rootfs rootwait rw console=ttySTM0,115200
X
    done
    echo SD > "$ROOT/sys/block/mmcblk0/device/type"
    echo MMC > "$ROOT/sys/block/mmcblk1/device/type"
    printf '%s\n' "${LAYOUT[*]}" > "$ROOT/env/layout"

    # an empty valid pair (copy 1 active), then ST's env written twice by
    # uboot_env.py so that both copies hold it, as after two saves
    python3 -I - "$ROOT/env/u-boot-env.bin" <<'X' || die "env pair"
import sys, zlib
size = 0x2000
data = b"\0\0".ljust(size - 5, b"\0")
def copy(flags):
    return zlib.crc32(data).to_bytes(4, "little") + bytes([flags]) + data
open(sys.argv[1], "wb").write(copy(0) + copy(1))  # copy 2 at 0, copy 1 at 0x2000
X
    local -a vars=()
    local line
    while IFS= read -r line; do
        case $line in '' | '#'* | *=) continue ;;
        bootfile=* | boot_prefixes=* | boot_targets=*) continue ;; esac
        vars+=("$line")
    done < "$ST_ENV"
    vars+=('boot_prefixes=/mmc0_' 'boot_targets=mmc0' 'bootcount=0')
    envtool set "$ROOT/env/u-boot-env.bin" "$ROOT/env/u-boot-env.bin" "${vars[@]}" >/dev/null &&
        envtool set "$ROOT/env/u-boot-env.bin" "$ROOT/env/u-boot-env.bin" "${vars[@]}" >/dev/null ||
        die "uboot_env.py set failed"

    local u=00000000-0000-0000-0000-0000000000
    cat > "$ROOT/captured/findmnt.txt" <<'X'
TARGET SOURCE         FSTYPE OPTIONS
/boot  /dev/mmcblk0p8 ext4   rw,relatime
X
    cat > "$ROOT/captured/lsblk.txt" <<'X'
NAME         LABEL      TYPE PARTLABEL   SIZE  MOUNTPOINT
mmcblk0                 disk                 14.8G
|-mmcblk0p1             part fsbla1      256K
|-mmcblk0p2             part fsbla2      256K
|-mmcblk0p3             part metadata1   256K
|-mmcblk0p4             part metadata2   256K
|-mmcblk0p5             part fip-a         4M
|-mmcblk0p6             part fip-b         4M
|-mmcblk0p7             part u-boot-env  512K
|-mmcblk0p8  bootfs     part bootfs       64M  /boot
|-mmcblk0p9  vendorfs   part vendorfs     16M
|-mmcblk0p10 rootfs     part rootfs        4G  /
`-mmcblk0p11 userfs     part userfs        8G  /usr/local
mmcblk1                 disk                 29.1G
|-mmcblk1p1             part fsbla1      256K
|-mmcblk1p7             part u-boot-env  512K
|-mmcblk1p8  bootfs     part bootfs       64M
`-mmcblk1p10 rootfs     part rootfs        4G
X
    cat > "$ROOT/captured/blkid.txt" <<X
/dev/mmcblk0p7: PARTLABEL="u-boot-env" PARTUUID="${u}07"
/dev/mmcblk0p8: LABEL="bootfs" UUID="${u}08" TYPE="ext4" PARTLABEL="bootfs" PARTUUID="${u}18"
/dev/mmcblk0p10: LABEL="rootfs" UUID="${u}0a" TYPE="ext4" PARTLABEL="rootfs" PARTUUID="${u}1a"
/dev/mmcblk1p8: LABEL="bootfs" UUID="${u}28" TYPE="ext4" PARTLABEL="bootfs" PARTUUID="${u}38"
X
    : > "$ROOT/writes.log"
    local n
    for n in fw_printenv fw_setenv findmnt lsblk blkid ssh sudo reboot systemctl; do
        printf '#!/usr/bin/env bash\nexec "%s" _shim %s "$(dirname "$(dirname "$(readlink -f "$0")")")" "$@"\n' \
            "$SELF" "$n" > "$ROOT/bin/$n"
        chmod +x "$ROOT/bin/$n"
    done
}

is_protected() {
    local p
    case $1 in mp2z_* | yako_*) return 0 ;; esac
    for p in $PROTECTED; do [ "$1" = "$p" ] && return 0; done
    return 1
}

check_boot_path() { # PATH -> prints the real path, or logs a VIOLATION and fails
    local p=$1 r=
    case $p in
    /boot | /boot/*) r=$ROOT$p ;;
    "$ROOT"/boot | "$ROOT"/boot/*) r=$p ;;
    esac
    case $p in *..*) r= ;; esac
    if [ -z "$r" ]; then log "VIOLATION file $p outside $ROOT/boot"; return 1; fi
    printf '%s\n' "$r"
}

fb_put() {
    local r
    r=$(check_boot_path "$1") || { echo "mp2s_fakeboard: VIOLATION: $1" >&2; return 1; }
    mkdir -p "$(dirname "$r")" && cat > "$r" || return 1
    log "file ${r#"$ROOT"} $(sha256sum < "$r" | cut -d' ' -f1)"
}

# fw_setenv NAME [VALUE] | -s FILE: every name checked first, then one edit
fw_setenv_shim() {
    local img=$ROOT/env/u-boot-env.bin script= line name val
    local -a names=() vals=() sets=() unsets=()
    if [ "${1:-}" = -s ]; then
        [ $# = 2 ] || die "fw_setenv -s FILE"
        if [ "$2" = - ]; then script=$(cat); else script=$(cat -- "$2") || exit 1; fi
        while IFS= read -r line; do
            line=${line#"${line%%[![:space:]]*}"}
            case $line in '' | '#'*) continue ;; esac
            name=${line%%[[:space:]]*}; val=${line#"$name"}; val=${val#"${val%%[![:space:]]*}"}
            names+=("$name"); vals+=("$val")
        done <<< "$script"
    else
        [ $# -ge 1 ] || die "fw_setenv NAME [VALUE] | -s FILE"
        name=$1; shift; names=("$name"); vals=("$*")
    fi
    local i bad=0
    for i in "${!names[@]}"; do
        is_protected "${names[$i]}" || continue
        log "VIOLATION setenv ${names[$i]} ${vals[$i]}"
        echo "mp2s_fakeboard: VIOLATION: write of ${names[$i]} refused" >&2
        bad=1
    done
    [ $bad = 0 ] || exit 1
    for i in "${!names[@]}"; do
        if [ -z "${vals[$i]}" ]; then
            # an unset of an absent name is not an error, as fw_setenv
            envtool print "$img" "${names[$i]}" >/dev/null 2>&1 && unsets+=("${names[$i]}")
        else
            sets+=("${names[$i]}=${vals[$i]}")
        fi
    done
    # one environment write: set and unset on a scratch file, one rename
    cp "$img" "$img.new" || exit 1
    if [ ${#sets[@]} -gt 0 ]; then
        envtool set "$img.new" "$img.new" "${sets[@]}" >/dev/null || { rm -f "$img.new"; exit 1; }
    fi
    if [ ${#unsets[@]} -gt 0 ]; then
        envtool unset "$img.new" "$img.new" "${unsets[@]}" >/dev/null || { rm -f "$img.new"; exit 1; }
    fi
    mv "$img.new" "$img"
    for i in "${!names[@]}"; do log "setenv ${names[$i]} ${vals[$i]}"; done
}

# refuse an ssh command that would write to the host outside DIR
host_write_guard() {
    local cmd=$1 t
    while read -r t; do
        [ -n "$t" ] || continue
        case $t in /dev/null | "$ROOT" | "$ROOT"/*) continue ;; esac
        log "VIOLATION file $t outside $ROOT/boot (not run)"
        echo "mp2s_fakeboard: VIOLATION: ssh command writes $t" >&2
        return 1
    done < <(grep -oE '(>>?|of=|tee( -a)?)[[:space:]]*/[^[:space:];|&)]*' <<< "$cmd" |
        sed -E 's/^(>>?|of=|tee( -a)?)[[:space:]]*//')
}

shim() {
    local what=$1; ROOT=$2; shift 2
    local img=$ROOT/env/u-boot-env.bin
    case $what in
    fw_printenv) envtool print "$img" "$@" ;;
    fw_setenv) fw_setenv_shim "$@" ;;
    findmnt)
        case " $* " in
        *" -o SOURCE "* | *" -oSOURCE "*) awk 'NR==2{print $2}' "$ROOT/captured/findmnt.txt" ;;
        *) cat "$ROOT/captured/findmnt.txt" ;;
        esac ;;
    lsblk) cat "$ROOT/captured/lsblk.txt" ;;
    blkid)
        if [ $# -gt 0 ] && [ "${1#-}" = "$1" ]; then grep "^$1:" "$ROOT/captured/blkid.txt"
        else cat "$ROOT/captured/blkid.txt"; fi ;;
    reboot) log reboot ;;
    systemctl)
        case ${1:-} in
        reboot | poweroff) log "$1" ;;
        *) echo "mp2s_fakeboard: systemctl $* not faked" >&2; exit 1 ;;
        esac ;;
    sudo)
        while [ $# -gt 0 ]; do case $1 in --) shift; break ;; -*) shift ;; *) break ;; esac; done
        [ $# -ge 1 ] || die "sudo CMD"
        PATH="$ROOT/bin:$PATH" exec "$@" ;;
    ssh)
        # ssh [-o X | -p N | -i F ...] HOST CMD...: run CMD here
        while [ $# -gt 0 ]; do
            case $1 in
            -[opiFlJLRDEBbcmSWwQ]) shift 2 ;;
            -*) shift ;;
            *) break ;;
            esac
        done
        [ $# -ge 2 ] || die "ssh HOST CMD"
        shift
        local cmd="$*"
        cmd=${cmd//\/boot/$ROOT/boot}
        cmd=${cmd//\/sys\/block\//$ROOT/sys/block/}
        host_write_guard "$cmd" || exit 1
        manifest > "$ROOT/.manifest"
        local rc=0 viol=0 h f after
        (cd "$ROOT" && PATH="$ROOT/bin:$PATH" bash -c "$cmd") || rc=$?
        after=$(manifest)
        while read -r h f; do # changed or new files
            [ -n "$f" ] || continue
            grep -qxF "$h  $f" "$ROOT/.manifest" && continue
            case $f in
            boot/*) log "file /$f $h" ;;
            *) log "VIOLATION file /$f outside $ROOT/boot"; viol=1 ;;
            esac
        done <<< "$after"
        while read -r h f; do # removed files
            [ -n "$f" ] || continue
            grep -qF "  $f" <<< "$after" && continue
            case $f in
            boot/*) log "file /$f removed" ;;
            *) log "VIOLATION file /$f removed outside $ROOT/boot"; viol=1 ;;
            esac
        done < "$ROOT/.manifest"
        rm -f "$ROOT/.manifest"
        [ $viol = 0 ] || { echo "mp2s_fakeboard: VIOLATION in ssh command" >&2; exit 1; }
        exit $rc ;;
    esac
}

case ${1:-} in
init)   [ $# = 2 ] || die "init DIR"; mkdir -p "$2" || die "mkdir $2"; fb_init "$(cd "$2" && pwd)" ;;
shell)  { [ $# = 2 ] && [ -d "$2/bin" ]; } || die "shell DIR (run init first)"
        r=$(cd "$2" && pwd)
        # shellcheck disable=SC2016 # $PATH is expanded by the caller's eval
        printf 'export PATH=%q:$PATH\nexport MP2S_FAKE_ROOT=%q\n' "$r/bin" "$r" ;;
writes) { [ $# = 2 ] && [ -f "$2/writes.log" ]; } || die "writes DIR"; cat "$2/writes.log" ;;
put)    [ $# = 3 ] || die "put DIR PATH"; ROOT=$(cd "$2" && pwd); fb_put "$3" ;;
_shim)  shift; shim "$@" ;;
*)      sed -n '7,10p' "$SELF" | sed 's/^# \{0,1\}//' >&2; exit 2 ;;
esac

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
# DIR holds boot/ (stock extlinux, Image, DTB: placeholder bytes), env/u-boot-env.bin
# (built with tools/uboot_env.py), captured/{findmnt,lsblk,blkid}.txt (the microSD's
# bootfs on /boot, generic serials) and bin/ (shims: fw_printenv fw_setenv findmnt
# lsblk blkid ssh). `ssh [opts] HOST CMD...` runs CMD locally with /boot mapped to
# DIR/boot. Every write is appended to DIR/writes.log:
#   setenv NAME VALUE | file PATH sha256 | file PATH removed | VIOLATION ...
# A write of bootcmd/bootcount/altbootcmd, or of a path outside DIR/boot, is
# logged VIOLATION and exits non-zero (an ssh command is judged after it ran).
set -u
SELF=$(readlink -f "${BASH_SOURCE[0]}")
TOOLS=$(dirname "$SELF")
PROTECTED="bootcmd bootcount altbootcmd"

die() { echo "mp2s_fakeboard: $*" >&2; exit 2; }
log() { printf '%s\n' "$*" >> "$ROOT/writes.log"; }

# sha256 manifest of everything under ROOT a command may change, minus our own state
manifest() {
    (cd "$ROOT" && find . -type f ! -path ./writes.log ! -path './bin/*' \
        ! -path './env/*' ! -path './captured/*' ! -path ./.manifest -print0 |
        sort -z | xargs -0 -r sha256sum | sed 's/  \.\//  /')
}

fb_init() {
    ROOT=$1; [ -n "$ROOT" ] || die "init DIR"
    mkdir -p "$ROOT/boot/extlinux" "$ROOT/env" "$ROOT/captured" "$ROOT/bin" || die "mkdir"
    printf 'placeholder kernel Image\n' > "$ROOT/boot/Image"
    printf 'placeholder device tree\n' > "$ROOT/boot/stm32mp257f-dk.dtb"
    cat > "$ROOT/boot/extlinux/extlinux.conf" <<'X'
default stock
timeout 1
label stock
    kernel /Image
    fdt /stm32mp257f-dk.dtb
    append root=PARTLABEL=rootfs rootwait
X
    # env image: a base copy with one variable, then the stock variables via uboot_env.py
    python3 -I - "$ROOT/env/base.bin" <<'X' || die "env base"
import sys, zlib
size = 0x10000
data = b"fake_board=1\0\0".ljust(size - 4, b"\0")
open(sys.argv[1], "wb").write(zlib.crc32(data).to_bytes(4, "little") + data)
X
    python3 -I "$TOOLS/uboot_env.py" set "$ROOT/env/base.bin" "$ROOT/env/u-boot-env.bin" \
        'bootcmd=run bootcmd_stm32mp' 'bootcount=0' 'bootlimit=1' \
        'altbootcmd=run bootcmd_stm32mp' 'boot_prefixes=/mmc0_' \
        'boot_targets=mmc0' 'boot_syslinux_conf=extlinux/extlinux.conf' >/dev/null \
        || die "uboot_env.py set failed"
    rm -f "$ROOT/env/base.bin"
    local u=00000000-0000-0000-0000-0000000000
    cat > "$ROOT/captured/findmnt.txt" <<X
TARGET SOURCE         FSTYPE OPTIONS
/boot  /dev/mmcblk0p8 ext4   rw,relatime
X
    cat > "$ROOT/captured/lsblk.txt" <<'X'
NAME         LABEL      TYPE PARTLABEL   SIZE  MOUNTPOINT
mmcblk0                 disk                 14.8G
|-mmcblk0p1  fsbla1     part fsbla1      256K
|-mmcblk0p2  fsbla2     part fsbla2      256K
|-mmcblk0p3  metadata1  part metadata1   256K
|-mmcblk0p4  metadata2  part metadata2   256K
|-mmcblk0p5  fip-a      part fip-a         4M
|-mmcblk0p6  fip-b      part fip-b         4M
|-mmcblk0p7  u-boot-env part u-boot-env  512K
|-mmcblk0p8  bootfs     part bootfs       64M  /boot
|-mmcblk0p9  vendorfs   part vendorfs     16M
|-mmcblk0p10 rootfs     part rootfs        4G  /
`-mmcblk0p11 userfs     part userfs        8G  /usr/local
X
    cat > "$ROOT/captured/blkid.txt" <<X
/dev/mmcblk0p7: PARTLABEL="u-boot-env" PARTUUID="${u}07"
/dev/mmcblk0p8: LABEL="bootfs" UUID="${u}08" TYPE="ext4" PARTLABEL="bootfs" PARTUUID="${u}18"
/dev/mmcblk0p10: LABEL="rootfs" UUID="${u}0a" TYPE="ext4" PARTLABEL="rootfs" PARTUUID="${u}1a"
X
    : > "$ROOT/writes.log"
    local n
    for n in fw_printenv fw_setenv findmnt lsblk blkid ssh; do
        printf '#!/usr/bin/env bash\nexec "%s" _shim %s "$(dirname "$(dirname "$(readlink -f "$0")")")" "$@"\n' \
            "$SELF" "$n" > "$ROOT/bin/$n"
        chmod +x "$ROOT/bin/$n"
    done
}

check_boot_path() { # PATH -> prints the real path, or logs a VIOLATION and fails
    local p=$1 r=
    case $p in
    /boot|/boot/*) r=$ROOT$p ;;
    $ROOT/boot|$ROOT/boot/*) r=$p ;;
    esac
    case $p in *..*) r= ;; esac
    if [ -z "$r" ]; then log "VIOLATION file $p outside $ROOT/boot"; return 1; fi
    printf '%s\n' "$r"
}

fb_put() {
    local r; r=$(check_boot_path "$1") || { echo "mp2s_fakeboard: VIOLATION: $1" >&2; return 1; }
    mkdir -p "$(dirname "$r")" && cat > "$r" || return 1
    log "file ${r#$ROOT} $(sha256sum < "$r" | cut -d' ' -f1)"
}

shim() {
    local what=$1; ROOT=$2; shift 2
    local img=$ROOT/env/u-boot-env.bin
    case $what in
    fw_printenv) python3 -I "$TOOLS/uboot_env.py" print "$img" "$@" ;;
    fw_setenv)
        [ $# -ge 1 ] || die "fw_setenv NAME [VALUE]"
        local name=$1; shift; local val="$*" p
        for p in $PROTECTED; do
            if [ "$name" = "$p" ]; then
                log "VIOLATION setenv $name $val"
                echo "mp2s_fakeboard: VIOLATION: write of $name refused" >&2
                exit 1
            fi
        done
        if [ -z "$val" ]; then
            python3 -I "$TOOLS/uboot_env.py" unset "$img" "$img.new" "$name" >/dev/null || exit 1
        else
            python3 -I "$TOOLS/uboot_env.py" set "$img" "$img.new" "$name=$val" >/dev/null || exit 1
        fi
        mv "$img.new" "$img"; log "setenv $name $val" ;;
    findmnt)
        case " $* " in
        *" -o SOURCE "*) awk 'NR==2{print $2}' "$ROOT/captured/findmnt.txt" ;;
        *) cat "$ROOT/captured/findmnt.txt" ;;
        esac ;;
    lsblk) cat "$ROOT/captured/lsblk.txt" ;;
    blkid)
        if [ $# -gt 0 ] && [ "${1#-}" = "$1" ]; then grep "^$1:" "$ROOT/captured/blkid.txt"
        else cat "$ROOT/captured/blkid.txt"; fi ;;
    ssh)
        # ssh [-o X | -p N | -i F ...] HOST CMD...: run CMD here, /boot -> ROOT/boot
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
shell)  [ $# = 2 ] && [ -d "$2/bin" ] || die "shell DIR (run init first)"
        r=$(cd "$2" && pwd)
        printf 'export PATH=%q:$PATH\nexport MP2S_FAKE_ROOT=%q\n' "$r/bin" "$r" ;;
writes) [ $# = 2 ] && [ -f "$2/writes.log" ] || die "writes DIR"; cat "$2/writes.log" ;;
put)    [ $# = 3 ] || die "put DIR PATH"; ROOT=$(cd "$2" && pwd); fb_put "$3" ;;
_shim)  shift; shim "$@" ;;
*)      sed -n '5,9p' "$SELF" | sed 's/^# \{0,1\}//' >&2; exit 2 ;;
esac

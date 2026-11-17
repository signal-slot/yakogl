#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_fakeboard.sh - a fake STM32MP257F-DK for the no-board selftests of the
# DK standalone runner (tools/board-stm32mp2-standalone.sh) and the session
# driver (docs/notes/stm32mp257f-dk-standalone.md). No ssh, no board, no real
# identifier: every serial, UUID and address here is a generic placeholder.
#
#   tools/mp2s_fakeboard.sh init DIR      make the fake root in DIR
#   tools/mp2s_fakeboard.sh shell DIR     print the PATH and environment
#                                         settings that point a runner at it
#                                         (eval "$(... shell DIR)")
#   tools/mp2s_fakeboard.sh writes DIR    print DIR/writes.log
#
# Sourced (`. tools/mp2s_fakeboard.sh`), it defines mp2s_fakeboard_init DIR,
# mp2s_fakeboard_shell DIR and mp2s_fakeboard_writes DIR and runs nothing.
#
# DIR, after init:
#   boot/        the microSD's bootfs as ST ships it (note, sections 1-2; the
#                one-shot note's listing): Image.gz, stm32mp257f-dk.dtb,
#                boot.scr.uimg, extlinux/extlinux.conf, mmc0_extlinux/ and
#                mmc1_extlinux/extlinux.conf. Placeholder bytes, not a kernel.
#   dev/mmcblk1p7  the card's u-boot-env partition: 512 KiB, a redundant pair
#                of CONFIG_ENV_SIZE 0x2000 copies, copy 1 at 0x7e000 and copy
#                2 at 0x7c000 (the runner's ENVLAY). Built from ST's default
#                environment (tools/testdata/mp2s-env/pristine.bin, else the
#                text fixture under driver/tools/mp2-standalone/st-env/) with
#                tools/uboot_env.py, as the stock boot.scr.uimg leaves it after
#                its one `env save`: boot_prefixes=/mmc0_, boot_targets=mmc0,
#                boot_syslinux_conf=extlinux/extlinux.conf (note, section 2).
#                It keeps ST's bootcmd and altbootcmd; there is no bootcount
#                (CONFIG_BOOTCOUNT_LIMIT is not set, note section 3).
#   dev/mmcblk0p7  the eMMC's u-boot-env: ST's defaults, never saved. A
#                runner that reads the wrong device sees a different env.
#   dev/mmcblk1p1 ... p6  fsbla1/2, metadata1/2 (FWU metadata v2, bank 0
#                accepted), fip-a/b: small placeholders (not the card's sizes)
#   captured/    parts.tsv (the fake's partition table: both disks, ST's
#                labels, generic UUIDs) and the captured outputs findmnt.txt,
#                lsblk.txt, blkid.txt, plus bootfs-check/ in the forms
#                tools/mp2s_bootfs_check.py reads
#   sys/, proc/, etc/  what the runner's preflight reads: sys/block/<disk>/
#                device/type (mmcblk0 MMC, mmcblk1 SD), the partitions' sysfs
#                (partition, size, uevent PARTNAME), sys/class/block links,
#                proc/self/mountinfo and proc/mounts, proc/sys/kernel/random/
#                boot_id, proc/uptime, proc/sys/vm/drop_caches (a sink),
#                etc/fw_env.config (the card's u-boot-env, both copies),
#                etc/os-release
#   bin/         the shims, first in PATH after `shell`: fw_printenv fw_setenv
#                findmnt lsblk blkid ssh sudo sync uname reboot systemctl
#   writes.log   every write, one line each (below)
#   README.txt   this layout, for a reader of a selftest's leftovers
#
# The shims. fw_printenv [-n] [NAME...] and fw_setenv NAME [VALUE...] |
# fw_setenv -s FILE (- is stdin) work on dev/mmcblk1p7 through
# tools/uboot_env.py. A script line is NAME=VALUE (the runner's form,
# libubootenv's) or NAME VALUE (U-Boot's fw_env); NAME= or a bare NAME
# deletes. One script is one environment write: the inactive copy is written
# with flags = active + 1 and the active copy is left alone, as env_save and
# fw_setenv do. findmnt, lsblk and blkid answer from captured/parts.tsv in
# the forms the runner and tools/mp2s_bootfs_check.py use (findmnt -n -v -o
# SOURCE,FSTYPE -T /boot; lsblk -rno PATH,PARTLABEL -- DEV; lsblk -P -o
# NAME,PKNAME,TYPE,FSTYPE,LABEL,PARTLABEL; blkid [-s TAG] [-o value] [DEV]).
# sudo [-n] CMD runs CMD; sync does nothing; reboot and systemctl
# reboot|poweroff only log; uname -r answers a fixed version.
#
# `ssh [options] HOST CMD...` runs CMD here, with cwd DIR, the shims first in
# PATH and the board paths /boot, /dev/mmcblk*, /sys/block, /sys/class/block,
# /etc/fw_env.config, /etc/os-release and the proc files above mapped into
# DIR (`--root /` becomes `--root DIR`, for tools/mp2s_fwenv_check.py). HOST
# is not looked at. Before CMD runs it is parsed, `sh -c '...'` included, and
# every path it would write (a redirect, tee, dd of=, the arguments of rm
# rmdir mv mkdir touch ln truncate ..., the destination of cp and install,
# find -delete/-exec, sed -i) must be strictly inside DIR/boot, or /dev/null
# or /proc/sys/vm/drop_caches: otherwise nothing runs. mount, umount and a
# write verb fed by xargs are refused outright. After CMD ran, a sha256
# manifest of DIR before and after names every file that changed: under
# boot/ it is logged as a write; anywhere else it is a VIOLATION, and so is
# dev/mmcblk1p7 changed by anything but fw_setenv. This is a check on the
# commands a runner sends, not a sandbox: a program that writes outside DIR
# by a path the parse cannot see is not stopped.
#
# writes.log lines:
#   setenv NAME VALUE            (a deletion: `setenv NAME`)
#   file /boot/PATH SHA256       (a new or changed file; `file /boot/PATH removed`)
#   reboot | poweroff
#   VIOLATION setenv NAME VALUE  (refused: bootcmd, bootcmd_*, bootcount,
#                                 bootlimit, upgrade_available, altbootcmd,
#                                 boot_targets, boot_syslinux_conf,
#                                 boot_extlinux, boot_scripts, preboot,
#                                 mp2z_*, yako_*: note, section 3)
#   VIOLATION file PATH outside /boot [(not run)]
#   VIOLATION env u-boot-env written outside fw_setenv
#   VIOLATION ssh ...            (a command that does not parse, mount, umount,
#                                 a write verb fed by xargs; not run)
# A refused fw_setenv writes nothing, prints "VIOLATION" on stderr and exits
# 1; a refused ssh command exits 1 before running; a VIOLATION found after a
# command ran exits 1 as well.
#
# Without tools/uboot_env.py beside this script (MP2S_FAKEBOARD_TOOLS names
# another tools directory), the environment is the text file
# env/u-boot-env.txt (NAME=VALUE lines) plus env/NOTE.txt, and dev/mmcblk1p7
# is not made; fw_printenv and fw_setenv then read and edit the text.
# MP2S_FAKEBOARD_REPO names the repository root for the fixtures (default:
# the parent of the tools directory). DIR must not be moved after init: the
# shims hold its absolute path.
set -u

MP2S_FB_SELF=$(readlink -f "${BASH_SOURCE[0]}")
MP2S_FB_TOOLS=${MP2S_FAKEBOARD_TOOLS:-$(dirname "$MP2S_FB_SELF")}
MP2S_FB_REPO=${MP2S_FAKEBOARD_REPO:-$(cd "$MP2S_FB_TOOLS/.." 2>/dev/null && pwd)}
MP2S_FB_ENVTOOL=$MP2S_FB_TOOLS/uboot_env.py
MP2S_FB_ST_TEXT=$MP2S_FB_REPO/driver/tools/mp2-standalone/st-env/stm32mp25-dk-initial-env.txt
MP2S_FB_PRISTINE=$MP2S_FB_REPO/tools/testdata/mp2s-env/pristine.bin
# the card's u-boot-env as the runner reads it: 512 KiB, copy 1 at the top
MP2S_FB_ENV_SIZE=0x2000
MP2S_FB_ENV_OFF1=0x7e000
MP2S_FB_ENV_OFF2=0x7c000
MP2S_FB_PART_BYTES=524288
MP2S_FB_LAYOUT=(--redundant --size "$MP2S_FB_ENV_SIZE" --offset "$MP2S_FB_ENV_OFF1" --offset2 "$MP2S_FB_ENV_OFF2")
MP2S_FB_SD=mmcblk1
MP2S_FB_EMMC=mmcblk0
# the variables the design never writes (note, section 3): exact names, and
# the prefixes bootcmd_ mp2z_ yako_
MP2S_FB_PROTECTED="bootcmd bootcount bootlimit upgrade_available altbootcmd boot_targets boot_syslinux_conf boot_extlinux boot_scripts preboot"
MP2S_FB_SHIMS="fw_printenv fw_setenv findmnt lsblk blkid ssh sudo sync uname reboot systemctl"
# what the stock boot.scr.uimg saves on its one run (note, section 2)
MP2S_FB_SAVED=(boot_prefixes=/mmc0_ boot_targets=mmc0 boot_syslinux_conf=extlinux/extlinux.conf)

fb_die() { echo "mp2s_fakeboard: $*" >&2; exit 2; }
fb_log() { printf '%s\n' "$*" >> "$ROOT/writes.log"; }
fb_envtool() { python3 -I "$MP2S_FB_ENVTOOL" "${MP2S_FB_LAYOUT[@]}" "$@"; }
fb_have_envtool() { [ -f "$MP2S_FB_ENVTOOL" ]; }

# ---- init ---------------------------------------------------------------------------

# the partition table of both disks: NAME PKNAME TYPE SECTORS FSTYPE LABEL
# PARTLABEL MOUNTPOINT UUID PARTUUID ("-" is empty). The eMMC carries ST's
# layout with the same labels as the card (note, section 1) and nothing
# mounted. The sizes are the DK's; the UUIDs are placeholders.
fb_parts_tsv() {
    local d n u
    printf 'NAME\tPKNAME\tTYPE\tSECTORS\tFSTYPE\tLABEL\tPARTLABEL\tMOUNTPOINT\tUUID\tPARTUUID\n'
    for d in $MP2S_FB_EMMC $MP2S_FB_SD; do
        if [ "$d" = "$MP2S_FB_SD" ]; then n=1; u=00000000-0000-4000-8000-0000000001; else n=0; u=00000000-0000-4000-8000-0000000000; fi
        if [ "$d" = "$MP2S_FB_SD" ]; then
            printf '%s\t-\tdisk\t%s\t-\t-\t-\t-\t-\t-\n' "$d" 31116288
        else
            printf '%s\t-\tdisk\t%s\t-\t-\t-\t-\t-\t-\n' "$d" 61071360
        fi
        printf '%sp1\t%s\tpart\t512\t-\t-\tfsbla1\t-\t-\t%s01\n' "$d" "$d" "$u"
        printf '%sp2\t%s\tpart\t512\t-\t-\tfsbla2\t-\t-\t%s02\n' "$d" "$d" "$u"
        printf '%sp3\t%s\tpart\t512\t-\t-\tmetadata1\t-\t-\t%s03\n' "$d" "$d" "$u"
        printf '%sp4\t%s\tpart\t512\t-\t-\tmetadata2\t-\t-\t%s04\n' "$d" "$d" "$u"
        printf '%sp5\t%s\tpart\t8192\t-\t-\tfip-a\t-\t-\t%s05\n' "$d" "$d" "$u"
        printf '%sp6\t%s\tpart\t8192\t-\t-\tfip-b\t-\t-\t%s06\n' "$d" "$d" "$u"
        printf '%sp7\t%s\tpart\t1024\t-\t-\tu-boot-env\t-\t-\t%s07\n' "$d" "$d" "$u"
        if [ "$n" = 1 ]; then
            printf '%sp8\t%s\tpart\t131072\text4\tbootfs\tbootfs\t/boot\t%s18\t%s08\n' "$d" "$d" "$u" "$u"
            printf '%sp9\t%s\tpart\t32768\text4\tvendorfs\tvendorfs\t-\t%s19\t%s09\n' "$d" "$d" "$u" "$u"
            printf '%sp10\t%s\tpart\t8388608\text4\trootfs\trootfs\t/\t%s1a\t%s0a\n' "$d" "$d" "$u" "$u"
            printf '%sp11\t%s\tpart\t16777216\text4\tuserfs\tuserfs\t/usr/local\t%s1b\t%s0b\n' "$d" "$d" "$u" "$u"
        else
            printf '%sp8\t%s\tpart\t131072\text4\tbootfs\tbootfs\t-\t%s18\t%s08\n' "$d" "$d" "$u" "$u"
            printf '%sp9\t%s\tpart\t32768\text4\tvendorfs\tvendorfs\t-\t%s19\t%s09\n' "$d" "$d" "$u" "$u"
            printf '%sp10\t%s\tpart\t8388608\text4\trootfs\trootfs\t-\t%s1a\t%s0a\n' "$d" "$d" "$u" "$u"
            printf '%sp11\t%s\tpart\t16777216\text4\tuserfs\tuserfs\t-\t%s1b\t%s0b\n' "$d" "$d" "$u" "$u"
        fi
    done
}

# fb_sysfs: sys/block, sys/class/block, proc/*/mount*, from parts.tsv
fb_sysfs() {
    local name pk type sectors fstype label plabel mnt rest maj=179 minor=0 id=20
    local mi="$ROOT/proc/self/mountinfo" mo="$ROOT/proc/mounts"
    mkdir -p "$ROOT/sys/class/block" "$ROOT/proc/self" || return 1
    : > "$mi"; : > "$mo"
    while IFS=$'\t' read -r name pk type sectors fstype label plabel mnt rest; do
        [ "$name" = NAME ] && continue
        [ -n "$name" ] || continue
        if [ "$type" = disk ]; then
            mkdir -p "$ROOT/sys/block/$name/device" || return 1
            echo "$sectors" > "$ROOT/sys/block/$name/size"
            printf 'MAJOR=%s\nMINOR=%s\nDEVNAME=%s\nDEVTYPE=disk\n' $maj $minor "$name" > "$ROOT/sys/block/$name/uevent"
            if [ "$name" = "$MP2S_FB_SD" ]; then echo SD; else echo MMC; fi > "$ROOT/sys/block/$name/device/type"
            ln -sfn "../../block/$name" "$ROOT/sys/class/block/$name"
        else
            mkdir -p "$ROOT/sys/block/$pk/$name" || return 1
            echo "${name##*p}" > "$ROOT/sys/block/$pk/$name/partition"
            echo "$sectors" > "$ROOT/sys/block/$pk/$name/size"
            printf 'MAJOR=%s\nMINOR=%s\nDEVNAME=%s\nDEVTYPE=partition\nPARTN=%s\nPARTNAME=%s\n' \
                $maj $minor "$name" "${name##*p}" "$plabel" > "$ROOT/sys/block/$pk/$name/uevent"
            ln -sfn "../../block/$pk/$name" "$ROOT/sys/class/block/$name"
            if [ "$mnt" != - ]; then
                id=$((id + 1))
                printf '%s 1 %s:%s / %s rw,relatime shared:%s - %s /dev/%s rw\n' $id $maj $minor "$mnt" $id "$fstype" "$name" >> "$mi"
                printf '/dev/%s %s %s rw,relatime 0 0\n' "$name" "$mnt" "$fstype" >> "$mo"
            fi
        fi
        minor=$((minor + 1))
    done < "$ROOT/captured/parts.tsv"
    cp "$mo" "$ROOT/proc/self/mounts"
}

# fb_fwu_metadata OUT: FWU metadata v2 (ST's U-Boot v2023.10), active bank 0,
# bank_state fc fc ff ff: both banks accepted, as tools/fwu_metadata.py reads
fb_fwu_metadata() {
    python3 -I - "$1" <<'PYEOF'
import struct, sys, zlib
body = struct.pack("<IIIIHH4sI", 2, 0, 0, 32, 0, 0, bytes.fromhex("fcfcffff"), 0)
with open(sys.argv[1], "wb") as fh:
    fh.write(struct.pack("<I", zlib.crc32(body)) + body + b"\xff" * (4096 - 4 - len(body)))
PYEOF
}

# fb_env_image OUT: the card's u-boot-env partition (512 KiB, ST's defaults
# in both copies) from the pristine fixture, else from the text fixture
fb_env_image() {
    local out=$1
    head -c "$MP2S_FB_PART_BYTES" /dev/zero > "$out" || return 1
    if [ -s "$MP2S_FB_PRISTINE" ] && [ "$(stat -c %s "$MP2S_FB_PRISTINE")" = 16384 ]; then
        # copy 1 is the fixture's first 0x2000 bytes, copy 2 its second
        dd if="$MP2S_FB_PRISTINE" of="$out" bs=8192 count=1 skip=0 seek=63 conv=notrunc status=none &&
            dd if="$MP2S_FB_PRISTINE" of="$out" bs=8192 count=1 skip=1 seek=62 conv=notrunc status=none || return 1
        fb_envtool check "$out" > /dev/null || return 1
        return 0
    fi
    [ -r "$MP2S_FB_ST_TEXT" ] || { echo "mp2s_fakeboard: neither $MP2S_FB_PRISTINE nor $MP2S_FB_ST_TEXT" >&2; return 1; }
    # an empty valid pair (both flags 0), then every ST variable through
    # uboot_env.py, twice, so that both copies hold them
    python3 -I - "$out" "$MP2S_FB_ENV_SIZE" "$MP2S_FB_ENV_OFF1" "$MP2S_FB_ENV_OFF2" <<'PYEOF' || return 1
import sys, zlib
out, size, off1, off2 = sys.argv[1], int(sys.argv[2], 0), int(sys.argv[3], 0), int(sys.argv[4], 0)
area = b"\0" * (size - 5)
copy = zlib.crc32(area).to_bytes(4, "little") + b"\0" + area
buf = bytearray(open(out, "rb").read())
buf[off1:off1 + size] = copy
buf[off2:off2 + size] = copy
open(out, "wb").write(buf)
PYEOF
    local -a vars=()
    local line
    while IFS= read -r line; do
        case $line in '' | '#'* | *=) continue ;; esac
        vars+=("$line")
    done < "$MP2S_FB_ST_TEXT"
    fb_envtool set "$out" "$out" "${vars[@]}" > /dev/null && fb_envtool set "$out" "$out" "${vars[@]}" > /dev/null
}

# fb_env_text OUT: the text fallback, ST's defaults with the saved edits
fb_env_text() {
    [ -r "$MP2S_FB_ST_TEXT" ] || { echo "mp2s_fakeboard: no $MP2S_FB_ST_TEXT" >&2; return 1; }
    # one line per name, the last value of a repeated name, as U-Boot's
    # import keeps it; the saved variables replace ST's at the end
    { cat "$MP2S_FB_ST_TEXT"; printf '%s\n' "${MP2S_FB_SAVED[@]}"; } |
        awk -F= '/^#/ || !NF || $2 == "" { next } { if (!($1 in v)) order[++n] = $1; v[$1] = $0 }
                 END { for (i = 1; i <= n; i++) print v[order[i]] }' > "$1"
}

fb_captured() {
    local c=$ROOT/captured
    fb_storage findmnt /boot > "$c/findmnt.txt"
    fb_storage lsblk > "$c/lsblk.txt"
    fb_storage blkid > "$c/blkid.txt"
    mkdir -p "$c/bootfs-check"
    fb_storage findmnt -n -o SOURCE /boot > "$c/bootfs-check/findmnt.txt"
    fb_storage lsblk -P -o NAME,PKNAME,TYPE,FSTYPE,LABEL,PARTLABEL > "$c/bootfs-check/lsblk.txt"
    fb_storage blkid > "$c/bootfs-check/blkid.txt"
    cat "$ROOT/sys/block/$MP2S_FB_SD/device/type" > "$c/bootfs-check/disk-type.txt"
}

fb_readme() {
    cat > "$ROOT/README.txt" <<EOF
A fake STM32MP257F-DK made by tools/mp2s_fakeboard.sh init (yakogl). Nothing
here comes from a board: placeholder files, generic UUIDs, a fixed uname.

boot/                 the microSD's bootfs as mounted on /boot
dev/${MP2S_FB_SD}p7         the card's u-boot-env (512 KiB): copies of $MP2S_FB_ENV_SIZE bytes at
                      $MP2S_FB_ENV_OFF1 (copy 1) and $MP2S_FB_ENV_OFF2 (copy 2). Read it with
                      python3 -I tools/uboot_env.py ${MP2S_FB_LAYOUT[*]} print dev/${MP2S_FB_SD}p7
dev/${MP2S_FB_EMMC}p7         the eMMC's u-boot-env (ST's defaults, never saved)
dev/${MP2S_FB_SD}p1..p6      fsbla1, fsbla2, metadata1, metadata2, fip-a, fip-b (placeholders)
captured/parts.tsv    the partition table the findmnt, lsblk and blkid shims answer from
captured/*.txt        captured outputs; bootfs-check/ in tools/mp2s_bootfs_check.py's forms
sys/ proc/ etc/       what a runner's preflight reads over ssh, mapped into here
bin/                  the shims ($MP2S_FB_SHIMS)
writes.log            every write; "VIOLATION" lines are refused writes
Only files under boot/ and the environment (through fw_setenv) may change.
EOF
}

mp2s_fakeboard_init() {
    [ $# = 1 ] || fb_die "init DIR"
    mkdir -p "$1" || fb_die "cannot make $1"
    ROOT=$(cd "$1" && pwd) || fb_die "cannot enter $1"
    local c d p
    mkdir -p "$ROOT"/boot/{extlinux,mmc0_extlinux,mmc1_extlinux} "$ROOT"/{captured,bin,dev,etc} \
        "$ROOT/proc/sys/vm" "$ROOT/proc/sys/kernel/random" || fb_die "mkdir under $ROOT"
    # the stock bootfs: placeholders, not a kernel
    printf 'placeholder: the kernel Image.gz of OpenSTLinux\n' > "$ROOT/boot/Image.gz"
    printf 'placeholder: the device tree stm32mp257f-dk.dtb\n' > "$ROOT/boot/stm32mp257f-dk.dtb"
    printf 'placeholder: ST'"'"'s stock boot.scr.uimg\n' > "$ROOT/boot/boot.scr.uimg"
    for c in extlinux mmc0_extlinux mmc1_extlinux; do
        cat > "$ROOT/boot/$c/extlinux.conf" <<'EOF'
# placeholder of ST's extlinux.conf (the real one is 302 bytes)
menu title Select the boot mode
TIMEOUT 20
DEFAULT stm32mp257f-dk
LABEL stm32mp257f-dk
	KERNEL /Image.gz
	FDTDIR /
	INITRD /initrd.img
	APPEND root=PARTUUID=00000000-0000-4000-8000-00000000010a rootwait rw console=ttySTM0,115200
EOF
    done
    fb_parts_tsv > "$ROOT/captured/parts.tsv"
    fb_sysfs || fb_die "sysfs"
    echo 00000000-0000-4000-8000-000000000001 > "$ROOT/proc/sys/kernel/random/boot_id"
    echo "100.00 390.00" > "$ROOT/proc/uptime"
    ln -sfn /dev/null "$ROOT/proc/sys/vm/drop_caches"
    printf 'ID=openstlinux-weston\nNAME="ST OpenSTLinux (fake board)"\nPRETTY_NAME="ST OpenSTLinux (fake board, tools/mp2s_fakeboard.sh)"\n' > "$ROOT/etc/os-release"
    {
        echo "# the fake DK: the microSD's u-boot-env, both copies (tools/mp2s_fwenv_check.py reads this)"
        echo "/dev/${MP2S_FB_SD}p7 $MP2S_FB_ENV_OFF1 $MP2S_FB_ENV_SIZE"
        echo "/dev/${MP2S_FB_SD}p7 $MP2S_FB_ENV_OFF2 $MP2S_FB_ENV_SIZE"
    } > "$ROOT/etc/fw_env.config"
    # the protected partitions: placeholders a read may fetch, never written
    for p in 1 2 5 6; do head -c 4096 /dev/zero > "$ROOT/dev/${MP2S_FB_SD}p$p"; done
    fb_fwu_metadata "$ROOT/dev/${MP2S_FB_SD}p3" || fb_die "FWU metadata"
    cp "$ROOT/dev/${MP2S_FB_SD}p3" "$ROOT/dev/${MP2S_FB_SD}p4"
    if fb_have_envtool; then
        fb_env_image "$ROOT/dev/${MP2S_FB_EMMC}p7" || fb_die "the environment image"
        cp "$ROOT/dev/${MP2S_FB_EMMC}p7" "$ROOT/dev/${MP2S_FB_SD}p7"
        fb_envtool set "$ROOT/dev/${MP2S_FB_SD}p7" "$ROOT/dev/${MP2S_FB_SD}p7" "${MP2S_FB_SAVED[@]}" > /dev/null ||
            fb_die "uboot_env.py set of the saved variables"
    else
        mkdir -p "$ROOT/env"
        fb_env_text "$ROOT/env/u-boot-env.txt" || fb_die "the environment text"
        cat > "$ROOT/env/NOTE.txt" <<EOF
No tools/uboot_env.py in $MP2S_FB_TOOLS: the environment is the text
u-boot-env.txt (NAME=VALUE lines), not a u-boot-env image, and dev/${MP2S_FB_SD}p7
does not exist. fw_printenv and fw_setenv read and edit the text; a runner
that reads the partition finds nothing.
EOF
    fi
    : > "$ROOT/writes.log"
    for c in $MP2S_FB_SHIMS; do
        printf '#!/usr/bin/env bash\nexec %q _shim %s %q "$@"\n' "$MP2S_FB_SELF" "$c" "$ROOT" > "$ROOT/bin/$c"
        chmod +x "$ROOT/bin/$c"
    done
    fb_captured
    fb_readme
    echo "mp2s_fakeboard: fake DK in $ROOT"
}

mp2s_fakeboard_shell() {
    { [ $# = 1 ] && [ -d "$1/bin" ]; } || fb_die "shell DIR (run init first)"
    local r
    r=$(cd "$1" && pwd) || fb_die "cannot enter $1"
    # shellcheck disable=SC2016  # $PATH is for the caller's eval
    printf 'export PATH=%q:$PATH\nexport MP2S_FAKE_ROOT=%q\n' "$r/bin" "$r"
}

mp2s_fakeboard_writes() {
    { [ $# = 1 ] && [ -f "$1/writes.log" ]; } || fb_die "writes DIR (run init first)"
    cat "$1/writes.log"
}

# ---- the storage shims: findmnt, lsblk, blkid from captured/parts.tsv ----------------

fb_storage() {
    python3 -I - "$ROOT" "$@" <<'PYEOF'
import os, sys

root, tool, args = sys.argv[1], sys.argv[2], sys.argv[3:]
rows, cols = [], None
with open(os.path.join(root, "captured", "parts.tsv")) as fh:
    for ln in fh:
        f = ln.rstrip("\n").split("\t")
        if cols is None:
            cols = f
            continue
        if f and f[0]:
            rows.append(dict(zip(cols, ["" if x == "-" else x for x in f])))


def fail(msg, rc=1):
    print("%s: %s" % (tool, msg), file=sys.stderr)
    sys.exit(rc)


def human(sectors):
    b = int(sectors) * 512
    for unit, div in (("G", 1 << 30), ("M", 1 << 20), ("K", 1 << 10)):
        if b >= div:
            v = b / div
            return ("%d%s" % (v, unit)) if v == int(v) else ("%.1f%s" % (v, unit))
    return "%dB" % b


def value(row, col, opts):
    if col in ("NAME", "KNAME"):
        return ("/dev/" + row["NAME"]) if opts.get("paths") else row["NAME"]
    if col == "PATH":
        return "/dev/" + row["NAME"]
    if col == "SIZE":
        return str(int(row["SECTORS"]) * 512) if opts.get("bytes") else human(row["SECTORS"])
    if col == "MOUNTPOINTS":
        return row["MOUNTPOINT"]
    if col in ("RM", "RO"):
        return "0"
    if col in row:
        return row[col]
    fail("unknown column: %s" % col)


def by_name(dev):
    name = os.path.basename(dev)
    for r in rows:
        if r["NAME"] == name:
            return r
    return None


def table(lines, header, noheadings, raw):
    """lines: list of value lists; padded columns unless raw."""
    out = [] if noheadings else [header]
    out += lines
    if raw:
        return [" ".join(l) for l in out]
    widths = [max(len(l[i]) for l in out) for i in range(len(header))]
    res = []
    for l in out:
        cells = [l[i].ljust(widths[i]) if i < len(header) - 1 else l[i] for i in range(len(header))]
        res.append(" ".join(cells).rstrip())
    return res


def lsblk(args):
    opts = {"cols": None, "nohead": False, "raw": False, "pairs": False, "nodeps": False,
            "paths": False, "list": False, "bytes": False}
    devs = []
    i = 0
    while i < len(args):
        a = args[i]
        if a == "--":
            devs += args[i + 1:]
            break
        if a.startswith("--output="):
            opts["cols"] = a.split("=", 1)[1]
        elif a == "--output":
            i += 1
            opts["cols"] = args[i]
        elif a in ("--noheadings", "--raw", "--pairs", "--nodeps", "--paths", "--list", "--bytes", "--ascii"):
            opts[{"--noheadings": "nohead", "--raw": "raw", "--pairs": "pairs", "--nodeps": "nodeps",
                  "--paths": "paths", "--list": "list", "--bytes": "bytes", "--ascii": "ascii"}[a]] = True
        elif a.startswith("-") and len(a) > 1:
            j = 1
            while j < len(a):
                ch = a[j]
                if ch == "o":
                    rest = a[j + 1:]
                    if rest:
                        opts["cols"] = rest
                    else:
                        i += 1
                        opts["cols"] = args[i]
                    break
                flag = {"n": "nohead", "r": "raw", "P": "pairs", "d": "nodeps", "p": "paths",
                        "l": "list", "b": "bytes", "i": "ascii", "a": "all"}.get(ch)
                if flag is None:
                    if ch == "f":
                        opts["cols"] = "NAME,FSTYPE,LABEL,UUID,MOUNTPOINT"
                    else:
                        fail("option -%s is not faked" % ch)
                else:
                    opts[flag] = True
                j += 1
        else:
            devs.append(a)
        i += 1
    cols = (opts["cols"] or "NAME,SIZE,TYPE,FSTYPE,LABEL,PARTLABEL,MOUNTPOINT").upper().split(",")
    if devs:
        sel = []
        for d in devs:
            r = by_name(d)
            if r is None:
                fail("%s: not a block device" % d, 32)
            sel.append(r)
            if r["TYPE"] == "disk" and not opts["nodeps"]:
                sel += [p for p in rows if p["PKNAME"] == r["NAME"]]
    else:
        sel = [r for r in rows if r["TYPE"] == "disk" or not opts["nodeps"]]
    lines = []
    for n, r in enumerate(sel):
        vals = [value(r, c, opts) for c in cols]
        if not (opts["raw"] or opts["pairs"] or opts["list"]) and r["TYPE"] == "part" and "NAME" in cols:
            last = n + 1 == len(sel) or sel[n + 1]["TYPE"] != "part"
            vals[cols.index("NAME")] = ("`-" if last else "|-") + vals[cols.index("NAME")]
        lines.append(vals)
    if opts["pairs"]:
        for vals in lines:
            print(" ".join('%s="%s"' % (c, v) for c, v in zip(cols, vals)))
        return
    for ln in table(lines, cols, opts["nohead"], opts["raw"]):
        print(ln)


def blkid(args):
    tags, fmt, devs = [], "full", []
    i = 0
    while i < len(args):
        a = args[i]
        if a == "--":
            devs += args[i + 1:]
            break
        if a == "-s":
            i += 1
            tags.append(args[i].upper())
        elif a.startswith("-s") and len(a) > 2:
            tags.append(a[2:].upper())
        elif a == "-o":
            i += 1
            fmt = args[i]
        elif a.startswith("-o") and len(a) > 2:
            fmt = a[2:]
        elif a in ("-p", "-g", "-i", "-d", "-k"):
            pass
        elif a == "-c":
            i += 1
        elif a.startswith("-") and len(a) > 1:
            fail("option %s is not faked" % a, 2)
        else:
            devs.append(a)
        i += 1
    if fmt not in ("full", "value", "device", "export"):
        fail("output format %s is not faked" % fmt, 2)

    def tags_of(r):
        t = []
        if r["TYPE"] == "disk":
            t += [("PTUUID", "00000000-0000-4000-8000-00000000f0f" + ("1" if r["NAME"] == "mmcblk1" else "0")), ("PTTYPE", "gpt")]
        else:
            if r["FSTYPE"]:
                t += [("LABEL", r["LABEL"]), ("UUID", r["UUID"]), ("BLOCK_SIZE", "4096"), ("TYPE", r["FSTYPE"])]
            t += [("PARTLABEL", r["PARTLABEL"]), ("PARTUUID", r["PARTUUID"])]
        return [(k, v) for k, v in t if not tags or k in tags]

    sel = []
    for d in devs:
        r = by_name(d)
        if r is None:
            sys.exit(2)
        sel.append(r)
    if not devs:
        sel = rows
    found = 0
    for r in sel:
        t = tags_of(r)
        if not t:
            continue
        found += 1
        dev = "/dev/" + r["NAME"]
        if fmt == "full":
            print("%s: %s" % (dev, " ".join('%s="%s"' % kv for kv in t)))
        elif fmt == "value":
            for _, v in t:
                print(v)
        elif fmt == "device":
            print(dev)
        else:
            print("DEVNAME=%s" % dev)
            for k, v in t:
                print("%s=%s" % (k, v))
            print()
    if not found:
        sys.exit(2)


def findmnt(args):
    nohead = raw = False
    cols = None
    target = lookup = None
    i = 0
    while i < len(args):
        a = args[i]
        if a == "--":
            if i + 1 < len(args):
                target = args[i + 1]
            break
        if a.startswith("--output="):
            cols = a.split("=", 1)[1]
        elif a in ("-o", "--output"):
            i += 1
            cols = args[i]
        elif a.startswith("-o") and len(a) > 2 and not a.startswith("--"):
            cols = a[2:]
        elif a in ("-T", "--target"):
            i += 1
            lookup = args[i]
        elif a in ("-M", "--mountpoint"):
            i += 1
            target = args[i]
        elif a == "--noheadings":
            nohead = True
        elif a == "--raw":
            raw = True
        elif a in ("--nofsroot", "--list", "--first-only", "--real", "--kernel"):
            pass
        elif a.startswith("-") and len(a) > 1 and not a.startswith("--"):
            for ch in a[1:]:
                if ch == "n":
                    nohead = True
                elif ch == "r":
                    raw = True
                elif ch in ("v", "l", "f", "k", "e"):
                    pass
                else:
                    fail("option -%s is not faked" % ch)
        elif a.startswith("--"):
            fail("option %s is not faked" % a)
        else:
            target = a
        i += 1
    mounts = [r for r in rows if r["MOUNTPOINT"]]

    def unmap(p):
        # the ssh shim mapped /boot (or /dev/mmcblk*) into DIR: a board path again
        if p is not None and p.startswith(root + "/"):
            return p[len(root):]
        return p

    lookup, target = unmap(lookup), unmap(target)
    if lookup is not None:
        best = None
        for r in mounts:
            m = r["MOUNTPOINT"]
            if lookup == m or m == "/" or lookup.startswith(m.rstrip("/") + "/"):
                if best is None or len(m) > len(best["MOUNTPOINT"]):
                    best = r
        sel = [best] if best else []
    elif target is not None:
        sel = [r for r in mounts if r["MOUNTPOINT"] == target or "/dev/" + r["NAME"] == target]
    else:
        sel = mounts
    if not sel:
        sys.exit(1)
    cols = (cols or "TARGET,SOURCE,FSTYPE,OPTIONS").upper().split(",")
    lines = []
    for r in sel:
        vals = []
        for c in cols:
            if c == "TARGET":
                vals.append(r["MOUNTPOINT"])
            elif c == "SOURCE":
                vals.append("/dev/" + r["NAME"])
            elif c == "FSTYPE":
                vals.append(r["FSTYPE"])
            elif c == "OPTIONS":
                vals.append("rw,relatime")
            elif c in ("LABEL", "PARTLABEL", "UUID", "PARTUUID"):
                vals.append(r[c])
            else:
                fail("unknown column: %s" % c)
        lines.append(vals)
    for ln in table(lines, cols, nohead, raw):
        print(ln)


{"lsblk": lsblk, "blkid": blkid, "findmnt": findmnt}[tool](args)
PYEOF
}

# ---- the environment shims ----------------------------------------------------------

fb_is_protected() {
    local p
    case $1 in bootcmd_* | mp2z_* | yako_*) return 0 ;; esac
    for p in $MP2S_FB_PROTECTED; do [ "$1" = "$p" ] && return 0; done
    return 1
}

fb_envfile() {
    if fb_have_envtool; then echo "$ROOT/dev/${MP2S_FB_SD}p7"; else echo "$ROOT/env/u-boot-env.txt"; fi
}

fb_printenv() {
    local value_only=0
    local -a names=()
    while [ $# -gt 0 ]; do
        case $1 in
        -n) value_only=1 ;;
        -c | -l) shift ;;
        -*) fb_die "fw_printenv: option $1 is not faked" ;;
        *) names+=("$1") ;;
        esac
        shift
    done
    if fb_have_envtool; then
        if [ $value_only = 1 ]; then fb_envtool print "$(fb_envfile)" -n "${names[@]+"${names[@]}"}"
        else fb_envtool print "$(fb_envfile)" "${names[@]+"${names[@]}"}"; fi
        return
    fi
    local f n line rc=0
    f=$(fb_envfile)
    if [ ${#names[@]} = 0 ]; then cat "$f"; return 0; fi
    [ $value_only = 1 ] && [ ${#names[@]} != 1 ] && fb_die "fw_printenv: -n takes exactly one name"
    for n in "${names[@]}"; do
        line=$(grep -m1 "^$n=" "$f") || { echo "## Error: \"$n\" not defined" >&2; rc=1; continue; }
        if [ $value_only = 1 ]; then printf '%s\n' "${line#*=}"; else printf '%s\n' "$line"; fi
    done
    return $rc
}

# fb_env_collapse ORIG EDITED OUT: EDITED is ORIG after one or two
# uboot_env.py edits; OUT is ORIG with only the originally inactive copy
# replaced by EDITED's newest copy, flags = active + 1: one write, as
# fw_setenv's
fb_env_collapse() {
    python3 -I - "$1" "$2" "$3" "$MP2S_FB_ENV_SIZE" "$MP2S_FB_ENV_OFF1" "$MP2S_FB_ENV_OFF2" <<'PYEOF'
import sys
orig_p, edit_p, out_p = sys.argv[1:4]
size, off = int(sys.argv[4], 0), (int(sys.argv[5], 0), int(sys.argv[6], 0))
orig = bytearray(open(orig_p, "rb").read())
edit = open(edit_p, "rb").read()


def active(buf):
    f1, f2 = buf[off[0] + 4], buf[off[1] + 4]
    if f1 == 0xff and f2 == 0:
        return 1
    if f2 == 0xff and f1 == 0:
        return 0
    return 1 if f2 > f1 else 0


a = active(orig)
i = 1 - a
e = active(edit)
new = bytearray(edit[off[e]:off[e] + size])
new[4] = (orig[off[a] + 4] + 1) & 0xff
orig[off[i]:off[i] + size] = new
open(out_p, "wb").write(orig)
PYEOF
}

# fw_setenv NAME [VALUE...] | fw_setenv -s FILE: every name is checked
# first, then one environment write
fb_setenv() {
    local script= line name val i f
    local -a names=() vals=()
    while [ $# -gt 0 ]; do
        case $1 in
        -c | -l) shift 2 ;;
        -s | --script)
            [ $# -ge 2 ] || fb_die "fw_setenv -s FILE"
            if [ "$2" = - ]; then script=$(cat); else script=$(cat -- "$2") || exit 1; fi
            shift 2
            while IFS= read -r line; do
                line=${line#"${line%%[![:space:]]*}"}
                case $line in '' | '#'*) continue ;; esac
                name=${line%%[[:space:]=]*}
                val=${line#"$name"}
                val=${val#=}
                val=${val#"${val%%[![:space:]]*}"}
                names+=("$name"); vals+=("$val")
            done <<< "$script"
            continue ;;
        -*) fb_die "fw_setenv: option $1 is not faked" ;;
        *) break ;;
        esac
    done
    if [ -z "$script" ]; then
        [ $# -ge 1 ] || fb_die "fw_setenv NAME [VALUE...] | -s FILE"
        name=$1; shift
        names=("$name"); vals=("$*")
    fi
    local bad=0
    for i in "${!names[@]}"; do
        name=${names[$i]}
        if [ -z "$name" ] || [[ $name == *[[:space:]=]* ]]; then
            echo "## Error: illegal variable name \"$name\"" >&2; exit 1
        fi
        if fb_is_protected "$name"; then
            fb_log "VIOLATION setenv $name ${vals[$i]}"
            echo "mp2s_fakeboard: VIOLATION: fw_setenv of $name refused (never written: docs/notes/stm32mp257f-dk-standalone.md section 3)" >&2
            bad=1
        fi
    done
    [ $bad = 0 ] || exit 1
    f=$(fb_envfile)
    if fb_have_envtool; then
        local -a sets=() unsets=()
        for i in "${!names[@]}"; do
            if [ -z "${vals[$i]}" ]; then
                fb_envtool print "$f" "${names[$i]}" > /dev/null 2>&1 && unsets+=("${names[$i]}")
            else
                sets+=("${names[$i]}=${vals[$i]}")
            fi
        done
        if [ ${#sets[@]} -gt 0 ] || [ ${#unsets[@]} -gt 0 ]; then
            local scratch
            scratch=$(mktemp "${TMPDIR:-/tmp}/mp2s_fb_env.XXXXXX") || exit 1
            cp "$f" "$scratch" || exit 1
            if [ ${#sets[@]} -gt 0 ]; then
                fb_envtool set "$scratch" "$scratch" "${sets[@]}" > /dev/null || { rm -f "$scratch"; exit 1; }
            fi
            if [ ${#unsets[@]} -gt 0 ]; then
                fb_envtool unset "$scratch" "$scratch" "${unsets[@]}" > /dev/null || { rm -f "$scratch"; exit 1; }
            fi
            fb_env_collapse "$f" "$scratch" "$scratch.out" || { rm -f "$scratch" "$scratch.out"; exit 1; }
            mv -f "$scratch.out" "$f" || exit 1
            rm -f "$scratch"
        fi
    else
        local tmp=$f.new
        cp "$f" "$tmp" || exit 1
        for i in "${!names[@]}"; do
            grep -v "^${names[$i]}=" "$tmp" > "$tmp.2"; mv -f "$tmp.2" "$tmp"
            [ -n "${vals[$i]}" ] && printf '%s=%s\n' "${names[$i]}" "${vals[$i]}" >> "$tmp"
        done
        mv -f "$tmp" "$f" || exit 1
    fi
    for i in "${!names[@]}"; do
        if [ -n "${vals[$i]}" ]; then fb_log "setenv ${names[$i]} ${vals[$i]}"; else fb_log "setenv ${names[$i]}"; fi
    done
}

# ---- the ssh shim -------------------------------------------------------------------

# fb_cmd_check ROOT CMD: print the command with the board paths mapped into
# ROOT and exit 0, or print the paths it would write outside ROOT/boot and
# exit 3; exit 2 when it does not parse. Also refuses mount, umount and a
# write verb fed by xargs (exit 3, "verb: ..." lines).
fb_cmd_check() {
    python3 -I - "$1" "$2" <<'PYEOF'
import os, re, shlex, sys

root, cmd = sys.argv[1], sys.argv[2]
B = r"(?<![\w/.-])"
E = r"(?![\w.-])"
MAPS = [(B + "/boot" + E, root + "/boot"),
        (B + "/sys/class/block" + E, root + "/sys/class/block"),
        (B + "/sys/block" + E, root + "/sys/block"),
        (B + r"/dev/mmcblk(?=\d)", root + "/dev/mmcblk"),
        (B + "/proc/sys/vm/drop_caches" + E, root + "/proc/sys/vm/drop_caches"),
        (B + "/proc/sys/kernel/random/boot_id" + E, root + "/proc/sys/kernel/random/boot_id"),
        (B + "/proc/self/mountinfo" + E, root + "/proc/self/mountinfo"),
        (B + "/proc/self/mounts" + E, root + "/proc/self/mounts"),
        (B + "/proc/mounts" + E, root + "/proc/mounts"),
        (B + "/proc/uptime" + E, root + "/proc/uptime"),
        (B + "/etc/fw_env.config" + E, root + "/etc/fw_env.config"),
        (B + "/etc/os-release" + E, root + "/etc/os-release"),
        (r"(?<=--root)(\s+)/(?=\s|$|['\"])", r"\g<1>" + root)]
for rx, rep in MAPS:
    cmd = re.sub(rx, rep, cmd)

SEPS = set(";|&()\n")
REDIR = {">", ">>", ">|", "&>", "&>>", "<>"}
SINKS = {"/dev/null", "/dev/stdout", "/dev/stderr", root + "/proc/sys/vm/drop_caches"}
# verb: (option letters that take an argument, how to pick the targets)
#   all     every non-option argument
#   last    the last non-option argument (cp, install: the destination)
#   skip1   every non-option argument but the first (chmod MODE FILE...)
#   none    no path argument is written (dd: only of=)
VERBS = {"rm": ("", "all"), "rmdir": ("", "all"), "unlink": ("", "all"), "mv": ("tS", "all"),
         "mkdir": ("m", "all"), "touch": ("drt", "all"), "ln": ("S", "all"), "truncate": ("sr", "all"),
         "fallocate": ("lon", "all"), "shred": ("ns", "all"), "tee": ("", "all"), "chmod": ("", "skip1"),
         "chown": ("", "skip1"), "chgrp": ("", "skip1"), "cp": ("tS", "last"), "install": ("mogtS", "last"),
         "dd": ("", "none"), "sed": ("ef", "sed"), "sort": ("otkS", "sort"), "mkfs": ("", "all"),
         "mke2fs": ("", "all"), "wipefs": ("", "all"), "sgdisk": ("", "all"), "sfdisk": ("", "all"),
         "parted": ("", "all"), "e2label": ("", "all"), "tune2fs": ("", "all"), "debugfs": ("fR", "all"),
         "rsync": ("e", "last"), "tar": ("Cf", "tar"), "patch": ("", "all"), "mount": ("", "refuse"),
         "umount": ("", "refuse"), "chroot": ("", "refuse")}
# prefix: the option letters that take an argument
PREFIX = {"sudo": "ugCpUhD", "env": "uCS", "nice": "n", "ionice": "cn", "nohup": "", "command": "",
          "exec": "", "time": "", "timeout": "ks", "doas": "u", "stdbuf": "ioe"}
SHELLS = {"sh", "bash", "dash", "ash", "zsh", "ksh", "busybox"}
targets, verbs = [], []


def tokens(text):
    lx = shlex.shlex(text, posix=True, punctuation_chars=";|&()<>\n")
    lx.whitespace = " \t\r"
    lx.whitespace_split = True
    lx.commenters = ""
    return list(lx)


def split_cmds(toks):
    """the simple commands: lists of tokens between separators"""
    out, cur = [], []
    for t in toks:
        if t and set(t) <= SEPS:
            if cur:
                out.append(cur)
            cur = []
        else:
            cur.append(t)
    if cur:
        out.append(cur)
    return out


def strip_prefixes(toks):
    """drop sudo/env/timeout ... and their options; return the command"""
    i = 0
    while i < len(toks):
        t = toks[i]
        base = os.path.basename(t)
        if base in PREFIX:
            i += 1
            argopts = PREFIX[base]
            if base == "timeout":
                while i < len(toks) and toks[i].startswith("-"):
                    if toks[i][1:2] in argopts and len(toks[i]) == 2:
                        i += 2
                    else:
                        i += 1
                i += 1  # the duration
                continue
            while i < len(toks) and toks[i].startswith("-"):
                if toks[i] == "--":
                    i += 1
                    break
                if len(toks[i]) == 2 and toks[i][1] in argopts:
                    i += 2
                else:
                    i += 1
            continue
        if "=" in t and not t.startswith("=") and re.match(r"^[A-Za-z_]\w*=", t):
            i += 1  # NAME=VALUE before the command
            continue
        break
    return toks[i:]


def scan(toks):
    """collect the write targets of one simple command, redirects first"""
    rest, i = [], 0
    while i < len(toks):
        t = toks[i]
        nxt = toks[i + 1] if i + 1 < len(toks) else ""
        if set(t) <= set("<>|&") and ">" in t:
            # >, >>, >|, &>, &>>, <>: the next token is written; >&N is a dup
            if t.endswith("&") and (nxt.isdigit() or nxt == "-"):
                i += 2
                continue
            targets.append(nxt)
            i += 2
            continue
        if set(t) <= set("<&-") and "<" in t:
            i += 2  # <, <<, <<<, <&: read
            continue
        if t.isdigit() and nxt and nxt[:1] in "<>&":
            i += 1  # the fd of 2>FILE
            continue
        rest.append(t)
        i += 1
    toks = strip_prefixes(rest)
    if not toks:
        return
    verb = os.path.basename(toks[0])
    args = toks[1:]
    if verb in SHELLS:
        if verb == "busybox" and args:
            verb, args = args[0], args[1:]
        for j, a in enumerate(args):
            if a == "-c" and j + 1 < len(args):
                check(args[j + 1])
                return
        return
    if verb == "xargs":
        j = 0
        while j < len(args) and args[j].startswith("-"):
            if args[j] in ("-n", "-I", "-L", "-P", "-s", "-d", "-E", "-a") and j + 1 < len(args):
                j += 2
            else:
                j += 1
        inner = os.path.basename(args[j]) if j < len(args) else ""
        if inner in VERBS or inner in SHELLS:
            verbs.append("xargs %s: a write verb fed by xargs" % inner)
        return
    if verb == "find":
        if any(a in ("-delete", "-exec", "-execdir", "-ok", "-okdir") for a in args):
            paths = []
            for a in args:
                if a.startswith("-") and a not in ("-H", "-L", "-P"):
                    break
                if a not in ("-H", "-L", "-P"):
                    paths.append(a)
            targets.extend(paths or ["."])
            if "-exec" in args or "-execdir" in args or "-ok" in args or "-okdir" in args:
                k = max(args.index(a) for a in args if a in ("-exec", "-execdir", "-ok", "-okdir"))
                sub = []
                for a in args[k + 1:]:
                    if a in (";", "+"):
                        break
                    sub.append(a)
                for p in paths or ["."]:
                    scan([p if a == "{}" else a for a in sub])
        return
    if verb not in VERBS:
        return
    argopts, mode = VERBS[verb]
    if mode == "refuse":
        verbs.append("%s: refused on the fake board" % verb)
        return
    # option arguments that are themselves written: -t DIR (mv cp install),
    # -o FILE (sort), -C DIR (tar)
    dir_opt = {"mv": "t", "cp": "t", "install": "t", "sort": "o", "tar": "C"}.get(verb, "")
    pos, j, dash, saw_dir = [], 0, False, False
    while j < len(args):
        a = args[j]
        if not dash and a == "--":
            dash = True
        elif not dash and a.startswith("--"):
            if a.startswith("--target-directory=") or a.startswith("--output=") or a.startswith("--directory="):
                targets.append(a.split("=", 1)[1])
                saw_dir = True
            elif a in ("--target-directory", "--output", "--directory") and j + 1 < len(args):
                j += 1
                targets.append(args[j])
                saw_dir = True
        elif not dash and a.startswith("-") and len(a) > 1:
            if len(a) == 2 and a[1] in argopts:
                if a[1] in dir_opt and j + 1 < len(args):
                    targets.append(args[j + 1])
                    saw_dir = True
                j += 1
            elif dir_opt and a[1] in dir_opt and len(a) > 2:
                targets.append(a[2:])
                saw_dir = True
        else:
            pos.append(a)
        j += 1
    if verb == "dd":
        targets.extend(a[3:] for a in args if a.startswith("of="))
    elif verb == "sed":
        if any(a in ("-i", "--in-place") or a.startswith("-i") or a.startswith("--in-place=") for a in args):
            has_e = any(a in ("-e", "-f", "--expression", "--file") or a.startswith("--expression=") for a in args)
            targets.extend(pos if has_e else pos[1:])
    elif verb == "sort":
        pass
    elif verb == "tar":
        first = args[0] if args else ""
        extract = "--extract" in args or "-x" in args or (first and not first.startswith("--") and "x" in first.lstrip("-"))
        if extract and not saw_dir:
            targets.append(".")
    elif mode == "all":
        targets.extend(pos)
    elif mode == "last":
        if pos and not saw_dir:
            targets.append(pos[-1])
    elif mode == "skip1":
        targets.extend(pos[1:])


def check(text):
    try:
        toks = tokens(text)
    except ValueError:
        sys.exit(2)
    for c in split_cmds(toks):
        scan(c)


check(cmd)
boot = os.path.join(root, "boot")
bad = []
for t in targets:
    if t in SINKS:
        continue
    p = os.path.normpath(os.path.join(root, t))
    if p.startswith(boot + os.sep):
        continue
    shown = t
    if shown.startswith(root + os.sep):
        shown = shown[len(root):]
    elif not shown.startswith("/"):
        shown = "/" + shown
    bad.append(shown)
if bad or verbs:
    for v in verbs:
        print("verb: " + v)
    for t in bad:
        print(t)
    sys.exit(3)
sys.stdout.write(cmd)
PYEOF
}

# the sha256 of every file in ROOT but the log and the manifest itself
fb_manifest() {
    (cd "$ROOT" && find . -type f ! -path ./writes.log ! -path ./.manifest -print0 | LC_ALL=C sort -z |
        xargs -0 -r sha256sum | sed 's|  \./|  |')
}

fb_ssh() {
    # ssh [-46AaCfGgKkMNnqsTtVvXxYy] [-b -c -D -E -e -F -I -i -J -L -l -m -O -o -p -Q -R -S -W -w -B -P ARG] HOST CMD...
    while [ $# -gt 0 ]; do
        case $1 in
        --) shift; break ;;
        -[bcDEeFIiJLlmOopQRSWwBP]) shift 2 ;;
        -[bcDEeFIiJLlmOopQRSWwBP]*) shift ;;
        -*) shift ;;
        *) break ;;
        esac
    done
    [ $# -ge 1 ] || fb_die "ssh: no host"
    shift
    [ $# -ge 1 ] || fb_die "ssh: an interactive session is not faked (give a command)"
    local cmd="$*" mapped rc=0 out
    out=$(fb_cmd_check "$ROOT" "$cmd") || rc=$?
    if [ $rc = 2 ]; then
        fb_log "VIOLATION ssh command does not parse (not run)"
        echo "mp2s_fakeboard: VIOLATION: the ssh command does not parse; not run" >&2
        exit 1
    elif [ $rc != 0 ]; then
        local t
        while IFS= read -r t; do
            [ -n "$t" ] || continue
            case $t in
            verb:*) fb_log "VIOLATION ssh ${t#verb: } (not run)"; echo "mp2s_fakeboard: VIOLATION: ${t#verb: }; not run" >&2 ;;
            *) fb_log "VIOLATION file $t outside /boot (not run)"; echo "mp2s_fakeboard: VIOLATION: the ssh command writes $t, outside /boot; not run" >&2 ;;
            esac
        done <<< "$out"
        exit 1
    fi
    mapped=$out
    local before_lines viol=0 h f after
    before_lines=$(wc -l < "$ROOT/writes.log")
    fb_manifest > "$ROOT/.manifest"
    (cd "$ROOT" && PATH="$ROOT/bin:$PATH" MP2S_FAKE_ROOT=$ROOT bash -c "$mapped") || rc=$?
    after=$(fb_manifest)
    local envfile=dev/${MP2S_FB_SD}p7
    while read -r h f; do # new or changed files
        [ -n "$f" ] || continue
        grep -qxF "$h  $f" "$ROOT/.manifest" && continue
        case $f in
        boot/*) fb_log "file /$f $h" ;;
        "$envfile")
            if [ "$(tail -n +"$((before_lines + 1))" "$ROOT/writes.log" | grep -c '^setenv ')" = 0 ]; then
                fb_log "VIOLATION env u-boot-env written outside fw_setenv"; viol=1
            fi ;;
        *) fb_log "VIOLATION file /$f outside /boot"; viol=1 ;;
        esac
    done <<< "$after"
    while read -r h f; do # removed files
        [ -n "$f" ] || continue
        grep -qF "  $f" <<< "$after" && continue
        case $f in
        boot/*) fb_log "file /$f removed" ;;
        *) fb_log "VIOLATION file /$f removed outside /boot"; viol=1 ;;
        esac
    done < "$ROOT/.manifest"
    rm -f "$ROOT/.manifest"
    if [ $viol != 0 ]; then
        echo "mp2s_fakeboard: VIOLATION: the ssh command wrote outside /boot (see writes.log)" >&2
        exit 1
    fi
    exit $rc
}

fb_shim() {
    local what=$1
    ROOT=$2
    shift 2
    case $what in
    fw_printenv) fb_printenv "$@" ;;
    fw_setenv) fb_setenv "$@" ;;
    findmnt | lsblk | blkid) fb_storage "$what" "$@" ;;
    sync) exit 0 ;;
    uname)
        case ${1:-} in
        -r) echo 6.6.0-fake ;;
        -m) echo aarch64 ;;
        -n) echo stm32mp257f-dk-fake ;;
        -a) echo "Linux stm32mp257f-dk-fake 6.6.0-fake #1 SMP PREEMPT aarch64 GNU/Linux" ;;
        '' | -s) echo Linux ;;
        *) fb_die "uname $1 is not faked" ;;
        esac ;;
    reboot) fb_log reboot ;;
    systemctl)
        case ${1:-} in
        reboot | poweroff | halt) fb_log "${1/halt/poweroff}" ;;
        is-system-running) echo running ;;
        *) fb_die "systemctl $* is not faked" ;;
        esac ;;
    sudo)
        while [ $# -gt 0 ]; do
            case $1 in
            --) shift; break ;;
            -u | -g | -C | -p | -h) shift 2 ;;
            -*) shift ;;
            *) break ;;
            esac
        done
        [ $# -ge 1 ] || fb_die "sudo: a shell is not faked (give a command)"
        PATH="$ROOT/bin:$PATH" exec "$@" ;;
    ssh) fb_ssh "$@" ;;
    *) fb_die "no shim $what" ;;
    esac
}

fb_usage() {
    sed -n '10,14p' "$MP2S_FB_SELF" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

fb_main() {
    case ${1:-} in
    init) shift; mp2s_fakeboard_init "$@" ;;
    shell) shift; mp2s_fakeboard_shell "$@" ;;
    writes) shift; mp2s_fakeboard_writes "$@" ;;
    _shim) shift; fb_shim "$@" ;;
    *) fb_usage ;;
    esac
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
    fb_main "$@"
fi

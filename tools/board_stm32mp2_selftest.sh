#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# board_stm32mp2_selftest.sh - tools/board-stm32mp2.sh without a board
# (tools/check.sh runs it: "tools   board-stm32mp2.sh dry-run matrix, env edits and replays").
#
#   tools/board_stm32mp2_selftest.sh
#
# 1. Every verb under --dry-run against a fake image, with ssh, scp, stty,
#    flock, sudo and fuser replaced by stubs that only record that they were
#    called: each verb exits 0, prints its board commands as "DRY ..." lines
#    (the one-shot's env write and the reboot among them), and no stub may
#    have run; "install" prints the UNDO before any write; nothing is written
#    on the host; no host name, address or console path appears in the output.
# 2. The refusals that need no board: an image without the watchdog marker,
#    one too big for the 256 MiB window, no AArch64 header, a bad run name,
#    a run without BOARD_UART (and the blind run MP2_NO_CONSOLE=1 allows),
#    an unknown verb.
# 3. The environment edits on a synthetic redundant image (built here from
#    the format, as tools/testdata/uboot-env/test_uboot_env.py does: no byte
#    from a board): hook wraps bootcmd once and keeps the saved one, is
#    idempotent, upgrades another version's strings, refuses a foreign hook;
#    unhook gives the variables of the input back; arm and disarm; a bad CRC
#    and a single-copy layout; the hook strings themselves are pinned.
# 4. Replays of synthetic consoles through the verdict (tally): the
#    mp2_identify PASS run is done (rc 0), a FAIL line failed (rc 4), no end
#    line no_end_line with the boot_verdict naming how far it got, PSCI
#    returning psci_returned, a board that did not come back not_back (rc 5),
#    a blind run returned, a one-shot left over rc 6.
# 5. Redaction of a run directory: host names, addresses and the console's
#    path or bridge become placeholders in the copies.
# Prints "board_stm32mp2_selftest: N checks, F failed"; exit 0 only when F is
# 0 and N is the pinned count.
set -uo pipefail
export LC_ALL=C
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
B="$HERE/board-stm32mp2.sh"
WANT_CHECKS=68

S=$(mktemp -d)
trap 'rm -rf "$S"' EXIT
n=0
fails=0
check() { # check DESCRIPTION COMMAND...
    local what=$1
    shift
    n=$((n + 1))
    if "$@"; then echo "  PASS  $what"; else echo "  FAIL  $what"; fails=$((fails + 1)); fi
}
same() { # same DESCRIPTION FILE-A FILE-B
    local what=$1
    n=$((n + 1))
    if [ -s "$2" ] && cmp -s "$2" "$3"; then
        echo "  PASS  $what"
    else
        echo "  FAIL  $what"
        cmp "$2" "$3" 2>&1 | head -3 | sed 's/^/        /'
        fails=$((fails + 1))
    fi
}
unset BOARD_HOST BOARD_UART MP2_NO_CONSOLE MP2_NO_WATCHDOG MP2_SUDO MP2_ENV_OFFSET MP2_ENV_OFFSET2 MP2_ENV_SIZE \
    MP2_BACKUP_DIR RUN_BOUND RETURN_BOUND BOARD_END_RE LOCK BOARD_LOCK LOCK_WAIT HOLD_MAX

echo "board_stm32mp2_selftest: build $(git -C "$REPO" describe --always --dirty 2>/dev/null || echo unknown)"

# ---- fake images: the AArch64 Image header, image_size, a marker ------------
mkimg() { # mkimg FILE FOOTPRINT MARKER [NOHEADER]
    local le
    le=$(printf '%016x' "$2" | sed -E 's/(..)(..)(..)(..)(..)(..)(..)(..)/\\x\8\\x\7\\x\6\\x\5\\x\4\\x\3\\x\2\\x\1/')
    {
        head -c 16 /dev/zero
        printf '%b' "$le"
        head -c 32 /dev/zero
        if [ -n "${4:-}" ]; then printf 'XXXX'; else printf 'ARMd'; fi
        head -c 1000 /dev/zero
        printf '%s\0*** Booting Zephyr OS build selftest ***\0' "$3"
    } > "$1"
}
mkdir -p "$S/img"
mkimg "$S/img/mp2.bin" 4194304 'gc_wdog_arm: armed OP-TEE arm,smc-wdt fid 0xb200005a, timeout 32 s'
mkimg "$S/img/nomarker.bin" 4194304 'mp2: no watchdog armed by this image'
mkimg "$S/img/huge.bin" $((300 * 1024 * 1024)) 'gc_wdog_arm: armed'
mkimg "$S/img/noheader.bin" 4194304 'gc_wdog_arm: armed' 1

# ---- stubs: any of these running is a failure of --dry-run --------------------
mkdir -p "$S/stub"
for c in ssh scp stty flock sudo fuser; do
    printf '#!/bin/sh\necho "%s $*" >> "%s/touched"\nexit 99\n' "$c" "$S" > "$S/stub/$c"
    chmod +x "$S/stub/$c"
done
dry() { PATH="$S/stub:$PATH" "$B" --dry-run "$@"; }
untouched() { [ ! -s "$S/touched" ]; }
# the names a dry run must never print (a fake host and console; the runner
# scrubs them, and the dry run itself names only placeholders). The fixtures
# are shapes the release redaction scan does not count (no login@host, no
# serial-bridge port, documentation addresses only).
FAKE_HOST=mp2board.example.lab
FAKE_UART=bridgehost.example.lab:40001
leaks() { grep -qE 'mp2board|bridgehost|40001|example\.lab' <<<"$1"; }

echo "== 1. every verb under --dry-run (stubs record any board access)"
for verb in status install uninstall disarm; do
    out=$(BOARD_HOST=$FAKE_HOST dry "$verb" 2>&1)
    rc=$?
    check "$verb: rc 0, DRY board lines, no host name in the output" \
        eval '[ "$rc" = 0 ] && grep -q "^DRY board" <<<"$out" && ! leaks "$out"'
done
out=$(BOARD_HOST=$FAKE_HOST dry install 2>&1)
check "install: the UNDO (the dd of the pristine image back) is printed before the env write" \
    eval 'u=$(grep -n "UNDO, should anything" <<<"$out" | cut -d: -f1); w=$(grep -n "dd of=/dev/mmcblkDp7" <<<"$out" | head -1 | cut -d: -f1);
          [ -n "$u" ] && [ -n "$w" ] && [ "$u" -lt "$w" ]'
check "install: the pristine image is kept on the host and on the card, the stock boot files hashed" \
    eval 'grep -q "u-boot-env.bin" <<<"$out" && grep -q "boot-sha256.txt" <<<"$out" && grep -q "cp <env.bin> .*/records/stm32mp257f-dk/uboot-env/" <<<"$out"'
check "install: the env write covers only the two copies (seek 0x7c000/512 = 992, 16 KiB), read back by sha256 and fw_printenv" \
    eval 'grep -q "head -c 16384 | ssh BOARD .*dd of=/dev/mmcblkDp7 bs=512 seek=992 conv=fsync" <<<"$out" &&
          grep -q "sha256sum   # must equal the host image" <<<"$out" && grep -q "fw_printenv yako_addr yako_part   # must show the edit (install)" <<<"$out"'
check "install: the hook's variables: yako_addr 0xa0000000 and yako_part from /boot's partition (8)" \
    eval 'grep -q "yako_addr=0xa0000000 yako_part=8" <<<"$out"'
check "install: the lock is taken, the sessions read (who), the mount of /boot and the card's labels read" \
    eval 'grep -q "board_lock.sh" <<<"$out" && grep -q "^DRY board\$ who" <<<"$out" && grep -q "findmnt -n -v -o SOURCE,FSTYPE -T /boot" <<<"$out" && grep -q "lsblk -rno PATH,PARTLABEL" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST MP2_SUDO="sudo -n " dry install 2>&1)
check "install with MP2_SUDO: the board commands carry the prefix" eval 'grep -q "sudo -n dd if=/dev/mmcblkDp7" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST dry uninstall 2>&1)
check "uninstall: bootcmd restored from yako_bootcmd_prev, the variables unset, the staging directory removed" \
    eval 'grep -q "bootcmd=<yako_bootcmd_prev>; unset yako_hook yako_mmc yako_addr yako_part yako_bootcmd_prev yako_once yako_file" <<<"$out" &&
          grep -q "rm -rf /boot/yakogl" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST dry disarm 2>&1)
check "disarm: yako_once and yako_file unset, the staged once-*.bin removed" \
    eval 'grep -q "unset <env.bin> <env.new> yako_once yako_file" <<<"$out" && grep -q "rm -f /boot/yakogl/once-\*.bin" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST dry status 2>&1)
check "status: fw_printenv of bootcmd and the yako_* variables, the install state, the stock boot files checked" \
    eval 'grep -q "fw_printenv bootcmd bootdelay boot_targets yako_hook" <<<"$out" && grep -q "^install: ok" <<<"$out" && grep -q "sha256sum -c /boot/yakogl/backup/boot-sha256.txt" <<<"$out"'

out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART dry probe "$S/logs" 2>&1)
rc=$?
check "probe (bridge): rc 0, the bridge probed as <BOARD_UART>, the lock, the pre-checks, no reboot, no leak" \
    eval '[ "$rc" = 0 ] && grep -q "am62p_netboot_uart.py probe <BOARD_UART>" <<<"$out" && grep -q "board_lock.sh" <<<"$out" &&
          grep -q "probe: the pre-checks passed" <<<"$out" && ! grep -q "systemctl reboot" <<<"$out" && ! leaks "$out"'
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART dry run "$S/img/mp2.bin" "$S/logs" trial 2>&1)
rc=$?
check "run (bridge): rc 0, the image checked (sha256, footprint), the boot command 'sync; systemctl reboot'" \
    eval '[ "$rc" = 0 ] && grep -q "image sha256 $(sha256sum "$S/img/mp2.bin" | cut -c1-16) footprint 4194304" <<<"$out" && grep -q "^DRY board\$ sync; systemctl reboot" <<<"$out"'
check "run: the order is stage, read back, capture, arm, reboot, wait, one-shot gone, staged image removed" \
    eval 'o=$(grep -nE "cat > /boot/yakogl/once-trial-|drop_caches|am62p_netboot_uart.py capture|set <env.bin> <env.new> yako_once=trial-|dd of=/dev/mmcblkDp7|systemctl reboot|wait until ssh answers from a new boot|fw_printenv -n yako_once   # must print nothing|rm -f /boot/yakogl/once-trial-" <<<"$out" | cut -d: -f1 | paste -sd" " -);
          [ "$(wc -w <<<"$o")" = 9 ] && [ "$(tr " " "\n" <<<"$o" | sort -n | paste -sd" " -)" = "$o" ]'
check "run: the hook must be installed, no one-shot pending, no leftover image, the free space of /boot read" \
    eval 'grep -q "fw_printenv bootcmd yako_hook" <<<"$out" && grep -q "fw_printenv -n yako_once" <<<"$out" && grep -q "ls /boot/yakogl/once-\*.bin" <<<"$out" && grep -q "df -B1 --output=avail /boot" <<<"$out"'
check "run: the one-shot names the staged file as U-Boot sees it (/yakogl/once-<token>.bin) and waits <= 300 s" \
    eval 'grep -q "yako_file=/yakogl/once-trial-" <<<"$out" && grep -q "<= 300 s" <<<"$out"'
check "run: no host, address or console path in the output; nothing written on the host; no stub ran" \
    eval '! leaks "$out" && [ ! -e "$S/logs" ] && untouched'
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART RUN_BOUND=200 RETURN_BOUND=100 dry run "$S/img/mp2.bin" "$S/logs" 2>&1)
check "run: RUN_BOUND and RETURN_BOUND set the wait; the name defaults to the image's" \
    eval 'grep -q "<= 300 s" <<<"$out" && grep -q "mp2: build" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=/dev/ttyFAKE0 dry run "$S/img/mp2.bin" "$S/logs" 2>&1)
rc=$?
check "run (device): rc 0, stty on <BOARD_UART>, the line reader, no bridge command, no device path printed" \
    eval '[ "$rc" = 0 ] && grep -q "stty -F <BOARD_UART> 115200" <<<"$out" && grep -q "read <BOARD_UART> line by line" <<<"$out" &&
          ! grep -q "am62p_netboot_uart.py" <<<"$out" && ! grep -q "ttyFAKE0" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST MP2_NO_CONSOLE=1 dry run "$S/img/mp2.bin" "$S/logs" 2>&1)
rc=$?
check "run (blind, MP2_NO_CONSOLE=1): rc 0, no probe, no capture" \
    eval '[ "$rc" = 0 ] && ! grep -q "am62p_netboot_uart.py" <<<"$out" && ! grep -q "stty" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART MP2_NO_WATCHDOG=1 dry run "$S/img/nomarker.bin" "$S/logs" 2>&1)
rc=$?
check "run of an image without the marker, MP2_NO_WATCHDOG=1: rc 0 with the WARNING" eval '[ "$rc" = 0 ] && grep -q "WARNING: MP2_NO_WATCHDOG=1" <<<"$out"'
check "dry runs: nothing written on the host, no stub ran" eval '[ ! -e "$S/logs" ] && untouched'

echo "== 2. refusals that need no board"
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART dry run "$S/img/nomarker.bin" "$S/logs" 2>&1)
rc=$?
check "an image without the watchdog marker: rc 3, REFUSED, names the marker and MP2_NO_WATCHDOG" \
    eval '[ "$rc" = 3 ] && grep -q "REFUSED" <<<"$out" && grep -q "gc_wdog_arm: armed" <<<"$out" && grep -q MP2_NO_WATCHDOG <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART dry run "$S/img/huge.bin" "$S/logs" 2>&1)
rc=$?
check "an image of 300 MiB: rc 3, does not fit 0xa0000000..0xb0000000" eval '[ "$rc" = 3 ] && grep -q "does not fit 0xa0000000..0xb0000000" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART dry run "$S/img/noheader.bin" "$S/logs" 2>&1)
rc=$?
check "an image without the AArch64 header: rc 3" eval '[ "$rc" = 3 ] && grep -q "no AArch64 Image header" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART dry run "$S/img/mp2.bin" "$S/logs" 'bad name' 2>&1)
rc=$?
check "a run name with a space: rc 2" eval '[ "$rc" = 2 ] && grep -q "letters, digits" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST "$B" run "$S/img/mp2.bin" "$S/logs" 2>&1)
rc=$?
check "a real run without BOARD_UART: rc 2 before the board is touched, names MP2_NO_CONSOLE" \
    eval '[ "$rc" = 2 ] && grep -q "set BOARD_UART" <<<"$out" && grep -q MP2_NO_CONSOLE <<<"$out" && untouched'
out=$(BOARD_HOST=$FAKE_HOST dry nosuchverb 2>&1)
rc=$?
check "an unknown verb: rc 2, the usage" eval '[ "$rc" = 2 ] && grep -q "tools/board-stm32mp2.sh \[--dry-run\] run" <<<"$out"'
out=$("$B" 2>&1)
check "no arguments: rc 2" [ $? = 2 ]
check "the refusals touched no stub and wrote nothing" eval '[ ! -e "$S/logs" ] && untouched'

echo "== 3. the environment edits on a synthetic image"
# A 512 KiB u-boot-env partition image with U-Boot's env/mmc.c layout: copy 1
# at 0x7e000 (the top), copy 2 at 0x7c000, 8 KiB each, a flag byte; the rest
# 0xff. Copy 1 is the active one (higher flags). No byte is from a board.
python3 -I - "$S/env.bin" "$S/env-single.bin" "$S/env-badcrc.bin" "$S/env-foreign.bin" <<'EOF'
import sys, zlib
def copy(entries, size, pad=0xff, flags=None):
    hdr = 4 if flags is None else 5
    data = b"".join(e + b"\0" for e in entries) + b"\0"
    area = data + bytes([pad]) * (size - hdr - len(data))
    return zlib.crc32(area).to_bytes(4, "little") + (b"" if flags is None else bytes([flags])) + area
V = [b"bootdelay=1", b"bootcmd=run bootcmd_stm32mp", b"boot_targets=mmc0 mmc1", b"fdtfile=st/stm32mp257f-dk.dtb"]
img = bytearray(b"\xff" * 0x80000)
img[0x7c000:0x7e000] = copy(V[:3], 0x2000, flags=1)
img[0x7e000:0x80000] = copy(V, 0x2000, flags=2)
open(sys.argv[1], "wb").write(img)
single = bytearray(b"\xff" * 0x80000)
single[0x7e000:0x80000] = copy(V, 0x2000)
open(sys.argv[2], "wb").write(single)
bad = bytearray(img)
bad[0x7e010] ^= 0x55
open(sys.argv[3], "wb").write(bad)
foreign = bytearray(b"\xff" * 0x80000)
foreign[0x7c000:0x7e000] = copy(V[:3], 0x2000, flags=1)
foreign[0x7e000:0x80000] = copy([b"bootdelay=1", b"bootcmd=run yako_hook; run bootcmd_stm32mp", b"yako_hook=echo other"], 0x2000, flags=2)
open(sys.argv[4], "wb").write(foreign)
EOF
ENVT() { python3 -I "$HERE/uboot_env.py" --redundant --size 0x2000 --offset 0x7e000 --offset2 0x7c000 "$@"; }
# vals IMAGE NAME...: the values, one per line ("-" when a name is not set)
vals() { local img=$1 v; shift; for v in "$@"; do ENVT print "$img" -n "$v" 2>/dev/null || echo -; done; }
out=$("$B" env-edit hook "$S/env.bin" "$S/hooked.bin" 8 2>&1)
rc=$?
check "hook: rc 0, 'installed', the output checks (both CRCs), copy 2 written and active" \
    eval '[ "$rc" = 0 ] && [ "$out" = installed ] && ENVT check "$S/hooked.bin" | grep -q "^active: copy 2"'
check "hook: bootcmd is 'run yako_hook; <bootcmd>', the saved bootcmd in yako_bootcmd_prev, bootdelay and the rest untouched" \
    eval '[ "$(ENVT print "$S/hooked.bin" -n bootcmd)" = "run yako_hook; run bootcmd_stm32mp" ] &&
          [ "$(ENVT print "$S/hooked.bin" -n yako_bootcmd_prev)" = "run bootcmd_stm32mp" ] &&
          [ "$(vals "$S/hooked.bin" bootdelay boot_targets fdtfile | paste -sd"|" -)" = "1|mmc0 mmc1|st/stm32mp257f-dk.dtb" ]'
check "hook: yako_hook clears and saves the one-shot BEFORE the load, then runs yako_mmc; a failed saveenv boots Linux" \
    eval 'h=$(ENVT print "$S/hooked.bin" -n yako_hook); [ "$h" = "if test -n \"\${yako_once}\"; then echo yakogl-mp2: once=\${yako_once}; setenv yako_once; if saveenv; then echo yakogl-mp2: cleared; run yako_mmc; else echo yakogl-mp2: saveenv failed, booting Linux; fi; fi" ]'
check "hook: yako_mmc loads from mmc \${boot_instance}:\${yako_part} (no device number hard-coded), caches off, go; a failed load boots Linux" \
    eval 'm=$(ENVT print "$S/hooked.bin" -n yako_mmc); [ "$m" = "if ext4load mmc \${boot_instance}:\${yako_part} \${yako_addr} \${yako_file}; then echo yakogl-mp2: go \${yako_addr}; dcache off; icache off; go \${yako_addr}; fi; echo yakogl-mp2: mmc load failed, booting Linux" ]'
check "hook: yako_addr 0xa0000000 (the link address), yako_part 8, no yako_once or yako_file" \
    eval '[ "$(vals "$S/hooked.bin" yako_addr yako_part yako_once yako_file | paste -sd"|" -)" = "0xa0000000|8|-|-" ]'
check "hook: the bytes outside the two copies are untouched" \
    eval 'cmp -s <(head -c $((0x7c000)) "$S/env.bin") <(head -c $((0x7c000)) "$S/hooked.bin")'
out=$("$B" env-edit hook "$S/hooked.bin" "$S/hooked2.bin" 8 2>&1)
check "hook twice: 'unchanged', the image byte for byte" eval '[ "$out" = unchanged ] && cmp -s "$S/hooked.bin" "$S/hooked2.bin"'
out=$("$B" env-edit hook "$S/hooked.bin" "$S/hooked9.bin" 9 2>&1)
check "hook with another partition number: 'upgraded', bootcmd still wrapped once, yako_part 9" \
    eval '[ "$out" = upgraded ] && [ "$(ENVT print "$S/hooked9.bin" -n bootcmd)" = "run yako_hook; run bootcmd_stm32mp" ] && [ "$(ENVT print "$S/hooked9.bin" -n yako_part)" = 9 ]'
out=$("$B" env-edit unhook "$S/hooked.bin" "$S/unhooked.bin" 2>&1)
rc=$?
check "unhook: rc 0, 'unhooked', the variables of the input back (bootcmd restored, no yako_*), both CRCs check" \
    eval '[ "$rc" = 0 ] && [ "$out" = unhooked ] && [ -z "$(ENVT diff "$S/env.bin" "$S/unhooked.bin")" ] && ENVT check "$S/unhooked.bin" >/dev/null'
out=$("$B" env-edit unhook "$S/env.bin" "$S/unhooked0.bin" 2>&1)
check "unhook of an image without the hook: 'not hooked', the variables unchanged" \
    eval '[ "$out" = "not hooked" ] && [ -z "$(ENVT diff "$S/env.bin" "$S/unhooked0.bin")" ]'
out=$("$B" env-edit arm "$S/hooked.bin" "$S/armed.bin" trial-0123456789abcdef-1 /yakogl/once-trial-0123456789abcdef-1.bin 2>&1)
check "arm: yako_once and yako_file set, everything else as hooked" \
    eval '[ "$out" = armed ] && [ "$(ENVT diff "$S/hooked.bin" "$S/armed.bin" | sort | paste -sd"|" -)" = "added   yako_file=/yakogl/once-trial-0123456789abcdef-1.bin|added   yako_once=trial-0123456789abcdef-1" ]'
out=$("$B" env-edit disarm "$S/armed.bin" "$S/disarmed.bin" 2>&1)
check "disarm: the variables of the hooked image back" eval '[ "$out" = disarmed ] && [ -z "$(ENVT diff "$S/hooked.bin" "$S/disarmed.bin")" ]'
out=$("$B" env-edit hook "$S/env-foreign.bin" "$S/x.bin" 8 2>&1)
rc=$?
check "hook over a foreign hook (yako_hook of another origin, no yako_bootcmd_prev): rc 3, 'not ours to touch', no output file" \
    eval '[ "$rc" = 3 ] && grep -q "not ours to touch" <<<"$out" && [ ! -e "$S/x.bin" ]'
out=$("$B" env-edit hook "$S/env-badcrc.bin" "$S/x.bin" 8 2>&1)
rc=$?
check "hook over an image whose active copy has a bad CRC: rc 2, 'does not check', no output file" \
    eval '[ "$rc" = 2 ] && grep -q "does not check" <<<"$out" && grep -q "bad CRC" <<<"$out" && [ ! -e "$S/x.bin" ]'
out=$(MP2_ENV_OFFSET2=none "$B" env-edit hook "$S/env-single.bin" "$S/hooked-single.bin" 8 2>&1)
rc=$?
check "MP2_ENV_OFFSET2=none: a single-copy layout at 0x7e000 is hooked in place" \
    eval '[ "$rc" = 0 ] && [ "$out" = installed ] && [ "$(python3 -I "$HERE/uboot_env.py" --size 0x2000 --offset 0x7e000 print "$S/hooked-single.bin" -n bootcmd)" = "run yako_hook; run bootcmd_stm32mp" ]'
out=$(MP2_ENV_OFFSET2=none "$B" env-edit unhook "$S/hooked-single.bin" "$S/unhooked-single.bin" 2>&1)
same "single copy: hook then unhook gives the input back byte for byte" "$S/env-single.bin" "$S/unhooked-single.bin"
out=$(MP2_ENV_OFFSET=0x7e100 "$B" env-edit hook "$S/env.bin" "$S/x.bin" 2>&1)
rc=$?
check "an offset that is not a sector multiple: rc 2 before any read" eval '[ "$rc" = 2 ] && grep -q "multiple of 512" <<<"$out"'
out=$("$B" env-edit hook "$S/nonexistent.bin" "$S/x.bin" 2>&1)
check "a missing image: rc 2" [ $? = 2 ]

echo "== 4. replays of synthetic consoles through the verdict (tally)"
mkrun() { # mkrun DIR KEY=VALUE...
    local d=$1
    shift
    mkdir -p "$d"
    printf 'board=stm32mp257f-dk\nboot=uboot-sd\n' > "$d/meta.txt"
    printf '%s\n' "$@" >> "$d/meta.txt"
}
# the lines the layer and mp2_identify print (stm32mp2.c, stm32mp2_wdog.c,
# tools/probes/stm32mp257f_dk/mp2_identify/src/main.c), after U-Boot's hook
UBOOT='12:00:00 yakogl-mp2: once=trial-0123456789abcdef-1
12:00:00 yakogl-mp2: cleared
12:00:01 yakogl-mp2: go 0xa0000000'
LAYER='12:00:02 MP2:B console up (USART2, polled)
12:00:02 mp2-handover: v=1 el=8 sctlr=0x30c50838 m=0 c=0 i=0 x0=0x2 x1=0xa0000000 pc=0xa0000000
12:00:02 gc_wdog_arm: armed OP-TEE arm,smc-wdt fid 0xb200005a, timeout 32 s, PET every 1000 ms
12:00:02 *** Booting Zephyr OS build selftest ***'
PASS_END='12:00:06 mp2_identify: PASS: watchdog armed (32 s), 3 PETs in 3012 ms
12:00:06 mp2: mp2_identify done: PSCI SYSTEM_RESET at 4120 ms'
d="$S/runs/20261108-1200-trial-0123abcd"
mkrun "$d" name=trial blind=0 back=1 t_reboot=1000 t_back=1070
printf '%s\n%s\n%s\n' "$UBOOT" "$LAYER" "$PASS_END" > "$d/console.txt"
out=$("$B" tally "$d" 2>&1)
rc=$?
check "PASS and the PSCI end line: rc 0, done, row, complete, result pass:mp2_identify, back_after 70s" \
    eval '[ "$rc" = 0 ] && [ "$out" = "board-stm32mp2.sh stm32mp257f-dk trial rc=0 verdict=done boot=uboot-sd row boot_verdict=complete result=pass:mp2_identify back_after=70s" ]'
d="$S/runs/20261108-1201-trial-0123abcd"
mkrun "$d" name=trial blind=0 back=1 t_reboot=1000 t_back=1075
printf '%s\n%s\n12:00:06 mp2_identify: FAIL: watchdog rc -3, 0 PETs, 0 refused (first 0)\n12:00:06 mp2: mp2_identify done: PSCI SYSTEM_RESET at 4120 ms\n' "$UBOOT" "$LAYER" > "$d/console.txt"
out=$("$B" tally "$d" 2>&1)
rc=$?
check "a FAIL line then the end: rc 4, failed, complete, result fail:mp2_identify" \
    eval '[ "$rc" = 4 ] && grep -q " rc=4 verdict=failed boot=uboot-sd row boot_verdict=complete result=fail:mp2_identify back_after=75s$" <<<"$out"'
d="$S/runs/20261108-1202-trial-0123abcd"
mkrun "$d" name=trial blind=0 back=1 t_reboot=1000 t_back=1090
printf '%s\n%s\n' "$UBOOT" "$LAYER" > "$d/console.txt"
out=$("$B" tally "$d" 2>&1)
rc=$?
check "the image started but no end line (the IWDG brought the board back): rc 4, no_end_line, image_started" \
    eval '[ "$rc" = 4 ] && grep -q " rc=4 verdict=no_end_line boot=uboot-sd row boot_verdict=image_started result=none " <<<"$out"'
d="$S/runs/20261108-1203-trial-0123abcd"
mkrun "$d" name=trial blind=0 back=1 t_reboot=1000 t_back=1090
printf '%s\n' "$UBOOT" > "$d/console.txt"
out=$("$B" tally "$d" 2>&1)
check "U-Boot's go but nothing from the image: no_end_line, uboot_go" grep -q "verdict=no_end_line boot=uboot-sd row boot_verdict=uboot_go" <<<"$out"
d="$S/runs/20261108-1204-trial-0123abcd"
mkrun "$d" name=trial blind=0 back=1 t_reboot=1000 t_back=1040
printf '12:00:00 yakogl-mp2: once=trial-1\n12:00:00 yakogl-mp2: cleared\n12:00:01 yakogl-mp2: mmc load failed, booting Linux\n' > "$d/console.txt"
out=$("$B" tally "$d" 2>&1)
check "the load failed, Linux booted: no_end_line, not_started" grep -q "verdict=no_end_line boot=uboot-sd row boot_verdict=not_started" <<<"$out"
d="$S/runs/20261108-1205-trial-0123abcd"
mkrun "$d" name=trial blind=0 back=1 t_reboot=1000 t_back=1090
: > "$d/console.txt"
out=$("$B" tally "$d" 2>&1)
check "an empty console: no_end_line, no_lines" grep -q "verdict=no_end_line boot=uboot-sd row boot_verdict=no_lines" <<<"$out"
d="$S/runs/20261108-1206-trial-0123abcd"
mkrun "$d" name=trial blind=0 back=1 t_reboot=1000 t_back=1090
printf '%s\n%s\n12:00:06 mp2: mp2_identify done: PSCI SYSTEM_RESET at 4120 ms\n12:00:06 mp2: PSCI SYSTEM_RESET returned -1: halted\n' "$UBOOT" "$LAYER" > "$d/console.txt"
out=$("$B" tally "$d" 2>&1)
rc=$?
check "the end line and then PSCI returned: the end line wins (complete, done); the halted line is in the record" \
    eval '[ "$rc" = 0 ] && grep -q "verdict=done boot=uboot-sd row boot_verdict=complete" <<<"$out"'
d="$S/runs/20261108-1207-trial-0123abcd"
mkrun "$d" name=trial blind=0 back=1 t_reboot=1000 t_back=1090
printf '%s\n%s\n12:00:06 mp2: PSCI SYSTEM_RESET returned -1: halted\n' "$UBOOT" "$LAYER" > "$d/console.txt"
out=$("$B" tally "$d" 2>&1)
rc=$?
check "PSCI returned without the end line: rc 4, psci_returned" eval '[ "$rc" = 4 ] && grep -q "verdict=psci_returned boot=uboot-sd row boot_verdict=psci_returned" <<<"$out"'
d="$S/runs/20261108-1208-trial-0123abcd"
mkrun "$d" name=trial blind=0 back=0 t_reboot=1000 t_back=1300 rc_cycle=5
printf '%s\n%s\n%s\n' "$UBOOT" "$LAYER" "$PASS_END" > "$d/console.txt"
out=$("$B" tally "$d" 2>&1)
rc=$?
check "the board did not come back: rc 5, not_back, back_after absent, even with a PASS" \
    eval '[ "$rc" = 5 ] && grep -q " rc=5 verdict=not_back boot=uboot-sd row boot_verdict=complete result=pass:mp2_identify back_after=absent$" <<<"$out"'
d="$S/runs/20261108-1209-trial-0123abcd"
mkrun "$d" name=trial blind=1 back=1 t_reboot=1000 t_back=1060
out=$("$B" tally "$d" 2>&1)
rc=$?
check "a blind run that came back: rc 0, returned, blind (never a row), no_console" \
    eval '[ "$rc" = 0 ] && grep -q " rc=0 verdict=returned boot=uboot-sd blind boot_verdict=no_console result=none back_after=60s$" <<<"$out"'
d="$S/runs/20261108-1210-trial-0123abcd"
mkrun "$d" name=trial blind=0 back=1 t_reboot=1000 t_back=1070 rc_cycle=6
printf '%s\n%s\n%s\n' "$UBOOT" "$LAYER" "$PASS_END" > "$d/console.txt"
out=$("$B" tally "$d" 2>&1)
rc=$?
check "a one-shot still set after the cycle: rc 6, the verdict stays done" eval '[ "$rc" = 6 ] && grep -q " rc=6 verdict=done " <<<"$out"'
d="$S/runs/20261108-1211-trial-0123abcd"
mkrun "$d" name=trial blind=0 back=1 t_reboot=1000 t_back=1070
printf '%s\n%s\n12:00:06 mp2_identify: PASS: ok\n12:00:07 mp2: lifetime: PSCI SYSTEM_RESET at 20001 ms\n' "$UBOOT" "$LAYER" > "$d/console.txt"
out=$(BOARD_END_RE='^never matches' "$B" tally "$d" 2>&1)
check "BOARD_END_RE overrides the end line" grep -q "verdict=no_end_line" <<<"$out"
out=$("$B" tally "$d" 2>&1)
check "the layer's lifetime reset is an end line too (any <why>)" grep -q "verdict=done .*boot_verdict=complete result=pass:mp2_identify" <<<"$out"
out=$("$B" tally "$S/runs/nonexistent" 2>&1)
check "tally of a directory without meta.txt: rc 2" [ $? = 2 ]

echo "== 5. redaction of a run directory"
# The runner redacts a run's copies at the end of a real run only (tally is
# read-only), so redact_run and what it needs are extracted from the script
# by name and run on a directory whose console names the board's host, the
# bridge, the console device and two addresses.
mkdir -p "$S/redact2/run"
printf 'meta=1\n' > "$S/redact2/run/meta.txt"
printf '12:00:00 login from 198.51.100.13 to mp2board.example.lab (198.51.100.13) and 203.0.113.8\n12:00:01 console bridgehost.example.lab:40001 via bridgehost.example.lab\n12:00:02 the device /dev/ttyFAKE0 is <nothing>\n' > "$S/redact2/run/console.txt"
bash -c '
set -uo pipefail
HERE=$1; d=$2
BOARD_HOST=mp2board.example.lab BOARD_UART=bridgehost.example.lab:40001 BHOSTNAME=mp2board.example.lab UART_REAL=/dev/ttyFAKE0
eval "$(sed -n "/^uart_is_bridge()/p; /^redact_words() {/,/^}/p; /^redact_run() {/,/^}/p" "$HERE/board-stm32mp2.sh")"
redact_run "$d"
' _ "$HERE" "$S/redact2/run"
out=$(cat "$S/redact2/run/console.txt")
check "redaction: the board's host name -> <board-host>, the bridge host:port -> <BOARD_UART>, its host alone -> <uart-bridge>" \
    eval 'grep -q "to <board-host> (" <<<"$out" && grep -q "console <BOARD_UART> via <uart-bridge>" <<<"$out" && ! grep -q "example.lab" <<<"$out"'
check "redaction: addresses -> 192.0.2.n in the order first seen, the same address the same n" \
    eval 'grep -q "from 192.0.2.1 to <board-host> (192.0.2.1) and 192.0.2.2" <<<"$out"'
check "redaction: the console device's path -> <BOARD_UART>" grep -q "the device <BOARD_UART> is <nothing>" <<<"$out"
check "redaction: meta.txt untouched when it names nothing" grep -qx "meta=1" "$S/redact2/run/meta.txt"

echo "board_stm32mp2_selftest: $n checks, $fails failed (pinned: $WANT_CHECKS)"
[ "$fails" = 0 ] && [ "$n" = "$WANT_CHECKS" ]

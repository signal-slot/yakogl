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
# 2. The refusals that need no board: an image without the watchdog marker
#    (to run and to install), one too big for the 256 MiB window, no AArch64
#    header, a bad run name, a run without BOARD_UART (and the blind run
#    MP2_NO_CONSOLE=1 allows) or without BOARD_HOST, an unknown verb; and the
#    device guard on its own: the eMMC and every never-written label refused,
#    the card's u-boot-env and bootfs accepted, a partition with the wrong
#    label refused.
# 3. The environment edits on a synthetic redundant image (built here from
#    the format, as tools/testdata/uboot-env/test_uboot_env.py does: no byte
#    from a board): hook writes the hook's variables and keeps bootcmd as it
#    is in yako_bootcmd_prev, is idempotent, upgrades another version's strings, refuses a foreign hook,
#    never writes bootcmd; unhook gives the variables of the input back once
#    bootcmd is ST's again; arm and disarm; a bad CRC
#    and a single-copy layout; the hook strings themselves are pinned.
# 4. Replays of synthetic consoles through the verdict (tally): the
#    mp2_identify PASS run is done (rc 0), a FAIL line failed (rc 4), no end
#    line no_end_line with the boot_verdict naming how far it got, PSCI
#    returning psci_returned, a board that did not come back not_back (rc 5),
#    a blind run returned, a one-shot left over rc 6.
# 5. Redaction of a run directory: host names, addresses and the console's
#    path or bridge become placeholders in the copies.
# 6. A fake board: ssh is a script that logs every command and runs it in a
#    scratch tree (/boot, the u-boot-env partition as a file, /proc/uptime
#    rewritten there; fw_printenv is tools/uboot_env.py on that file; the
#    reboot clears yako_once as U-Boot's hook does and starts a new "uptime"),
#    the console a file with the layer's lines. Through it the live paths run
#    without --dry-run: install (the hook's variables in the partition,
#    bootcmd left to a person, who is simulated by editing the partition
#    file; the pristine image kept, the stock boot files hashed), status, a run that comes back (the
#    verdict line byte for byte, the record, the staged image removed, the
#    one-shot gone), a run whose board never answers again (not_back, rc 5,
#    the NEEDS A PERSON line, one reboot and no retry), the eMMC refused
#    before any write, disarm, and uninstall after which the stock files are
#    byte for byte what they were and the environment's variables the input's.
#    The command log never names mmcblk0, a module, tailscale or sshd, and
#    every dd writes the u-boot-env partition alone.
# 7. The expected-watchdog run (run --expect-watchdog, the hang option of
#    mp2_hello): the dry run waits WDOG_BOUND, replays of consoles with the
#    hang line give watchdog_reset only when the banner and the hang line
#    were seen, no PSCI line followed and the board came back; the fake board
#    runs it live.
# 8. MP2_HOLD_S: the dry run's wait is extended by the hold, a bad value is
#    refused (rc 2), the live run prints the "hold:" line once the PASS line
#    shows, records hold_s and the extended wait_bound, keeps the verdict
#    line; without it no "hold:" line and no hold_s (cases in 1/2 and 6).
# 9. The runner-safety-audit gaps (the G numbers in the checks' names):
#    bootcmd is ST's and never written (G4: install writes the yako_*
#    variables and stops with NEEDS A PERSON, rc 7, until a person puts the
#    hook in front of bootcmd at U-Boot's prompt; uninstall the same the other
#    way round; run refuses meanwhile); the card is the SD card /boot is on by
#    its sysfs type, an eMMC numbered other than 0 refused (G1); ssh is strict
#    about the host key and records none (G7); a reboot that did not happen
#    is disarmed and reported not_rebooted, not NEEDS A PERSON (G10); the
#    lock's watchdog TERM during the reboot's ssh ends the run at once
#    through its undo, a TERM during an environment write lets the write
#    finish, a SIGKILL leaves the state file that the next run shows and
#    disarm clears (G11); the UNDO before staging, cut-off copies (.part),
#    a board read that fails is not "unset", the lock's HOLD_MAX over the
#    run's own bounds, the cycle's own disarm, uninstall's UNDO and its
#    check of the stock files before the record goes.
# Prints "board_stm32mp2_selftest: N checks, F failed"; exit 0 only when F is
# 0 and N is the pinned count. About 90 s (the live runs poll every 5 s; the
# watchdog case waits 12 s).
set -uo pipefail
export LC_ALL=C
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
B="$HERE/board-stm32mp2.sh"
WANT_CHECKS=171

S=$(mktemp -d)
trap 'rm -rf "$S"' EXIT
n=0
fails=0
check() { # check DESCRIPTION COMMAND...
    local what=$1
    shift
    n=$((n + 1))
    if "$@"; then
        echo "  PASS  $what"
    else
        echo "  FAIL  $what"
        fails=$((fails + 1))
        # MP2_SELFTEST_DEBUG=1: the last captured output, for a failing check
        [ "${MP2_SELFTEST_DEBUG:-0}" = 1 ] && [ -n "${out:-}" ] && sed 's/^/        | /' <<<"$out" | head -40
    fi
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
out=$(BOARD_HOST=$FAKE_HOST dry install 2>&1)
check "install (G4): the edit never names bootcmd (ST's variable); yako_bootcmd_prev is recorded" \
    eval 'l=$(grep "uboot_env.py .* set <env.bin>" <<<"$out"); [ -n "$l" ] && ! grep -q "'"'"'bootcmd=" <<<"$l" && grep -q "yako_bootcmd_prev=" <<<"$l" && grep -q "bootcmd is ST.s: never written" <<<"$l"'
check "install (G1): the card is the disk /boot is on, and its sysfs type is read" \
    eval 'grep -q "/sys/block/\${p##\*/}/device/type" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST dry uninstall 2>&1)
check "uninstall: the variables unset (bootcmd never written), the UNDO first, the stock files checked, then the staging directory removed" \
    eval 'grep -q "unset <env.bin> <env.new> yako_hook yako_mmc yako_addr yako_part yako_bootcmd_prev yako_once yako_file   # bootcmd is ST.s: never written" <<<"$out" &&
          u=$(grep -n "UNDO, should the uninstall" <<<"$out" | cut -d: -f1); w=$(grep -n "dd of=" <<<"$out" | head -1 | cut -d: -f1);
          c=$(grep -n "sha256sum -c /boot/yakogl/backup/boot-sha256.txt" <<<"$out" | cut -d: -f1); r=$(grep -n "rm -rf /boot/yakogl" <<<"$out" | cut -d: -f1);
          [ -n "$u" ] && [ -n "$w" ] && [ -n "$c" ] && [ -n "$r" ] && [ "$u" -lt "$w" ] && [ "$c" -lt "$r" ] && ! grep -q "bootcmd=<" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST dry disarm 2>&1)
check "disarm: yako_once and yako_file unset, the staged once-*.bin and a cut-off copy's once-*.bin.part removed, then listed empty" \
    eval 'grep -q "unset <env.bin> <env.new> yako_once yako_file" <<<"$out" && grep -q "rm -f /boot/yakogl/once-\*.bin /boot/yakogl/once-\*.bin.part" <<<"$out" &&
          grep -q "ls /boot/yakogl/once-\* 2>/dev/null   # must print nothing" <<<"$out"'
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
    eval 'grep -q "fw_printenv bootcmd yako_hook" <<<"$out" && grep -q "fw_printenv -n yako_once" <<<"$out" &&grep -q "ls /boot/yakogl/once-\* " <<<"$out" && grep -q "df -B1 --output=avail /boot" <<<"$out"'
check "run: the UNDO ('disarm') is printed before the image is staged, and names the cut-off copy" \
    eval 'u=$(grep -n "UNDO, should this run be cut off" <<<"$out" | cut -d: -f1); s=$(grep -n "cat > /boot/yakogl/once-trial-" <<<"$out" | cut -d: -f1);
          [ -n "$u" ] && [ -n "$s" ] && [ "$u" -lt "$s" ] && grep -q "tools/board-stm32mp2.sh disarm   # unsets yako_once and yako_file, removes /boot/yakogl/once-\*.bin and once-\*.bin.part" <<<"$out"'
check "run: the lock's HOLD_MAX covers the wait, the copy and the checks (300 + 900 + 300 = 1500)" grep -q "HOLD_MAX=1500," <<<"$out"
check "run (G10): the wait names the reboot grace (the old boot still answering: not_rebooted, disarmed)" grep -q "min(90, 300) s: not_rebooted, disarmed" <<<"$out"
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
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART dry run --expect-watchdog "$S/img/mp2.bin" "$S/logs" 2>&1)
rc=$?
check "run --expect-watchdog: rc 0, the wait is WDOG_BOUND (default 120 s), not RUN_BOUND + RETURN_BOUND" \
    eval '[ "$rc" = 0 ] && grep -q "<= 120 s" <<<"$out" && ! grep -q "<= 300 s" <<<"$out" && ! leaks "$out"'
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART WDOG_BOUND=90 RUN_BOUND=200 dry run --expect-watchdog "$S/img/mp2.bin" "$S/logs" wdog 2>&1)
check "run --expect-watchdog: WDOG_BOUND sets the wait" eval 'grep -q "<= 90 s" <<<"$out"'
check "dry runs: nothing written on the host, no stub ran" eval '[ ! -e "$S/logs" ] && untouched'

echo "== 2. refusals that need no board"
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART MP2_HOLD_S=30 dry run "$S/img/mp2.bin" "$S/logs" hold 2>&1)
rc=$?
check "dry run with MP2_HOLD_S=30: rc 0, the wait is RUN_BOUND + RETURN_BOUND + 30 (<= 330 s), no hold line yet" \
    eval '[ "$rc" = 0 ] && grep -q "<= 330 s" <<<"$out" && ! grep -q "^hold:" <<<"$out"'
check "dry run with MP2_HOLD_S=30: the lock's HOLD_MAX grows with the hold (1530)" grep -q "HOLD_MAX=1530," <<<"$out"
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART MP2_REBOOT_GRACE=soon dry run "$S/img/mp2.bin" "$S/logs" 2>&1)
rc=$?
check "MP2_REBOOT_GRACE that is not whole seconds: rc 2, named" eval '[ "$rc" = 2 ] && grep -q "MP2_REBOOT_GRACE must be whole seconds" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART MP2_HOLD_S=30 WDOG_BOUND=90 dry run --expect-watchdog "$S/img/mp2.bin" "$S/logs" holdw 2>&1)
check "dry run --expect-watchdog with MP2_HOLD_S=30: the wait is WDOG_BOUND + 30 (<= 120 s)" eval 'grep -q "<= 120 s" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART MP2_HOLD_S=abc dry run "$S/img/mp2.bin" "$S/logs" 2>&1)
rc=$?
check "MP2_HOLD_S that is not whole seconds: rc 2, named" eval '[ "$rc" = 2 ] && grep -q "MP2_HOLD_S must be whole seconds" <<<"$out"'
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
out=$(BOARD_UART=$FAKE_UART PATH="$S/stub:$PATH" "$B" run "$S/img/mp2.bin" "$S/logs" 2>&1)
rc=$?
check "a real run without BOARD_HOST: rc 2 before the board is touched, names BOARD_HOST and the lease" \
    eval '[ "$rc" = 2 ] && grep -q "set BOARD_HOST" <<<"$out" && grep -q "lease" <<<"$out" && untouched'
for verb in status install uninstall disarm; do
    out=$(PATH="$S/stub:$PATH" "$B" "$verb" 2>&1)
    rc=$?
    check "a real $verb without BOARD_HOST: rc 2, nothing touched" eval '[ "$rc" = 2 ] && grep -q "set BOARD_HOST" <<<"$out" && untouched'
done
out=$(BOARD_HOST=$FAKE_HOST dry install "$S/img/nomarker.bin" 2>&1)
rc=$?
check "install with an image without the marker: rc 3, REFUSED before any board line" \
    eval '[ "$rc" = 3 ] && grep -q "REFUSED" <<<"$out" && ! grep -q "^DRY" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST dry install "$S/img/mp2.bin" 2>&1)
rc=$?
check "install <image> (dry run): rc 0, the UNDO before the first change command (every dd of=, cp, mkdir and uboot_env.py set)" \
    eval 'u=$(grep -n "UNDO, should anything" <<<"$out" | cut -d: -f1); w=$(grep -nE "dd of=|mkdir -p|cp <env.bin>|uboot_env.py .* set" <<<"$out" | head -1 | cut -d: -f1);
          [ "$rc" = 0 ] && [ -n "$u" ] && [ -n "$w" ] && [ "$u" -lt "$w" ] && grep -q "tools/board-stm32mp2.sh uninstall" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST dry nosuchverb 2>&1)
rc=$?
check "an unknown verb: rc 2, the usage names run, install [image] and selftest" \
    eval '[ "$rc" = 2 ] && grep -q "tools/board-stm32mp2.sh \[--dry-run\] run" <<<"$out" && grep -q "install \[image\]" <<<"$out" && grep -q "tools/board-stm32mp2.sh selftest" <<<"$out"'
check "the selftest verb runs this script" grep -q '^selftest) exec bash "$HERE/board_stm32mp2_selftest.sh" ;;' "$B"
out=$("$B" 2>&1)
check "no arguments: rc 2" [ $? = 2 ]
check "the refusals touched no stub and wrote nothing" eval '[ ! -e "$S/logs" ] && untouched'

# the device guard alone (dev_guard, its constants and die, extracted by
# name as section 5 does): the card's layout as #1281 lists it
CARD_TABLE='/dev/mmcblk1
/dev/mmcblk1p1 fsbla1
/dev/mmcblk1p2 fsbla2
/dev/mmcblk1p3 metadata1
/dev/mmcblk1p4 metadata2
/dev/mmcblk1p5 fip-a
/dev/mmcblk1p6 fip-b
/dev/mmcblk1p7 u-boot-env
/dev/mmcblk1p8 bootfs
/dev/mmcblk1p9 vendorfs
/dev/mmcblk1p10 rootfs
/dev/mmcblk1p11 userfs'
guard() { # guard DEV WANT-LABEL [TABLE [CARD]]: the guard's rc and message
    bash -c '
set -uo pipefail
B=$1; dev=$2; want=$3; PART_TABLE=$4; CARD_DISK=$5; TAG=guard
eval "$(sed -n "/^NEVER_LABELS=/p; /^EMMC_RE=/p; /^PART_RE=/p; /^scrub() {/,/^}/p; /^die() /p; /^dev_guard() {/,/^}/p" "$B")"
dev_guard "$dev" "$want" && echo ok
' _ "$B" "$1" "$2" "${3:-$CARD_TABLE}" "${4-/dev/mmcblk1}" 2>&1
}
for p in 1:fsbla1 2:fsbla2 3:metadata1 4:metadata2 5:fip-a 6:fip-b 9:vendorfs 10:rootfs 11:userfs; do
    out=$(guard "/dev/mmcblk1p${p%%:*}" "${p#*:}")
    rc=$?
    check "guard: /dev/mmcblk1p${p%%:*} (${p#*:}) refused even when asked for by its own label, rc 3" \
        eval '[ "$rc" = 3 ] && grep -q "REFUSED: /dev/mmcblk1p${p%%:*} is labelled ${p#*:}: never written" <<<"$out"'
done
out=$(guard /dev/mmcblk0p7 u-boot-env "$(sed 's/mmcblk1/mmcblk0/' <<<"$CARD_TABLE")")
rc=$?
check "guard: the eMMC's u-boot-env (/dev/mmcblk0p7, the same label) refused as the eMMC, rc 3" \
    eval '[ "$rc" = 3 ] && grep -q "is the eMMC (mmcblk0)" <<<"$out"'
out=$(guard /dev/mmcblk0 u-boot-env)
rc=$?
check "guard: /dev/mmcblk0 itself refused" eval '[ "$rc" = 3 ] && grep -q "is the eMMC" <<<"$out"'
out=$(guard /dev/mmcblk1 u-boot-env)
rc=$?
check "guard: a whole card (/dev/mmcblk1) is not a partition: refused" eval '[ "$rc" = 3 ] && grep -q "not a partition" <<<"$out"'
out=$(guard /dev/mmcblk1p8 u-boot-env)
rc=$?
check "guard: bootfs asked for as u-boot-env: refused (wrong label)" eval '[ "$rc" = 3 ] && grep -q "labelled .bootfs., not u-boot-env" <<<"$out"'
out=$(guard /dev/mmcblk1p12 u-boot-env)
rc=$?
check "guard: a partition absent from the table: refused ('nothing')" eval '[ "$rc" = 3 ] && grep -q "labelled .nothing." <<<"$out"'
check "guard: the card's u-boot-env (p7) and bootfs (p8) pass" \
    eval '[ "$(guard /dev/mmcblk1p7 u-boot-env)" = ok ] && [ "$(guard /dev/mmcblk1p8 bootfs)" = ok ]'
out=$(guard /dev/mmcblk2p7 u-boot-env "$CARD_TABLE
/dev/mmcblk2p7 u-boot-env")
rc=$?
check "guard (G1): a u-boot-env partition of another mmc device (an eMMC numbered 2) refused: not the SD card /boot is on" \
    eval '[ "$rc" = 3 ] && grep -q "REFUSED: /dev/mmcblk2p7 is not a partition of the SD card /boot is on (/dev/mmcblk1)" <<<"$out"'
out=$(guard /dev/mmcblk1p7 u-boot-env "$CARD_TABLE" "")
rc=$?
check "guard (G1): no SD card found (CARD_DISK empty): every device refused" \
    eval '[ "$rc" = 3 ] && grep -q "(none found)" <<<"$out"'

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
out=$("$B" env-edit hook "$S/env.bin" "$S/vars.bin" 8 2>&1)
rc=$?
check "hook (G4): rc 0, 'installed', bootcmd not hooked: the line a person types is named; the output checks (both CRCs), copy 2 written and active" \
    eval '[ "$rc" = 0 ] && [ "$out" = "installed; bootcmd not hooked: setenv bootcmd '"'"'run yako_hook; run bootcmd_stm32mp'"'"'; saveenv" ] && ENVT check "$S/vars.bin" | grep -q "^active: copy 2"'
check "hook (G4): bootcmd is NOT written (ST's variable), yako_bootcmd_prev keeps it, bootdelay and the rest untouched" \
    eval '[ "$(ENVT print "$S/vars.bin" -n bootcmd)" = "run bootcmd_stm32mp" ] &&
          [ "$(ENVT print "$S/vars.bin" -n yako_bootcmd_prev)" = "run bootcmd_stm32mp" ] &&
          [ "$(vals "$S/vars.bin" bootdelay boot_targets fdtfile | paste -sd"|" -)" = "1|mmc0 mmc1|st/stm32mp257f-dk.dtb" ]'
# the person's edit at U-Boot's prompt, as the line above says it (not the runner's)
ENVT set "$S/vars.bin" "$S/hooked.bin" "bootcmd=run yako_hook; run bootcmd_stm32mp" >/dev/null
out=$("$B" env-edit hook "$S/hooked.bin" "$S/hooked1.bin" 8 2>&1)
check "hook after a person put the hook in bootcmd: 'unchanged', the image byte for byte" eval '[ "$out" = unchanged ] && cmp -s "$S/hooked.bin" "$S/hooked1.bin"'
check "hook: yako_hook clears and saves the one-shot BEFORE the load, then runs yako_mmc; a failed saveenv boots Linux" \
    eval 'h=$(ENVT print "$S/hooked.bin" -n yako_hook); [ "$h" = "if test -n \"\${yako_once}\"; then echo yakogl-mp2: once=\${yako_once}; setenv yako_once; if saveenv; then echo yakogl-mp2: cleared; run yako_mmc; else echo yakogl-mp2: saveenv failed, booting Linux; fi; fi" ]'
check "hook: yako_mmc loads from mmc \${boot_instance}:\${yako_part} (no device number hard-coded), caches off, go; a failed load boots Linux" \
    eval 'm=$(ENVT print "$S/hooked.bin" -n yako_mmc); [ "$m" = "if ext4load mmc \${boot_instance}:\${yako_part} \${yako_addr} \${yako_file}; then echo yakogl-mp2: go \${yako_addr}; dcache off; icache off; go \${yako_addr}; fi; echo yakogl-mp2: mmc load failed, booting Linux" ]'
check "hook: yako_addr 0xa0000000 (the link address), yako_part 8, no yako_once or yako_file" \
    eval '[ "$(vals "$S/hooked.bin" yako_addr yako_part yako_once yako_file | paste -sd"|" -)" = "0xa0000000|8|-|-" ]'
check "hook: the bytes outside the two copies are untouched" \
    eval 'cmp -s <(head -c $((0x7c000)) "$S/env.bin") <(head -c $((0x7c000)) "$S/vars.bin")'
"$B" env-edit hook "$S/vars.bin" "$S/vars2.bin" 8 >/dev/null 2>&1
check "hook twice without the person's edit: the image byte for byte (still not hooked)" cmp -s "$S/vars.bin" "$S/vars2.bin"
out=$("$B" env-edit hook "$S/hooked.bin" "$S/hooked9.bin" 9 2>&1)
check "hook with another partition number: 'upgraded', bootcmd as the person left it, yako_part 9" \
    eval '[ "$out" = upgraded ] && [ "$(ENVT print "$S/hooked9.bin" -n bootcmd)" = "run yako_hook; run bootcmd_stm32mp" ] && [ "$(ENVT print "$S/hooked9.bin" -n yako_part)" = 9 ]'
out=$("$B" env-edit unhook "$S/hooked.bin" "$S/unhooked-h.bin" 2>&1)
rc=$?
check "unhook while bootcmd still begins with the hook (G4): 'bootcmd still hooked', bootcmd not written, the hook kept, the one-shot gone" \
    eval '[ "$rc" = 0 ] && [ "$out" = "bootcmd still hooked" ] && [ "$(vals "$S/unhooked-h.bin" bootcmd yako_hook yako_once | sed "s/^if test.*/HOOK/" | paste -sd"|" -)" = "run yako_hook; run bootcmd_stm32mp|HOOK|-" ]'
out=$("$B" env-edit unhook "$S/vars.bin" "$S/unhooked.bin" 2>&1)
rc=$?
check "unhook once bootcmd is ST's again: rc 0, 'unhooked', the variables of the input back (no yako_*), both CRCs check" \
    eval '[ "$rc" = 0 ] && [ "$out" = unhooked ] && [ -z "$(ENVT diff "$S/env.bin" "$S/unhooked.bin")" ] && ENVT check "$S/unhooked.bin" >/dev/null'
out=$("$B" env-edit unhook "$S/env.bin" "$S/unhooked0.bin" 2>&1)
check "unhook of an image without the hook: 'unhooked', the variables unchanged" \
    eval '[ "$out" = unhooked ] && [ -z "$(ENVT diff "$S/env.bin" "$S/unhooked0.bin")" ]'
check "G4: no edit of the runner changed bootcmd (hook, upgrade, unhook: each output's bootcmd is its input's)" \
    eval '[ "$(ENVT print "$S/vars.bin" -n bootcmd)" = "$(ENVT print "$S/env.bin" -n bootcmd)" ] && [ "$(ENVT print "$S/hooked9.bin" -n bootcmd)" = "$(ENVT print "$S/hooked.bin" -n bootcmd)" ] &&
          [ "$(ENVT print "$S/unhooked-h.bin" -n bootcmd)" = "$(ENVT print "$S/hooked.bin" -n bootcmd)" ] && [ "$(ENVT print "$S/unhooked.bin" -n bootcmd)" = "$(ENVT print "$S/vars.bin" -n bootcmd)" ]'
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
check "MP2_ENV_OFFSET2=none: a single-copy layout at 0x7e000 gets the hook's variables in place, bootcmd untouched" \
    eval '[ "$rc" = 0 ] && [ "${out%%;*}" = installed ] && [ "$(python3 -I "$HERE/uboot_env.py" --size 0x2000 --offset 0x7e000 print "$S/hooked-single.bin" -n bootcmd)" = "run bootcmd_stm32mp" ] &&
          [ "$(python3 -I "$HERE/uboot_env.py" --size 0x2000 --offset 0x7e000 print "$S/hooked-single.bin" -n yako_addr)" = 0xa0000000 ]'
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
HANG='12:00:03 mp2_hello: hanging on purpose with interrupts locked, the OP-TEE watchdog must reset'
d="$S/runs/20261108-1212-wdog-0123abcd"
mkrun "$d" name=wdog blind=0 back=1 t_reboot=1000 t_back=1070 expect_wdog=1
printf '%s\n%s\n%s\n' "$UBOOT" "$LAYER" "$HANG" > "$d/console.txt"
out=$("$B" tally "$d" 2>&1)
rc=$?
check "expected watchdog: banner, hang line, no PSCI line, back: rc 0, watchdog_reset, hung, back_after 70s" \
    eval '[ "$rc" = 0 ] && [ "$out" = "board-stm32mp2.sh stm32mp257f-dk wdog rc=0 verdict=watchdog_reset boot=uboot-sd row boot_verdict=hung result=none back_after=70s" ]'
d="$S/runs/20261108-1213-wdog-0123abcd"
mkrun "$d" name=wdog blind=0 back=1 t_reboot=1000 t_back=1070 expect_wdog=1
printf '%s\n%s\n%s\n12:00:20 mp2: lifetime over: PSCI SYSTEM_RESET at 20001 ms\n' "$UBOOT" "$LAYER" "$HANG" > "$d/console.txt"
out=$("$B" tally "$d" 2>&1)
rc=$?
check "expected watchdog but a PSCI reset line followed: rc 4, psci_reset_before_wdog" \
    eval '[ "$rc" = 4 ] && grep -q " rc=4 verdict=psci_reset_before_wdog boot=uboot-sd row boot_verdict=complete " <<<"$out"'
d="$S/runs/20261108-1214-wdog-0123abcd"
mkrun "$d" name=wdog blind=0 back=1 t_reboot=1000 t_back=1070 expect_wdog=1
printf '%s\n%s\n%s\n12:00:20 mp2: PSCI SYSTEM_RESET returned -1: halted\n' "$UBOOT" "$LAYER" "$HANG" > "$d/console.txt"
out=$("$B" tally "$d" 2>&1)
rc=$?
check "expected watchdog but PSCI returned: rc 4, psci_returned" eval '[ "$rc" = 4 ] && grep -q "verdict=psci_returned " <<<"$out"'
d="$S/runs/20261108-1215-wdog-0123abcd"
mkrun "$d" name=wdog blind=0 back=1 t_reboot=1000 t_back=1070 expect_wdog=1
printf '%s\n%s\n' "$UBOOT" "$LAYER" > "$d/console.txt"
out=$("$B" tally "$d" 2>&1)
rc=$?
check "expected watchdog, banner but no hang line: rc 4, no_hang, image_started" \
    eval '[ "$rc" = 4 ] && grep -q " rc=4 verdict=no_hang boot=uboot-sd row boot_verdict=image_started " <<<"$out"'
d="$S/runs/20261108-1216-wdog-0123abcd"
mkrun "$d" name=wdog blind=0 back=1 t_reboot=1000 t_back=1070 expect_wdog=1
printf '%s\n%s\n' "$UBOOT" "$HANG" > "$d/console.txt"
out=$("$B" tally "$d" 2>&1)
check "expected watchdog, a hang line without the banner: not a pass (no_end_line, uboot_go)" \
    grep -q "rc=4 verdict=no_end_line boot=uboot-sd row boot_verdict=uboot_go" <<<"$out"
d="$S/runs/20261108-1217-wdog-0123abcd"
mkrun "$d" name=wdog blind=0 back=0 t_reboot=1000 t_back=1300 rc_cycle=5 expect_wdog=1
printf '%s\n%s\n%s\n' "$UBOOT" "$LAYER" "$HANG" > "$d/console.txt"
out=$("$B" tally "$d" 2>&1)
rc=$?
check "expected watchdog but Linux never came back: rc 5, not_back" \
    eval '[ "$rc" = 5 ] && grep -q " rc=5 verdict=not_back boot=uboot-sd row boot_verdict=hung " <<<"$out"'
d="$S/runs/20261108-1218-hang-0123abcd"
mkrun "$d" name=hang blind=0 back=1 t_reboot=1000 t_back=1070
printf '%s\n%s\n%s\n' "$UBOOT" "$LAYER" "$HANG" > "$d/console.txt"
out=$("$B" tally "$d" 2>&1)
check "a hang line in a run that did not expect the watchdog: still no_end_line, image_started" \
    grep -q "rc=4 verdict=no_end_line boot=uboot-sd row boot_verdict=image_started " <<<"$out"
d="$S/runs/20261108-1219-norb-0123abcd"
mkrun "$d" name=norb blind=0 back=1 rebooted=0 t_reboot=1000 t_back=1095
: > "$d/console.txt"
out=$("$B" tally "$d" 2>&1)
rc=$?
check "G10: the reboot did not happen (rebooted=0, the board still up): rc 4, not_rebooted, back_after absent, no NEEDS A PERSON" \
    eval '[ "$rc" = 4 ] && grep -q " rc=4 verdict=not_rebooted boot=uboot-sd row boot_verdict=no_lines result=none back_after=absent$" <<<"$out" && ! grep -q "NEEDS A PERSON" <<<"$out"'
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

echo "== 6. the live paths against a fake board (a fake ssh, a fake console)"
F="$S/fake"
mkdir -p "$F/dev" "$F/boot/mmc0_extlinux" "$F/boot/mmc1_extlinux" "$F/proc" "$F/state" "$F/bin" "$S/fakebin" \
    "$F/sys/block/mmcblk1/device" "$F/sys/block/mmcblk2/device"
# the MMC core's word for each disk: the card is an SD card, mmcblk2 an eMMC
# that is not numbered 0 (G1)
echo SD > "$F/sys/block/mmcblk1/device/type"
echo MMC > "$F/sys/block/mmcblk2/device/type"
cp "$S/env.bin" "$F/dev/mmcblk1p7"
# the stock boot files of the card (names as #1281 lists them; contents fake)
printf 'stock Image.gz\n' > "$F/boot/Image.gz"
printf 'stock boot.scr.uimg\n' > "$F/boot/boot.scr.uimg"
printf 'stock stm32mp257f-dk.dtb\n' > "$F/boot/stm32mp257f-dk.dtb"
printf 'label stm32mp257f-dk\n  kernel /Image.gz\n' > "$F/boot/mmc0_extlinux/extlinux.conf"
printf 'label stm32mp257f-dk\n  kernel /Image.gz\n' > "$F/boot/mmc1_extlinux/extlinux.conf"
printf '100.00 300.00\n' > "$F/proc/uptime"
printf '/dev/mmcblk1p8 ext4\n' > "$F/state/findmnt"
printf '%s\n' "$CARD_TABLE" > "$F/state/lsblk"
# the console as the layer and mp2_identify would print it (section 4's lines, no stamps)
printf '%s\n%s\n%s\n' "$UBOOT" "$LAYER" "$PASS_END" | sed 's/^[0-9:]\{8\} //' > "$F/console.txt"
manifest() { (cd "$F/boot" && find . -path ./yakogl -prune -o -type f -print0 | sort -z | xargs -0 sha256sum); }
manifest > "$S/boot-before.txt"
# the commands the fake ssh runs: fw_printenv is uboot_env.py on the
# partition file; the reboot clears yako_once (U-Boot's hook: setenv, saveenv)
# unless state/mode says otherwise, and starts a new boot (uptime 1 s) or,
# in mode "die", never answers again; "noreboot" does nothing (the old boot
# goes on answering), "hang" holds the ssh session 20 s and does nothing
cat > "$F/bin/fw_printenv" <<EOF
#!/bin/sh
exec python3 -I "$HERE/uboot_env.py" --redundant --size 0x2000 --offset 0x7e000 --offset2 0x7c000 print "$F/dev/mmcblk1p7" "\$@"
EOF
cat > "$F/bin/findmnt" <<EOF
#!/bin/sh
cat "$F/state/findmnt"
EOF
cat > "$F/bin/lsblk" <<EOF
#!/bin/sh
cat "$F/state/lsblk"
EOF
printf '#!/bin/sh\necho fakeboard\n' > "$F/bin/uname"
printf '#!/bin/sh\nexit 0\n' > "$F/bin/who"
cat > "$F/bin/systemctl" <<EOF
#!/bin/sh
case "\$1" in
is-system-running) exit 0 ;;
reboot)
    echo reboot >> "$F/state/reboots"
    mode=\$(cat "$F/state/mode" 2>/dev/null)
    case "\$mode" in
    die) touch "$F/state/dead" ;;
    noclear) printf '1.00 1.00\n' > "$F/proc/uptime" ;;
    noreboot) ;;
    hang) sleep 20 ;;
    *)
        python3 -I "$HERE/uboot_env.py" --redundant --size 0x2000 --offset 0x7e000 --offset2 0x7c000 unset "$F/dev/mmcblk1p7" "$F/dev/mmcblk1p7.new" yako_once >/dev/null &&
            mv "$F/dev/mmcblk1p7.new" "$F/dev/mmcblk1p7"
        printf '1.00 1.00\n' > "$F/proc/uptime" ;;
    esac ;;
*) echo "fake systemctl: \$*" >&2; exit 1 ;;
esac
EOF
# the fake ssh: options skipped, the host ignored, the command logged, then
# run by sh with the board's paths rewritten into the scratch tree (dd of=
# a block device would not truncate: notrunc keeps the file's tail)
cat > "$S/fakebin/ssh" <<EOF
#!/bin/bash
F="$F"
opts=
while [ \$# -gt 0 ]; do case "\$1" in -o) opts="\$opts \$2"; shift 2 ;; -*) shift ;; *) break ;; esac; done
shift   # the host
cmd="\$*"
printf '%s\n' "\$cmd" >> "\$F/ssh.log"
printf '%s\n' "\$cmd" >> "\$F/ssh.all.log"
printf '%s\n' "\${opts# }" >> "\$F/ssh.opts"
[ -e "\$F/state/dead" ] && exit 255
# state/fwfail: a read of one variable fails as an ssh that drops would
[ -e "\$F/state/fwfail" ] && case "\$cmd" in *"fw_printenv -n"*) exit 255 ;; esac
# state/slowdd: a write of the partition takes 3 s (a signal arrives in it)
[ -e "\$F/state/slowdd" ] && case "\$cmd" in *"dd of="*) sleep 3 ;; esac
cmd=\$(printf '%s' "\$cmd" | sed -E -e "s#/boot(/|[ );']|\$)#\$F/boot\\1#g" -e "s#/dev/mmcblk1p7#\$F/dev/mmcblk1p7#g" \\
    -e "s#/proc/uptime#\$F/proc/uptime#g" -e "s#/sys/block/#\$F/sys/block/#g" -e "s#/proc/sys/vm/drop_caches#\$F/proc/drop_caches#g" -e "s#conv=fsync#conv=fsync,notrunc#g")
PATH="\$F/bin:\$PATH" exec sh -c "\$cmd"
EOF
printf '#!/bin/sh\nexit 0\n' > "$S/fakebin/stty"
chmod +x "$F"/bin/* "$S"/fakebin/*
# (the fake ssh ignores the host; a bare name, not login@host: a shape the
# release redaction scan does not count, as the fixtures above)
live() { PATH="$S/fakebin:$PATH" BOARD_HOST=$FAKE_HOST BOARD_UART=$F/console.txt LOCK=$S/board.lock LOCK_WAIT=0 MP2_BACKUP_DIR=$S/backup RUN_BOUND=${RUN_BOUND:-1} RETURN_BOUND=${RETURN_BOUND:-1} "$B" "$@"; }
# live_bg: the same as the runner's own process (exec), so a signal reaches it
live_bg() { PATH="$S/fakebin:$PATH" BOARD_HOST=$FAKE_HOST BOARD_UART=$F/console.txt LOCK=$S/board.lock LOCK_WAIT=0 MP2_BACKUP_DIR=$S/backup RUN_BOUND=${RUN_BOUND:-1} RETURN_BOUND=${RETURN_BOUND:-1} exec "$B" "$@"; }
LEFTF=$S/board.mp2-left   # the state file beside the lock
# wait_for FILE PATTERN SECONDS: until FILE has a line matching PATTERN
wait_for() { local i=0; while [ "$i" -lt $(($3 * 5)) ]; do grep -q "$2" "$1" 2>/dev/null && return 0; sleep 0.2; i=$((i + 1)); done; return 1; }
# person_bootcmd VALUE: what a person does at U-Boot's prompt (setenv bootcmd; saveenv), never the runner
person_bootcmd() { ENVT set "$PART" "$PART.p" "bootcmd=$1" >/dev/null && mv "$PART.p" "$PART"; }
leaks_live() { leaks "$1" || grep -q "$F/console.txt" <<<"$1"; }
PART="$F/dev/mmcblk1p7"

out=$(live install 2>&1)
rc=$?
check "live install (G4): rc 7, NEEDS A PERSON naming the line for U-Boot's prompt, the UNDO printed, no host or console path in the output" \
    eval '[ "$rc" = 7 ] && grep -q "NEEDS A PERSON: U-Boot runs the one-shot only if bootcmd begins with" <<<"$out" &&
          grep -qF "setenv bootcmd '"'"'run yako_hook; run bootcmd_stm32mp'"'"'; saveenv" <<<"$out" && grep -q "UNDO, should anything" <<<"$out" && ! leaks_live "$out"'
check "live install (G4): bootcmd is NOT written; the hook's variables are (yako_part 8, yako_addr 0xa0000000, yako_bootcmd_prev), both copies check" \
    eval '[ "$(vals "$PART" bootcmd yako_bootcmd_prev yako_part yako_addr | paste -sd"|" -)" = "run bootcmd_stm32mp|run bootcmd_stm32mp|8|0xa0000000" ] && ENVT check "$PART" >/dev/null'
check "live install: the pristine image is kept on the host (mode 600) and on the card, byte for byte" \
    eval 'p=$(ls "$S"/backup/*-u-boot-env.bin 2>/dev/null | head -1); [ -n "$p" ] && cmp -s "$p" "$S/env.bin" && [ "$(stat -c %a "$p")" = 600 ] && cmp -s "$F/boot/yakogl/backup/u-boot-env.bin" "$S/env.bin"'
check "live install: boot-sha256.txt lists the five stock files and checks" \
    eval '[ "$(wc -l < "$F/boot/yakogl/backup/boot-sha256.txt")" = 5 ] && (cd "$F/boot" && sha256sum -c --quiet yakogl/backup/boot-sha256.txt)'
check "live install: the bytes outside the two copies of the partition are untouched" \
    eval 'cmp -s <(head -c $((0x7c000)) "$S/env.bin") <(head -c $((0x7c000)) "$PART")'
: > "$F/ssh.log"
out=$(live run "$S/img/mp2.bin" "$S/live-logs" nohook 2>&1)
rc=$?
check "live run before the person's bootcmd edit (G4): rc 7, NEEDS A PERSON, nothing staged, no reboot" \
    eval '[ "$rc" = 7 ] && grep -q "NEEDS A PERSON" <<<"$out" && ! grep -q "cat > /boot" "$F/ssh.log" && ! grep -q "systemctl reboot" "$F/ssh.log" && ! grep -q "dd of=" "$F/ssh.log"'
out=$(live status 2>&1)
check "live status before the person's edit: 'install: needs a person' and the line to type" \
    eval 'grep -q "^install: needs a person: bootcmd does not begin with" <<<"$out" && grep -qF "a person types at U-Boot'"'"'s prompt: setenv bootcmd" <<<"$out"'
person_bootcmd "run yako_hook; run bootcmd_stm32mp"
out=$(live install 2>&1)
check "live install after the person's edit: 'installed already, byte for byte; nothing to do', no second pristine image" \
    eval 'grep -q "installed already, byte for byte" <<<"$out" && [ "$(ls "$S"/backup/*-u-boot-env.bin | wc -l)" = 1 ]'
out=$(live status 2>&1)
rc=$?
check "live status: rc 0, 'install: ok', the env and bootfs devices on the SD card, 'stock boot files: unchanged since install', nothing left" \
    eval '[ "$rc" = 0 ] && grep -q "^install: ok" <<<"$out" && grep -q "^env partition: /dev/mmcblk1p7 .*bootfs: /dev/mmcblk1p8 (U-Boot part 8, card /dev/mmcblk1)" <<<"$out" &&
          grep -q "stock boot files: unchanged since install" <<<"$out" && grep -q "^left behind: nothing recorded" <<<"$out"'

: > "$F/ssh.log"
: > "$F/ssh.opts"
out=$(live run "$S/img/mp2.bin" "$S/live-logs" trial 2>&1)
rc=$?
rd=$(ls -d "$S"/live-logs/*-trial-* 2>/dev/null | head -1)
trial_out=$out
trial_rd=$rd
check "live run (comes back): rc 0 and the verdict line byte for byte but the seconds: done, row, complete, pass:mp2_identify" \
    eval '[ "$rc" = 0 ] && grep -qE "^board-stm32mp2.sh stm32mp257f-dk trial rc=0 verdict=done boot=uboot-sd row boot_verdict=complete result=pass:mp2_identify back_after=[0-9]+s$" <<<"$out"'
check "live run: the verdict line has board.sh's shape (<runner> <board> <name> rc= verdict= boot= row boot_verdict= result= back_after=)" \
    eval 'grep -qE "^board-stm32mp2.sh stm32mp257f-dk [A-Za-z0-9_-]+ rc=[0-9]+ verdict=[a-z_]+ boot=uboot-sd (row|blind) boot_verdict=[a-z_]+ result=(pass|fail):[A-Za-z0-9_]+ back_after=([0-9]+s|absent)$" <<<"$out"'
check "live run: the record (meta.txt, console.txt, INDEX.txt), the console redacted, the index names the verdict" \
    eval '[ -n "$rd" ] && [ -s "$rd/meta.txt" ] && [ -s "$rd/console.txt" ] && [ -s "$rd/INDEX.txt" ] && grep -q "^verdict: board-stm32mp2.sh stm32mp257f-dk trial rc=0 verdict=done" "$rd/INDEX.txt" &&
          grep -q "mp2_identify: PASS" "$rd/console.txt" && ! grep -rq "mp2board\|$F/console.txt" "$rd"'
check "live run: the one-shot was armed with the staged file, cleared by the boot, the staged image removed, the hook still installed" \
    eval 'grep -q "^token=trial-" "$rd/meta.txt" && grep -q "fw_printenv yako_once yako_file" "$F/ssh.log" && [ "$(vals "$PART" yako_once | head -1)" = - ] && [ -z "$(ls "$F"/boot/yakogl/once-*.bin 2>/dev/null)" ] && [ "$(vals "$PART" bootcmd)" = "run yako_hook; run bootcmd_stm32mp" ] && grep -q "^staged=.*read back equal" "$rd/meta.txt"'
check "live run: one reboot, after the arm's write and its read-back (dd of=, sha256sum, fw_printenv yako_once yako_file)" \
    eval '[ "$(grep -c "systemctl reboot" "$F/ssh.log")" = 1 ] && o=$(grep -nE "dd of=/dev/mmcblk1p7|dd if=/dev/mmcblk1p7 bs=64k status=none \| sha256sum|fw_printenv yako_once yako_file|systemctl reboot" "$F/ssh.log" | cut -d: -f1 | paste -sd" " -);
          [ "$(wc -w <<<"$o")" = 4 ] && [ "$(tr " " "\n" <<<"$o" | sort -n | paste -sd" " -)" = "$o" ]'
check "live run: the wait is written in the record (wait_bound=2) and back_after is at least the 5 s poll" \
    eval 'grep -qx "wait_bound=2" "$rd/meta.txt" && [ "$(( $(sed -n "s/^t_back=//p" "$rd/meta.txt") - $(sed -n "s/^t_reboot=//p" "$rd/meta.txt") ))" -ge 5 ]'
check "live run (G7): every ssh checks the host key and records none (StrictHostKeyChecking=yes, UpdateHostKeys=no), never StrictHostKeyChecking=no" \
    eval '[ -s "$F/ssh.opts" ] && ! grep -v "StrictHostKeyChecking=yes" "$F/ssh.opts" | grep -q . && ! grep -v "UpdateHostKeys=no" "$F/ssh.opts" | grep -q . && ! grep -q "StrictHostKeyChecking=no" "$F/ssh.opts"'
check "live run: the state file was written ahead of the changes and cleared at the end; the record says what was planned (left=...)" \
    eval '[ ! -e "$LEFTF" ] && grep -q "^left=armed yako_once=trial-" "$rd/meta.txt" && grep -q "^left=rebooting with yako_once=trial-" "$rd/meta.txt"'
: > "$F/ssh.opts"
out=$(MP2_KNOWN_HOSTS=$S/known_hosts.board live status 2>&1)
check "live status with MP2_KNOWN_HOSTS (G7): ssh reads that file only (UserKnownHostsFile, no global file), still strict" \
    eval '! grep -v "UserKnownHostsFile=$S/known_hosts.board" "$F/ssh.opts" | grep -q . && ! grep -v "GlobalKnownHostsFile=/dev/null" "$F/ssh.opts" | grep -q . && ! grep -v "StrictHostKeyChecking=yes" "$F/ssh.opts" | grep -q . && [ ! -e "$S/known_hosts.board" ]'

# the hold: the same run with MP2_HOLD_S=3 prints the hold line when the PASS line shows, waits, and the verdict line is the same
: > "$F/ssh.log"
out=$(MP2_HOLD_S=3 live run "$S/img/mp2.bin" "$S/live-logs" held 2>&1)
rc=$?
rd=$(ls -d "$S"/live-logs/*-held-* 2>/dev/null | head -1)
check "live run with MP2_HOLD_S=3: rc 0, one 'hold: result seen at <UTC ISO>, holding 3s' line before the verdict, the verdict line the same as without" \
    eval '[ "$rc" = 0 ] && [ "$(grep -c "^hold: " <<<"$out")" = 1 ] && grep -qE "^hold: result seen at [0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z, holding 3s$" <<<"$out" &&
          [ "$(grep -n "^hold: " <<<"$out" | cut -d: -f1)" -lt "$(grep -n "verdict=done" <<<"$out" | head -1 | cut -d: -f1)" ] &&
          grep -qE "^board-stm32mp2.sh stm32mp257f-dk held rc=0 verdict=done boot=uboot-sd row boot_verdict=complete result=pass:mp2_identify back_after=[0-9]+s$" <<<"$out"'
check "live run with MP2_HOLD_S=3: the record has hold_s=3 and wait_bound=5 (the bound extended by the hold), back_after covers the hold" \
    eval 'grep -qx "hold_s=3" "$rd/meta.txt" && grep -qx "wait_bound=5" "$rd/meta.txt" && [ "$(( $(sed -n "s/^t_back=//p" "$rd/meta.txt") - $(sed -n "s/^t_reboot=//p" "$rd/meta.txt") ))" -ge 8 ]'
check "live run without MP2_HOLD_S (the first run): no hold line, no hold_s in the record" \
    eval '! grep -q "^hold:" <<<"$trial_out" && ! grep -q "^hold_s=" "$trial_rd/meta.txt"'

# a run that hangs on purpose: the console has the hang line and no PSCI line
cp "$F/console.txt" "$F/console.pass"
printf '%s\n%s\n%s\n' "$UBOOT" "$LAYER" "$HANG" | sed 's/^[0-9:]\{8\} //' > "$F/console.txt"
: > "$F/ssh.log"
out=$(WDOG_BOUND=2 live run --expect-watchdog "$S/img/mp2.bin" "$S/live-logs" wdog 2>&1)
rc=$?
rd=$(ls -d "$S"/live-logs/*-wdog-* 2>/dev/null | head -1)
check "live run --expect-watchdog (comes back, hang line, no PSCI line): rc 0 and verdict=watchdog_reset, boot_verdict=hung" \
    eval '[ "$rc" = 0 ] && grep -qE "^board-stm32mp2.sh stm32mp257f-dk wdog rc=0 verdict=watchdog_reset boot=uboot-sd row boot_verdict=hung result=none back_after=[0-9]+s$" <<<"$out"'
check "live run --expect-watchdog without MP2_HOLD_S: no hold line" eval '! grep -q "^hold:" <<<"$out"'
check "live run --expect-watchdog: the record has expect_wdog=1 and wait_bound=2, the index shows the hang line, one reboot, the staged image removed" \
    eval 'grep -qx "expect_wdog=1" "$rd/meta.txt" && grep -qx "wait_bound=2" "$rd/meta.txt" && grep -q "mp2_hello: hanging on purpose" "$rd/INDEX.txt" &&
          [ "$(grep -c "systemctl reboot" "$F/ssh.log")" = 1 ] && [ -z "$(ls "$F"/boot/yakogl/once-*.bin 2>/dev/null)" ]'
cp "$F/console.pass" "$F/console.txt"

# G10: the reboot command did not reboot: the old boot goes on answering
echo noreboot > "$F/state/mode"
printf '100000.00 300.00\n' > "$F/proc/uptime"
: > "$F/ssh.log"
out=$(live run "$S/img/mp2.bin" "$S/live-logs" norb 2>&1)
rc=$?
rd=$(ls -d "$S"/live-logs/*-norb-* 2>/dev/null | head -1)
check "live run whose reboot did not happen (G10): rc 4, verdict not_rebooted, no NEEDS A PERSON" \
    eval '[ "$rc" = 4 ] && grep -qE "^board-stm32mp2.sh stm32mp257f-dk norb rc=4 verdict=not_rebooted boot=uboot-sd row .* back_after=absent$" <<<"$out" && ! grep -q "NEEDS A PERSON" <<<"$out"'
check "live run whose reboot did not happen (G10): disarmed (the pre-arm environment back: no yako_once, yako_file not this run's), the staged image removed, rebooted=0 in the record, no state left" \
    eval '[ "$(vals "$PART" yako_once | head -1)" = - ] && ! vals "$PART" yako_file | grep -q norb &&[ -z "$(ls "$F"/boot/yakogl/once-* 2>/dev/null)" ] &&
          grep -qx "rebooted=0" "$rd/meta.txt" && grep -qx "disarmed=not rebooted" "$rd/meta.txt" && [ ! -e "$LEFTF" ] && [ "$(grep -c "systemctl reboot" "$F/ssh.log")" = 1 ]'
: > "$F/ssh.log"
out=$(MP2_REBOOT_GRACE=3 RUN_BOUND=100 RETURN_BOUND=100 live run "$S/img/mp2.bin" "$S/live-logs" norbg 2>&1)
rc=$?
check "live run whose reboot did not happen, MP2_REBOOT_GRACE=3 inside a 200 s bound: not_rebooted at the first poll after the grace, disarmed" \
    eval '[ "$rc" = 4 ] && grep -q " verdict=not_rebooted " <<<"$out" && [ "$(vals "$PART" yako_once | head -1)" = - ] && [ "$(grep -c "cut -d. -f1 /proc/uptime" "$F/ssh.log")" = 1 ]'

# the one-shot not cleared by the boot: the cycle's own disarm, rc 6, and a
# record, not an exit without a verdict
echo noclear > "$F/state/mode"
out=$(live run "$S/img/mp2.bin" "$S/live-logs" noclr 2>&1)
rc=$?
rd=$(ls -d "$S"/live-logs/*-noclr-* 2>/dev/null | head -1)
check "live run whose boot left yako_once set: rc 6, verdict done, the one-shot disarmed by the runner, recorded, nothing left" \
    eval '[ "$rc" = 6 ] && grep -q " rc=6 verdict=done " <<<"$out" && [ "$(vals "$PART" yako_once yako_file | paste -sd"|" -)" = "-|-" ] &&
          grep -qx "disarmed=after the cycle" "$rd/meta.txt" && [ ! -e "$LEFTF" ]'
rm -f "$F/state/mode"

# G11: the lock's watchdog sends TERM while the reboot's ssh is in the
# foreground (state/mode hang: it holds the session 20 s); the run must end
# at once through its undo, well inside the watchdog's 10 s before KILL
echo hang > "$F/state/mode"
printf '100.00 300.00\n' > "$F/proc/uptime"
: > "$F/ssh.log"
t0=$(date +%s)
HOLD_MAX=12 live_bg run "$S/img/mp2.bin" "$S/live-logs" wdkill > "$S/wdkill.out" 2>&1 &
pid=$!
wait "$pid"
rc=$?
dt=$(($(date +%s) - t0))
out=$(cat "$S/wdkill.out")
check "G11: the lock's watchdog TERM at HOLD_MAX=12 during the reboot's ssh: rc 143 within 20 s (not after the 20 s ssh), the watchdog's lines shown" \
    eval '[ "$rc" = 143 ] && [ "$dt" -lt 20 ] && grep -q "SIGTERM: ending through the undo" <<<"$out" && grep -q "held the board past 12s" <<<"$out"'
check "G11: the undo ran: the pre-arm environment back (no yako_once, yako_file not this run's), the staged image removed, the state file cleared, the lock free" \
    eval '[ "$(vals "$PART" yako_once | head -1)" = - ] && ! vals "$PART" yako_file | grep -q wdkill &&[ -z "$(ls "$F"/boot/yakogl/once-* 2>/dev/null)" ] && [ ! -e "$LEFTF" ] &&
          grep -q "writing the pre-arm environment back" <<<"$out" && ! [ -s "$S/board.lock" ]'
# a SIGKILL runs no trap: the state file written ahead says what is left,
# the next run refuses and shows it, disarm applies the undo and clears it
: > "$F/ssh.log"
live_bg run "$S/img/mp2.bin" "$S/live-logs" killed > "$S/killed.out" 2>&1 &
pid=$!
wait_for "$F/ssh.log" "systemctl reboot" 30
sleep 1
kill -KILL "$pid"
wait "$pid" 2>/dev/null
check "G11: SIGKILL after the arm: the state file names the armed one-shot and the staged image (no address in it)" \
    eval '[ -s "$LEFTF" ] && grep -q "^left=rebooting with yako_once=killed-" "$LEFTF" && grep -q "^left=/boot/yakogl/once-killed-.*\.bin on the card" "$LEFTF" && ! leaks "$(cat "$LEFTF")"'
rm -f "$F/state/mode"
sleep 2   # the lock's watchdog ends within a second of its run
out=$(live run "$S/img/mp2.bin" "$S/live-logs" after 2>&1)
rc=$?
check "the next run after a SIGKILL: rc 3, refuses (a one-shot pending), shows the earlier run's record" \
    eval '[ "$rc" = 3 ] && grep -q "a one-shot is already pending" <<<"$out" && grep -q "an earlier run left this record" <<<"$out" && [ -s "$LEFTF" ]'
mkdir -p "$F/boot/yakogl" && printf 'cut\n' > "$F/boot/yakogl/once-cut-0-1.bin.part"
out=$(live disarm 2>&1)
rc=$?
check "disarm after a SIGKILL: rc 0, the record shown, yako_once and yako_file gone, the staged image and a cut-off .part removed, the state file cleared" \
    eval '[ "$rc" = 0 ] && grep -q "an earlier run left this record" <<<"$out" && [ "$(vals "$PART" yako_once yako_file | paste -sd"|" -)" = "-|-" ] &&
          [ -z "$(ls "$F"/boot/yakogl/once-* 2>/dev/null)" ] && [ ! -e "$LEFTF" ]'
out=$(live disarm 2>&1)
rc=$?
check "disarm again: rc 0, nothing written (idempotent)" eval '[ "$rc" = 0 ] && grep -q "disarmed: no one-shot pending" <<<"$out" && ! grep -q "disarming:" <<<"$out"'
printf 'cut\n' > "$F/boot/yakogl/once-cut-0-1.bin.part"
out=$(live run "$S/img/mp2.bin" "$S/live-logs" part 2>&1)
rc=$?
check "a run with a cut-off copy (once-*.bin.part) on the card: rc 3, refused before any change, names disarm" \
    eval '[ "$rc" = 3 ] && grep -q "or a cut-off copy" <<<"$out" && grep -q "disarm" <<<"$out"'
rm -f "$F/boot/yakogl/once-cut-0-1.bin.part"
touch "$F/state/fwfail"
out=$(live disarm 2>&1)
rc=$?
check "disarm when the board's variable read fails: rc 5, 'not taken as unset', not 'disarmed'" \
    eval '[ "$rc" = 5 ] && grep -q "cannot read yako_once from the board .*not taken as unset" <<<"$out" && ! grep -q "disarmed:" <<<"$out"'
rm -f "$F/state/fwfail"
# a TERM while an environment write is in flight: the write finishes (never
# cut in half), then the script ends
ENVT set "$PART" "$PART.a" yako_once=cut-1 yako_file=/yakogl/once-cut-1.bin >/dev/null && mv "$PART.a" "$PART"
touch "$F/state/slowdd"
: > "$F/ssh.log"
live_bg disarm > "$S/slowdd.out" 2>&1 &
pid=$!
wait_for "$F/ssh.log" "dd of=" 30
sleep 1
kill -TERM "$pid"
wait "$pid"
rc=$?
rm -f "$F/state/slowdd"
out=$(cat "$S/slowdd.out")
check "G11: a TERM during an environment write: rc 143 after the write finished (both copies check, the one-shot gone), not cut in half" \
    eval '[ "$rc" = 143 ] && grep -q "waiting for the environment write in flight" <<<"$out" && ENVT check "$PART" >/dev/null && [ "$(vals "$PART" yako_once | head -1)" = - ]'

echo die > "$F/state/mode"
: > "$F/ssh.log"
out=$(live run "$S/img/mp2.bin" "$S/live-logs" gone 2>&1)
rc=$?
rd=$(ls -d "$S"/live-logs/*-gone-* 2>/dev/null | head -1)
check "live run (never answers again): rc 5, verdict not_back, back_after absent" \
    eval '[ "$rc" = 5 ] && grep -qE "^board-stm32mp2.sh stm32mp257f-dk gone rc=5 verdict=not_back boot=uboot-sd row boot_verdict=complete result=pass:mp2_identify back_after=absent$" <<<"$out"'
check "live run (never answers again): the NEEDS A PERSON line after the verdict, naming the wait, no retry and no outside reset" \
    eval 'v=$(grep -n "verdict=not_back" <<<"$out" | head -1 | cut -d: -f1); p=$(grep -n "NEEDS A PERSON" <<<"$out" | cut -d: -f1);
          [ -n "$v" ] && [ -n "$p" ] && [ "$p" -gt "$v" ] && grep -q "within 2 s after the reboot" <<<"$out" && grep -q "never retries" <<<"$out" && grep -q "Nothing resets it from outside" <<<"$out"'
check "live run (never answers again): exactly one reboot, no command after the board went silent, the staged image left on the card" \
    eval '[ "$(grep -c "systemctl reboot" "$F/ssh.log")" = 1 ] && [ "$(tail -c 2000 "$F/ssh.log" | grep -c "cut -d. -f1 /proc/uptime")" -ge 1 ] &&
          [ -n "$(ls "$F"/boot/yakogl/once-gone-*.bin 2>/dev/null)" ] && grep -q "^back=0" "$rd/meta.txt" && grep -q "^cleanup=not done: Linux not back" "$rd/meta.txt"'
check "live run (never answers again): the state file names the staged image and the reboot, and the undo ('disarm')" \
    eval 'grep -q "^left=/boot/yakogl/once-gone-.*on the card" "$LEFTF" && grep -q "^left=the board was rebooted and is not back" "$LEFTF" && grep -q "the undo, once the board answers: tools/board-stm32mp2.sh disarm" <<<"$out"'
rm -f "$F/state/dead" "$F/state/mode"
printf '1.00 1.00\n' > "$F/proc/uptime"
out=$(live disarm 2>&1)
rc=$?
check "live disarm (after the board is back by hand): rc 0, yako_once and yako_file gone, the staged image removed, the state file cleared" \
    eval '[ "$rc" = 0 ] && grep -q "disarmed: no one-shot pending" <<<"$out" && [ "$(vals "$PART" yako_once yako_file | paste -sd"|" -)" = "-|-" ] && [ -z "$(ls "$F"/boot/yakogl/once-*.bin 2>/dev/null)" ] && [ ! -e "$LEFTF" ]'

printf '/dev/mmcblk0p8 ext4\n' > "$F/state/findmnt"
: > "$F/ssh.log"
out=$(live run "$S/img/mp2.bin" "$S/live-logs" emmc 2>&1)
rc=$?
check "live run with /boot on the eMMC: rc 3, REFUSED naming mmcblk0, no dd of=, no staging, no reboot (the record holds no t_reboot)" \
    eval '[ "$rc" = 3 ] && grep -q "REFUSED: /boot is on /dev/mmcblk0p8 of the eMMC (mmcblk0)" <<<"$out" && ! grep -q "dd of=" "$F/ssh.log" && ! grep -q "cat > /boot" "$F/ssh.log" &&
          ! grep -q "systemctl reboot" "$F/ssh.log" && ! grep -q "^t_reboot=" "$S"/live-logs/*-emmc-*/meta.txt'
out=$(live install 2>&1)
rc=$?
check "live install with /boot on the eMMC: rc 3, no write" eval '[ "$rc" = 3 ] && ! grep -q "dd of=" "$F/ssh.log"'
printf '/dev/mmcblk2p8 ext4\n' > "$F/state/findmnt"
sed 's/mmcblk1/mmcblk2/' <<<"$CARD_TABLE" > "$F/state/lsblk"
out=$(live run "$S/img/mp2.bin" "$S/live-logs" emmc2 2>&1)
rc=$?
check "live run with /boot on an eMMC numbered mmcblk2 (sysfs type MMC, the card's labels) (G1): rc 3, REFUSED 'not SD', no write, no reboot" \
    eval '[ "$rc" = 3 ] && grep -q "REFUSED: /boot is on /dev/mmcblk2, whose /sys/block/mmcblk2/device/type is .MMC., not SD" <<<"$out" && ! grep -q "dd of=" "$F/ssh.log" && ! grep -q "systemctl reboot" "$F/ssh.log"'
out=$(live install 2>&1)
rc=$?
check "live install with /boot on an eMMC numbered mmcblk2 (G1): rc 3, no write" eval '[ "$rc" = 3 ] && grep -q "not SD" <<<"$out" && ! grep -q "dd of=" "$F/ssh.log"'
printf '/dev/mmcblk1p8 ext4\n' > "$F/state/findmnt"
grep -v u-boot-env <<<"$CARD_TABLE" > "$F/state/lsblk"
out=$(live run "$S/img/mp2.bin" "$S/live-logs" nolabel 2>&1)
rc=$?
check "live run when the card has no u-boot-env label: rc 3, no write" eval '[ "$rc" = 3 ] && grep -q "no partition labelled u-boot-env" <<<"$out" && ! grep -q "dd of=" "$F/ssh.log"'
printf '%s\n' "$CARD_TABLE" > "$F/state/lsblk"

out=$(live uninstall 2>&1)
rc=$?
check "live uninstall while bootcmd begins with the hook (G4): rc 7, NEEDS A PERSON with the line to type, the UNDO printed first, bootcmd and the hook kept, /boot/yakogl kept" \
    eval '[ "$rc" = 7 ] && grep -q "NEEDS A PERSON: bootcmd still begins with" <<<"$out" && grep -qF "setenv bootcmd '"'"'run bootcmd_stm32mp'"'"'; saveenv" <<<"$out" &&
          grep -q "UNDO, should the uninstall go wrong" <<<"$out" && [ "$(vals "$PART" bootcmd)" = "run yako_hook; run bootcmd_stm32mp" ] && [ "$(vals "$PART" yako_hook | cut -c1-7)" = "if test" ] && [ -d "$F/boot/yakogl/backup" ]'
person_bootcmd "run bootcmd_stm32mp"
printf 'changed Image.gz\n' > "$F/boot/Image.gz"
out=$(live uninstall 2>&1)
rc=$?
check "live uninstall when a stock boot file changed since install: rc 5, named, /boot/yakogl and its record kept for a person" \
    eval '[ "$rc" = 5 ] && grep -q "the stock boot files differ from install.s record" <<<"$out" && [ -s "$F/boot/yakogl/backup/boot-sha256.txt" ]'
printf 'stock Image.gz\n' > "$F/boot/Image.gz"
out=$(live uninstall 2>&1)
rc=$?
check "live uninstall after the person put bootcmd back: rc 0, 'unhooked', the stock files checked, /boot/yakogl removed, the image before the uninstall kept" \
    eval '[ "$rc" = 0 ] && grep -q "unhooked: no yako_\* variable, bootcmd .run bootcmd_stm32mp. (not written)" <<<"$out" && grep -q "stock boot files: unchanged since install" <<<"$out" &&
          [ ! -e "$F/boot/yakogl" ] && [ -n "$(ls "$S"/backup/*-u-boot-env.before-uninstall.bin 2>/dev/null)" ]'
check "live uninstall: the environment's variables are the input's (no yako_*, bootcmd as it was), both copies check" \
    eval '[ -z "$(ENVT diff "$S/env.bin" "$PART")" ] && ENVT check "$PART" >/dev/null'
manifest > "$S/boot-after.txt"
same "live uninstall: the stock boot files are byte for byte what they were before install (boot.scr.uimg, the extlinux files, Image.gz, the dtb)" "$S/boot-before.txt" "$S/boot-after.txt"
check "live: the whole command log never names mmcblk0, a module, tailscale, sshd, a partition tool or fw_setenv" \
    eval '[ -s "$F/ssh.all.log" ] && ! grep -qE "mmcblk0|modprobe|insmod|rmmod|tailscale|sshd|sfdisk|fdisk|parted|mkfs|fw_setenv" "$F/ssh.all.log"'
check "live: every dd that writes names the u-boot-env partition and the copies' sectors alone (seek=992), every rm is under /boot/yakogl" \
    eval '[ "$(grep -c "dd of=" "$F/ssh.all.log")" -ge 4 ] && ! grep "dd of=" "$F/ssh.all.log" | grep -vq "dd of=/dev/mmcblk1p7 bs=512 seek=992 conv=fsync status=none" &&
          ! grep -E "(^| )rm " "$F/ssh.all.log" | grep -vqE "rm -(f|rf) /boot/yakogl"'
check "live: nothing of the fake board or its host leaked into the host's record directory names or the outputs" \
    eval '! grep -rq "mp2board" "$S/live-logs" && ! ls "$S/live-logs" | grep -q "mp2board"'

echo "board_stm32mp2_selftest: $n checks, $fails failed (pinned: $WANT_CHECKS)"
[ "$fails" = 0 ] && [ "$n" = "$WANT_CHECKS" ]

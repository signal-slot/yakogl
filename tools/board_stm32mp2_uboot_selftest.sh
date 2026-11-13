#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# board_stm32mp2_uboot_selftest.sh - tools/board-stm32mp2-uboot.sh without a
# board (the DK's typed U-Boot one-shot runner).
#
#   tools/board_stm32mp2_uboot_selftest.sh
#
# 1. Every verb under --dry-run, both backends, with ssh, scp, curl, stty,
#    flock and sudo replaced by stubs that record any call: rc 0, the plan as
#    "DRY ..." lines (the fw_printenv and /boot sha256 records before and
#    after, the staging, the typed line, the reboot, the diffs, the verdict
#    shape), the UNDO before the staging, no stub run, nothing written on the
#    host, no host name, address or console path in the output.
# 2. The refusals that need no board: an image of 255 MiB or more (refused
#    before anything else, live too: not one ssh), 255 MiB less one byte
#    accepted, no AArch64 header, no watchdog marker (and MP2_NO_WATCHDOG=1),
#    a bad name, no BOARD_HOST, no BOARD_UART, a bad BOARD_BOOT, uboot-tftp
#    without its server, an unknown verb.
# 3. The runner's text: no environment write, no partition write, no module,
#    tailscale or sshd command, no distro-loop file name; the typed lines set
#    only mp2z_* (and autoload on the net route), load the script to
#    ${scriptaddr}, keep the 255 MiB bound (mmc) and end in "run bootcmd".
# 4. A fake board (ssh runs the command in a scratch tree; fw_printenv prints
#    a file; the reboot starts a new "uptime") and a fake console helper (it
#    records the typed line and plays a boot): a run that comes back (the
#    verdict line, the empty diffs, the staged files gone, the record
#    redacted), the TFTP route, a changed environment and a changed stock file
#    (rc 6), a refusal on the console (rc 4), a reboot that did not happen
#    (not_rebooted), a board that never comes back (rc 7, NEEDS A PERSON, one
#    reboot, the state file kept, then disarm leaves nothing), a run cut off by
#    TERM (its exit removes the staged files) and by SIGKILL (disarm removes
#    them and the record), and the live refusals: a bad CRC, scriptaddr unset
#    or in the window, /boot on the eMMC, files of an earlier run, a busy
#    console. The command log never writes the environment or a device.
# Prints "board_stm32mp2_uboot_selftest: N checks, F failed"; exit 0 only when
# F is 0 and N is the pinned count.
set -uo pipefail
export LC_ALL=C
HERE=$(cd "$(dirname "$0")" && pwd)
B="$HERE/board-stm32mp2-uboot.sh"
WANT_CHECKS=68

S=$(mktemp -d)
trap 'pkill -f "$S/" 2>/dev/null; rm -rf "$S"' EXIT
n=0
fails=0
out=
check() { # check DESCRIPTION COMMAND...
    local what=$1
    shift
    n=$((n + 1))
    if "$@"; then
        echo "  PASS  $what"
    else
        echo "  FAIL  $what"
        fails=$((fails + 1))
        [ "${MP2U_SELFTEST_DEBUG:-0}" = 1 ] && [ -n "${out:-}" ] && sed 's/^/        | /' <<<"$out" | head -60
    fi
}
unset BOARD_HOST BOARD_UART BOARD_BOOT NETBOOT_SERVER NETBOOT_TFTP_ROOT MP2_SUDO MP2_KNOWN_HOSTS MP2_NO_WATCHDOG \
    RUN_BOUND RETURN_BOUND MP2_REBOOT_GRACE MP2U_POLL MP2U_CONSOLE LOCK BOARD_LOCK LOCK_WAIT HOLD_MAX

# ---- fake images ----------------------------------------------------------------
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
MK='gc_wdog_arm: armed OP-TEE arm,smc-wdt'
mkimg "$S/img/mp2_hello.bin" 4194304 "$MK"
mkimg "$S/img/nomarker.bin" 4194304 'no watchdog here'
mkimg "$S/img/noheader.bin" 4194304 "$MK" 1
mkimg "$S/img/huge.bin" 4194304 "$MK"
truncate -s $((255 * 1024 * 1024)) "$S/img/huge.bin"           # exactly 255 MiB: refused
mkimg "$S/img/edge.bin" 4194304 "$MK"
truncate -s $((255 * 1024 * 1024 - 1)) "$S/img/edge.bin"       # one byte less: accepted

# ---- stubs: any of these running is a failure of --dry-run --------------------------
mkdir -p "$S/stub"
for c in ssh scp curl stty flock sudo; do
    printf '#!/bin/sh\necho "%s $*" >> "%s/touched"\nexit 99\n' "$c" "$S" > "$S/stub/$c"
    chmod +x "$S/stub/$c"
done
dry() { PATH="$S/stub:$PATH" "$B" --dry-run "$@"; }
untouched() { [ ! -s "$S/touched" ]; }
FAKE_HOST=mp2board.example.lab
FAKE_UART=bridgehost.example.lab:40001
leaks() { grep -qE 'mp2board|bridgehost|40001|example\.lab|192\.0\.2\.77' <<<"$1"; }
line_of() { grep -n -m1 -e "$1" <<<"$out" | cut -d: -f1; }
before() { local a b; a=$(line_of "$1"); b=$(line_of "$2"); [ -n "$a" ] && [ -n "$b" ] && [ "$a" -lt "$b" ]; }

echo "board_stm32mp2_uboot_selftest"
echo "== 1. --dry-run (stubs record any board access)"
for verb in status disarm; do
    for boot in uboot-mmc uboot-tftp; do
        out=$(BOARD_HOST=$FAKE_HOST BOARD_BOOT=$boot NETBOOT_SERVER=192.0.2.77 dry "$verb" 2>&1)
        rc=$?
        check "$verb $boot: rc 0, DRY board lines, no host name or address" \
            eval '[ "$rc" = 0 ] && grep -q "^DRY board" <<<"$out" && ! leaks "$out"'
    done
done
out=$(BOARD_HOST=$FAKE_HOST dry status 2>&1)
check "status: reads fw_printenv (all), the mount of /boot, lists the staged files, prints the undo (disarm)" \
    eval 'grep -q "^DRY board\$ fw_printenv 2>&1" <<<"$out" && grep -q "findmnt -n -v -o SOURCE,FSTYPE -T /boot" <<<"$out" &&
          grep -q "ls -d /boot/yakogl/mp2z-\*" <<<"$out" && grep -q "UNDO: of anything staged: tools/board-stm32mp2-uboot.sh disarm" <<<"$out"'
mkdir -p "$S/logs"
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART dry run "$S/img/mp2_hello.bin" "$S/logs" 2>&1)
rc=$?
check "run uboot-mmc: rc 0, no stub ran, no run directory made, no leak" \
    eval '[ "$rc" = 0 ] && untouched && [ -z "$(ls -A "$S/logs")" ] && ! leaks "$out"'
check "run uboot-mmc: lock, console probe, fw_printenv and the /boot sha256 before, the staging, the reboot" \
    eval 'grep -q "board_lock.sh" <<<"$out" && grep -q "probe <BOARD_UART>" <<<"$out" && grep -q "fw_printenv 2>&1   # > .*/env-before.txt" <<<"$out" &&
          grep -q "sha256sum   # > .*/boot-sha256.txt" <<<"$out" && grep -q "cat > /boot/yakogl/mp2z-mp2_hello-.*\.bin\.part" <<<"$out" && grep -q "systemctl reboot" <<<"$out"'
check "run uboot-mmc: the UNDO lines come before the staging and before the typed line" \
    eval 'before "UNDO: board\$ rm -f /boot/yakogl/mp2z-" "cat > /boot/yakogl/mp2z-" && before "UNDO: of the typed line" "dk: typed line: " && before "UNDO: should this run be cut off" "cat > /boot/yakogl"'
check "run uboot-mmc: the script is built from driver/tools/uboot-mp2 and read back" \
    eval 'grep -q "driver/tools/uboot-mp2/build.sh" <<<"$out" && grep -q "payload is mp2-mmc.cmd byte for byte" <<<"$out"'
TL=$(sed -n 's/.*stm32mp257f-dk: typed line: //p' <<<"$out")
check "run uboot-mmc: the typed line: mp2z_* only, script to \${scriptaddr}, the 255 MiB bound, run bootcmd at the end" \
    eval '[[ $TL == "setenv mp2z_part 8; setenv mp2z_file /yakogl/mp2z-mp2_hello-"* ]] && grep -qF "itest \${filesize} -lt 0xff00000" <<<"$TL" &&
          grep -qF "load mmc \${boot_instance}:8 \${scriptaddr} /yakogl/mp2z-" <<<"$TL" && grep -qF "source \${scriptaddr}" <<<"$TL" && [[ $TL == *"; run bootcmd" ]]'
check "run uboot-mmc: the cleanup, fw_printenv and /boot after, both diffs, the verdict shape" \
    eval 'grep -q "rm -f /boot/yakogl/mp2z-" <<<"$out" && grep -q "env-after.txt" <<<"$out" && grep -q "boot-sha256.after.txt" <<<"$out" &&
          grep -q "diff <rundir>/env-before.txt <rundir>/env-after.txt   # must be empty" <<<"$out" &&
          grep -q "verdict=<v> boot=uboot-mmc row boot_verdict=<scan>" <<<"$out" && grep -q "not_back, rc 7, NEEDS A PERSON, no retry" <<<"$out"'
rm -f "$S/touched"
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART BOARD_BOOT=uboot-tftp NETBOOT_SERVER=192.0.2.77 NETBOOT_TFTP_ROOT=$S/tftp-dry dry run "$S/img/mp2_hello.bin" "$S/logs" 2>&1)
rc=$?
TL=$(sed -n 's/.*stm32mp257f-dk: typed line: //p' <<<"$out")
check "run uboot-tftp: rc 0, no stub, nothing in the TFTP root, the canary, the staging into the root" \
    eval '[ "$rc" = 0 ] && untouched && [ ! -e "$S/tftp-dry" ] && grep -q "curl -s --max-time 5 tftp://" <<<"$out" && grep -q "cp .* $S/tftp-dry/yakogl/mp2z-mp2_hello-.*\.bin" <<<"$out"'
check "run uboot-tftp: the typed line fetches the script to \${scriptaddr}, sets mp2z_* and autoload only, ends in run bootcmd; no address shown" \
    eval '[[ $TL == "setenv mp2z_server <NETBOOT_SERVER>; setenv mp2z_file yakogl/mp2z-mp2_hello-"* ]] && grep -qF "setenv autoload no; dhcp; tftpboot \${scriptaddr} <NETBOOT_SERVER>:yakogl/mp2z-" <<<"$TL" &&
          [[ $TL == *"; run bootcmd" ]] && ! leaks "$out" && before "UNDO: host\$ rm -f" "DRY host\$ cp "'
out=$(BOARD_HOST=$FAKE_HOST dry stage "$S/img/mp2_hello.bin" 2>&1)
rc=$?
check "stage: rc 0, the undo before the staging, the line a person types, no stub" \
    eval '[ "$rc" = 0 ] && untouched && before "UNDO: board" "cat > /boot/yakogl" && grep -q "a person types at ST" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART dry run "$S/img/edge.bin" "$S/logs" 2>&1); rc=$?
check "an image of 255 MiB less one byte is accepted" eval '[ "$rc" = 0 ] && grep -q "cat > /boot/yakogl" <<<"$out"'

echo "== 2. refusals without a board"
refused() { # refused RC PATTERN: the last command's rc and output
    [ "$rc" = "$1" ] && grep -q -- "$2" <<<"$out"
}
rm -f "$S/touched"
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART dry run "$S/img/huge.bin" "$S/logs" 2>&1); rc=$?
check "255 MiB: run refused (rc 3) before staging, nothing printed for the board" \
    eval 'refused 3 "255 MiB (267386880) or more" && ! grep -q "^DRY" <<<"$out" && untouched'
out=$(BOARD_HOST=$FAKE_HOST dry stage "$S/img/huge.bin" 2>&1); rc=$?
check "255 MiB: stage refused (rc 3) before staging" eval 'refused 3 "255 MiB" && ! grep -q "^DRY" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART BOARD_BOOT=uboot-tftp NETBOOT_TFTP_ROOT=$S/tftp-dry dry run "$S/img/huge.bin" "$S/logs" 2>&1); rc=$?
check "255 MiB: refused on the TFTP route too, nothing in the root" eval 'refused 3 "255 MiB" && [ ! -e "$S/tftp-dry" ]'
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART dry run "$S/img/noheader.bin" "$S/logs" 2>&1); rc=$?
check "no AArch64 header: refused" eval 'refused 3 "no AArch64 Image header"'
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART dry run "$S/img/nomarker.bin" "$S/logs" 2>&1); rc=$?
check "no watchdog marker: refused" eval 'refused 3 "does not contain"'
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART MP2_NO_WATCHDOG=1 dry run "$S/img/nomarker.bin" "$S/logs" 2>&1); rc=$?
check "no watchdog marker with MP2_NO_WATCHDOG=1: accepted with a warning" eval '[ "$rc" = 0 ] && grep -q "WARNING: MP2_NO_WATCHDOG=1" <<<"$out"'
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART dry run "$S/img/mp2_hello.bin" "$S/logs" 'bad name' 2>&1); rc=$?
check "a bad run name: rc 2" eval 'refused 2 "letters, digits"'
out=$(BOARD_UART=$FAKE_UART PATH="$S/stub:$PATH" "$B" run "$S/img/mp2_hello.bin" "$S/logs" 2>&1); rc=$?
check "no BOARD_HOST: rc 2" eval 'refused 2 "set BOARD_HOST"'
out=$(BOARD_HOST=$FAKE_HOST PATH="$S/stub:$PATH" "$B" run "$S/img/mp2_hello.bin" "$S/logs" 2>&1); rc=$?
check "no BOARD_UART: rc 2 (the typed line is the route)" eval 'refused 2 "set BOARD_UART"'
out=$(BOARD_HOST=$FAKE_HOST BOARD_BOOT=extlinux dry status 2>&1); rc=$?
check "a bad BOARD_BOOT: rc 2" eval 'refused 2 "BOARD_BOOT must be"'
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART BOARD_BOOT=uboot-tftp PATH="$S/stub:$PATH" "$B" run "$S/img/mp2_hello.bin" "$S/logs" 2>&1); rc=$?
check "uboot-tftp without NETBOOT_SERVER: rc 2" eval 'refused 2 "set NETBOOT_SERVER"'
out=$(dry frobnicate 2>&1); rc=$?
check "an unknown verb: rc 2 and the usage" eval 'refused 2 "status"'
check "no stub ran in any refusal" untouched

echo "== 3. the runner's text"
code=$(grep -v '^[[:space:]]*#' "$B")
check "no environment write anywhere in the runner (save""env, fw_set""env, env save|write|erase)" \
    eval '! grep -qE "save""env|fw_set""env|env (save|write|erase)" "$B"'
check "no device write (dd of=, mmc write, ext4write, mkfs, sgdisk, parted)" \
    eval '! grep -qE "dd of=|mmc write|ext4write|mkfs|sgdisk|parted|wipefs" "$B"'
check "no module, tailscale or sshd command" \
    eval '! grep -qE "modprobe|insmod|rmmod|tailscale|sshd" <<<"$code"'
check "no distro-loop file name (extlinux.conf, boot.scr)" eval '! grep -qE "extlinux\.conf|boot\.scr" "$B"'
check "bootcmd is only run, never set" eval '! grep -qE "setenv bootcmd|env set bootcmd" <<<"$code"'
check "the go window and the size bound are the plan's (0xa0000000..0xb0000000, 255 MiB)" \
    eval 'grep -q "^LOAD=0xa0000000" "$B" && grep -q "^LOAD_END=0xb0000000" "$B" && grep -q "^MAX_IMG=\$((255 \* 1024 \* 1024))" "$B" && [ $((0xff00000)) = $((255 * 1024 * 1024)) ]'

echo "== 4. a fake board and a fake console"
F=$S/board
mkdir -p "$F/boot/mmc0_extlinux" "$F/bin" "$F/proc/sys/vm" "$F/sys/block/mmcblk1/device" "$F/state" "$S/fakebin" "$S/tftp"
printf 'kernel\n' > "$F/boot/Image.gz"
printf 'dtb\n' > "$F/boot/stm32mp257f-dk.dtb"
printf 'label stock\n' > "$F/boot/mmc0_extlinux/stock.conf"
printf 'SD\n' > "$F/sys/block/mmcblk1/device/type"
ENV0='arch=arm
bootcmd=run bootcmd_stm32mp
bootdelay=1
ethaddr=00:80:e1:00:00:01
scriptaddr=0x90100000'
reset_board() {
    rm -rf "$F/state" "$F/boot/yakogl"
    mkdir -p "$F/state"
    printf '%s\n' "$ENV0" > "$F/env.txt"
    printf '1000.00 1000.00\n' > "$F/proc/uptime"
    printf '/dev/mmcblk1p8 ext4\n' > "$F/state/findmnt"
    : > "$F/ssh.log"
    : > "$F/typed.lines"
    rm -f "$S/board.mp2u-left"
}
cat > "$F/bin/fw_printenv" <<EOF
#!/bin/sh
[ -e "$F/state/badcrc" ] && echo "Warning: Bad CRC, using default environment"
cat "$F/env.txt"
EOF
printf '#!/bin/sh\ncat "%s/state/findmnt"\n' "$F" > "$F/bin/findmnt"
printf '#!/bin/sh\necho bootfs\n' > "$F/bin/lsblk"
printf '#!/bin/sh\necho fakeboard\n' > "$F/bin/uname"
printf '#!/bin/sh\nexit 0\n' > "$F/bin/who"
cat > "$F/bin/systemctl" <<EOF
#!/bin/sh
[ "\$1" = reboot ] || exit 1
echo reboot >> "$F/state/reboots"
case "\$(cat "$F/state/mode" 2>/dev/null)" in
die) touch "$F/state/dead" "$F/state/rebooted" ;;
noreboot) ;;
hang) sleep 30 ;;
envchange) echo "mp2z_file=left" >> "$F/env.txt"; printf '0.00 0.00\n' > "$F/proc/uptime"; touch "$F/state/rebooted" ;;
bootchange) echo changed >> "$F/boot/stm32mp257f-dk.dtb"; printf '0.00 0.00\n' > "$F/proc/uptime"; touch "$F/state/rebooted" ;;
*) printf '0.00 0.00\n' > "$F/proc/uptime"; touch "$F/state/rebooted" ;;
esac
EOF
# the fake ssh: options skipped, the host ignored, the command logged and run
# by sh with the board's paths rewritten into the scratch tree
cat > "$S/fakebin/ssh" <<EOF
#!/bin/bash
F="$F"
while [ \$# -gt 0 ]; do case "\$1" in -o) shift 2 ;; -*) shift ;; *) break ;; esac; done
shift
cmd="\$*"
printf '%s\n' "\$cmd" >> "\$F/ssh.log"
[ -e "\$F/state/dead" ] && exit 255
case "\$cmd" in *".part"*) echo "fake-ssh: staging" >&2 ;; esac
cmd=\$(printf '%s' "\$cmd" | sed -E -e "s#/boot(/|[ );']|\\\$)#\$F/boot\\1#g" -e "s#/proc/uptime#\$F/proc/uptime#g" \\
    -e "s#/sys/block/#\$F/sys/block/#g" -e "s#/proc/sys/vm/drop_caches#\$F/proc/sys/vm/drop_caches#g")
PATH="\$F/bin:\$PATH" exec sh -c "\$cmd"
EOF
printf '#!/bin/sh\necho canary\n' > "$S/fakebin/curl"
# the fake console helper: probe answers state/busy; typed records its lines,
# waits for the reboot and plays state/console (or the default boot), stamped
cat > "$S/fakebin/fakecon" <<EOF
#!/bin/bash
F="$F"
case "\$1" in
probe) [ -e "\$F/state/busy" ] && exit 4; exit 0 ;;
typed) ;;
*) exit 2 ;;
esac
raw=\$3 ts=\$4
shift 4
printf '%s\n' "\$@" >> "\$F/typed.lines"
trap 'exit 0' TERM
i=0
while [ ! -e "\$F/state/rebooted" ] && [ \$i -lt 300 ]; do sleep 0.1; i=\$((i + 1)); done
[ -e "\$F/state/rebooted" ] || exit 0
src="\$F/console.done"
[ -e "\$F/state/console" ] && src=\$(cat "\$F/state/console")
while IFS= read -r l; do
    l=\${l//@LINE@/\$1}
    printf '%s\r\n' "\$l" >> "\$raw"
    printf '%s %s\n' "\$(date +%s.%N | cut -c1-14)" "\$l" >> "\$ts"
done < "\$src"
while :; do sleep 0.2; done
EOF
BOOTLOG='NOTICE:  CPU: STM32MP257FAK Rev.Y
NOTICE:  Reset reason: Power-on Reset (PORRSTN)
NOTICE:  BL2: v2.10-stm32mp2-r1.0(release):v2.10.0
I/TC: OP-TEE version: 4.0.0-stm32mp2
U-Boot 2023.10-stm32mp2 (Jan 01 2025 - 10:00:00 +0000)
Hit any key to stop autoboot:  0
STM32MP> @LINE@'
{ printf '%s\n' "$BOOTLOG"; printf '%s\n' 'mp2z: undo: nothing persistent; run bootcmd boots ST Linux' 'mp2z: go 0xa0000000' \
    '*** Booting Zephyr OS build selftest ***' 'mp2_hello: PASS: hello from 198.51.100.99' 'mp2: lifetime: PSCI SYSTEM_RESET at 1234 ms'
  printf '%s\n' "$BOOTLOG" | head -5; printf '%s\n' 'Hit any key to stop autoboot:  0' 'Starting kernel ...'; } > "$F/console.done"
{ printf '%s\n' "$BOOTLOG"; printf '%s\n' 'mp2z: undo: nothing persistent; run bootcmd boots ST Linux' 'mp2z: refused: ext4load failed' \
    'mp2z: back to bootcmd' 'Starting kernel ...'; } > "$F/console.refused"
chmod +x "$F"/bin/* "$S"/fakebin/*
live() { PATH="$S/fakebin:$PATH" BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART MP2U_CONSOLE=$S/fakebin/fakecon LOCK=$S/board.lock LOCK_WAIT=0 \
    MP2U_POLL=1 RUN_BOUND=${RUN_BOUND:-20} RETURN_BOUND=${RETURN_BOUND:-20} "$B" "$@"; }
LEFTF=$S/board.mp2u-left
staged_on_card() { ls "$F/boot/yakogl/" 2>/dev/null | grep -q '^mp2z-'; }
log_clean() { # the board commands never write the environment or a device, nor touch modules or services
    ! grep -qE "save""env|fw_set""env|dd |mmcblk0|modprobe|insmod|rmmod|tailscale|sshd|setenv" "$F/ssh.log"
}
vline() { grep '^board-stm32mp2-uboot.sh stm32mp257f-dk ' <<<"$out" | tail -1; }

reset_board
mkdir -p "$S/runs"
out=$(live run "$S/img/mp2_hello.bin" "$S/runs" 2>&1); rc=$?
RD=$(ls -d "$S/runs"/*-mp2_hello-* 2>/dev/null | head -1)
check "mmc run: rc 0, verdict=done row, boot_verdict done, result pass, both diffs empty" \
    eval '[ "$rc" = 0 ] && [[ "$(vline)" =~ ^board-stm32mp2-uboot\.sh\ stm32mp257f-dk\ mp2_hello\ rc=0\ verdict=done\ boot=uboot-mmc\ row\ boot_verdict=done\ result=pass:mp2_hello\ env_diff=empty\ boot_diff=empty\ back_after=[0-9]+s$ ]]'
check "mmc run: the staged files are gone and the directory it made too" eval '! staged_on_card && [ ! -e "$F/boot/yakogl" ]'
check "mmc run: the typed line names the bootfs partition (8) and \${scriptaddr}, typed once" \
    eval '[ "$(grep -c . "$F/typed.lines")" = 1 ] && grep -qF "load mmc \${boot_instance}:8 \${scriptaddr} /yakogl/mp2z-mp2_hello-" "$F/typed.lines"'
check "mmc run: one reboot; the UNDO before the staging" \
    eval '[ "$(grep -c reboot "$F/state/reboots")" = 1 ] && before "UNDO: board\$ rm -f /boot/yakogl/mp2z-" "fake-ssh: staging"'
check "mmc run: the record: env before/after, /boot sha256 before/after, empty diffs, console, typed.log" \
    eval '[ -n "$RD" ] && [ -s "$RD/env-before.txt" ] && cmp -s "$RD/env-before.txt" "$RD/env-after.txt" && [ -s "$RD/boot-sha256.txt" ] &&
          cmp -s "$RD/boot-sha256.txt" "$RD/boot-sha256.after.txt" && [ ! -s "$RD/env.diff" ] && [ ! -s "$RD/boot.diff" ] && [ -s "$RD/console.ts" ] && [ -e "$RD/typed.log" ]'
check "mmc run: the record is redacted (MAC, host, address)" \
    eval 'grep -q "^ethaddr=<redacted>" "$RD/env-before.txt" && ! grep -rqE "00:80:e1|mp2board|bridgehost|198\.51\.100\.99" "$RD" && grep -q "192.0.2.1" "$RD/console.ts"'
check "mmc run: the stock files are byte for byte unchanged" eval 'grep -qx kernel "$F/boot/Image.gz" && grep -qx dtb "$F/boot/stm32mp257f-dk.dtb"'
check "mmc run: the board commands never write the environment or a device" log_clean
check "mmc run: no state file left, no leak in the output" eval '[ ! -e "$LEFTF" ] && ! leaks "$out"'
check "mmc run: the staged script is the built mp2-mmc.scr (read back by sha256 on the card)" \
    eval 'grep -q "sha256sum mp2z-mp2_hello-.*\.scr mp2z-mp2_hello-.*\.bin" "$F/ssh.log"'

reset_board
out=$(BOARD_BOOT=uboot-tftp NETBOOT_SERVER=192.0.2.77 NETBOOT_TFTP_ROOT=$S/tftp live run "$S/img/mp2_hello.bin" "$S/runs" tftp_hello 2>&1); rc=$?
check "tftp run: rc 0, verdict=done boot=uboot-tftp, both diffs empty" \
    eval '[ "$rc" = 0 ] && grep -q "rc=0 verdict=done boot=uboot-tftp row boot_verdict=done result=pass:mp2_hello env_diff=empty boot_diff=empty" <<<"$(vline)"'
check "tftp run: nothing left in the TFTP root, nothing staged on the card" eval '[ -z "$(ls -A "$S/tftp/yakogl")" ] && ! staged_on_card && ! grep -q "\.part" "$F/ssh.log"'
check "tftp run: the typed line fetches the script to \${scriptaddr} from the server; no address in the output" \
    eval 'grep -qF "tftpboot \${scriptaddr} 192.0.2.77:yakogl/mp2z-tftp_hello-" "$F/typed.lines" && ! leaks "$out"'
TR=$(ls -d "$S/runs"/*-tftp_hello-* | head -1)
check "tftp run: the server's address is redacted in the record" eval '! grep -rq "192\.0\.2\.77" "$TR"'

reset_board
echo envchange > "$F/state/mode"
out=$(live run "$S/img/mp2_hello.bin" "$S/runs" envch 2>&1); rc=$?
check "a changed environment: rc 6, verdict=env_changed, the diff printed" \
    eval '[ "$rc" = 6 ] && grep -q "verdict=env_changed .* env_diff=changed boot_diff=empty" <<<"$(vline)" && grep -q "> mp2z_file=left" <<<"$out"'
reset_board
echo bootchange > "$F/state/mode"
out=$(live run "$S/img/mp2_hello.bin" "$S/runs" bootch 2>&1); rc=$?
check "a changed stock /boot file: rc 6, verdict=boot_changed, the diff names it" \
    eval '[ "$rc" = 6 ] && grep -q "verdict=boot_changed .* boot_diff=changed" <<<"$(vline)" && grep -q "stm32mp257f-dk.dtb" <<<"$out"'
printf 'dtb\n' > "$F/boot/stm32mp257f-dk.dtb"

reset_board
echo "$F/console.refused" > "$F/state/console"
out=$(live run "$S/img/mp2_hello.bin" "$S/runs" refusal 2>&1); rc=$?
check "a refusal on the console: rc 4, verdict=refused, cleaned up, diffs empty" \
    eval '[ "$rc" = 4 ] && grep -q "verdict=refused .* boot_verdict=refused result=none env_diff=empty boot_diff=empty" <<<"$(vline)" && ! staged_on_card'

reset_board
echo noreboot > "$F/state/mode"
out=$(MP2_REBOOT_GRACE=2 live run "$S/img/mp2_hello.bin" "$S/runs" norb 2>&1); rc=$?
check "a reboot that did not happen: rc 4, verdict=not_rebooted, the staged files removed" \
    eval '[ "$rc" = 4 ] && grep -q "verdict=not_rebooted" <<<"$(vline)" && ! staged_on_card && [ ! -e "$LEFTF" ]'

reset_board
echo die > "$F/state/mode"
out=$(RUN_BOUND=2 RETURN_BOUND=2 live run "$S/img/mp2_hello.bin" "$S/runs" gone 2>&1); rc=$?
check "a board that never comes back: rc 7, verdict=not_back, NEEDS A PERSON, one reboot, no retry" \
    eval '[ "$rc" = 7 ] && grep -q "rc=7 verdict=not_back" <<<"$(vline)" && grep -q "NEEDS A PERSON" <<<"$out" && [ "$(grep -c reboot "$F/state/reboots")" = 1 ]'
check "not_back: the state file names the staged files; they are still on the card" \
    eval 'grep -q "mp2z-mp2_hello\|mp2z-gone" "$LEFTF" && staged_on_card'
rm -f "$F/state/dead"
out=$(live disarm 2>&1); rc=$?
check "then disarm: rc 0, the earlier record shown, nothing staged left, the state file gone" \
    eval '[ "$rc" = 0 ] && grep -q "an earlier run left this record" <<<"$out" && ! staged_on_card && [ ! -e "$LEFTF" ]'
out=$(live disarm 2>&1); rc=$?
check "disarm again: rc 0 (safe to repeat)" eval '[ "$rc" = 0 ]'

# a run cut off by TERM while the reboot's ssh is in flight: its exit undoes
reset_board
echo hang > "$F/state/mode"
( live run "$S/img/mp2_hello.bin" "$S/runs" cutterm > "$S/cut.out" 2>&1; echo "rc=$?" >> "$S/cut.out" ) &
CUT=$!
i=0; while [ ! -e "$F/state/reboots" ] && [ $i -lt 100 ]; do sleep 0.2; i=$((i + 1)); done
check "cut off: the run reached the reboot with the files staged" eval '[ -e "$F/state/reboots" ] && staged_on_card'
pkill -TERM -f "board-stm32mp2-uboot.sh run $S/img/mp2_hello.bin $S/runs cutterm"
wait "$CUT"
out=$(cat "$S/cut.out")
check "cut off by TERM: rc 143, the staged files removed by its exit, no state file" \
    eval 'grep -q "rc=143" <<<"$out" && grep -q "SIGTERM: ending through the undo" <<<"$out" && ! staged_on_card && [ ! -e "$LEFTF" ]'
pkill -f "$F/bin/systemctl" 2>/dev/null

# a run killed outright (SIGKILL, no trap): disarm leaves nothing behind
reset_board
echo hang > "$F/state/mode"
( live run "$S/img/mp2_hello.bin" "$S/runs" cutkill > /dev/null 2>&1 ) &
i=0; while [ ! -e "$F/state/reboots" ] && [ $i -lt 100 ]; do sleep 0.2; i=$((i + 1)); done
pkill -KILL -f "board-stm32mp2-uboot.sh run $S/img/mp2_hello.bin $S/runs cutkill"
sleep 0.5
pkill -f "$S/fakebin/fakecon" 2>/dev/null; pkill -f "$F/bin/systemctl" 2>/dev/null
sleep 2   # the lock's watchdog sees the runner gone within a second
check "SIGKILL: the staged files stay, the state file records them" eval 'staged_on_card && grep -q "mp2z-cutkill" "$LEFTF"'
rm -f "$F/state/mode"
out=$(live disarm 2>&1); rc=$?
check "SIGKILL, then disarm: rc 0, no mp2z file on the card, no state file, the stock files untouched" \
    eval '[ "$rc" = 0 ] && ! staged_on_card && [ ! -e "$F/boot/yakogl" ] && [ ! -e "$LEFTF" ] && grep -qx dtb "$F/boot/stm32mp257f-dk.dtb" && log_clean'

echo "== 5. live refusals (before anything is staged)"
# Each refusal has its own run name: the run directory is named by the second,
# the name and the image, and the runner never overwrites a record, so two
# refusals in the same second under one name would collide.
nrefusal=0
live_refused() { # live_refused DESCRIPTION PATTERN [ENV=VALUE ...]
    local what=$1 pat=$2
    shift 2
    nrefusal=$((nrefusal + 1))
    : > "$F/ssh.log"
    out=$(env "$@" PATH="$S/fakebin:$PATH" BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART MP2U_CONSOLE=$S/fakebin/fakecon LOCK=$S/board.lock LOCK_WAIT=0 MP2U_POLL=1 \
        "$B" run "$S/img/mp2_hello.bin" "$S/runs" "live_refusal$nrefusal" 2>&1)
    rc=$?
    check "$what: rc 3, nothing staged, no reboot" \
        eval '[ "$rc" = 3 ] && grep -q -- "$pat" <<<"$out" && ! grep -q "\.part\|reboot" "$F/ssh.log" && [ ! -e "$F/state/reboots" ] && ! leaks "$out"'
}
reset_board
: > "$F/ssh.log"
out=$(BOARD_HOST=$FAKE_HOST BOARD_UART=$FAKE_UART PATH="$S/fakebin:$PATH" MP2U_CONSOLE=$S/fakebin/fakecon LOCK=$S/board.lock LOCK_WAIT=0 "$B" run "$S/img/huge.bin" "$S/runs" 2>&1); rc=$?
check "255 MiB, live: rc 3 before one ssh" eval '[ "$rc" = 3 ] && [ ! -s "$F/ssh.log" ] && ! staged_on_card'
reset_board; touch "$F/state/badcrc"
live_refused "a bad CRC in fw_printenv" "bad CRC"
reset_board; printf '%s\n' "$ENV0" | grep -v scriptaddr > "$F/env.txt"
live_refused "scriptaddr unset" "scriptaddr is 'unset'"
reset_board; printf '%s\n' "$ENV0" | sed 's/^scriptaddr=.*/scriptaddr=0xa8000000/' > "$F/env.txt"
live_refused "scriptaddr inside the go window" "inside or next to the go window"
reset_board; printf '/dev/mmcblk0p8 ext4\n' > "$F/state/findmnt"
live_refused "/boot on the eMMC" "eMMC"
reset_board; mkdir -p "$F/boot/yakogl"; : > "$F/boot/yakogl/mp2z-old.bin"
live_refused "files of an earlier run on the card" "disarm' first"
rm -rf "$F/boot/yakogl"
reset_board; touch "$F/state/busy"
live_refused "a busy console" "busy or unreachable"
check "every live refusal: the board commands never wrote anything" log_clean

pkill -f "$S/fakebin/fakecon" 2>/dev/null
echo "board_stm32mp2_uboot_selftest: $n checks, $fails failed"
[ "$fails" = 0 ] && [ "$n" = "$WANT_CHECKS" ] || { [ "$n" = "$WANT_CHECKS" ] || echo "expected $WANT_CHECKS checks, ran $n"; exit 1; }

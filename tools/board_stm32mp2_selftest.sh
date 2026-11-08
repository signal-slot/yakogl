#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# board_stm32mp2_selftest.sh - tools/board-stm32mp2.sh without a board.
#
#   tools/board_stm32mp2_selftest.sh
#
# 1. Every verb under --dry-run, with ssh, scp, curl, fuser, flock, stty and
#    sudo replaced by stubs that only record that they were called: each verb
#    exits 0 and prints its board commands as "DRY ..." lines, the install's
#    undo comes before its first board command, no stub runs, and no command
#    of any verb names fsbla*, metadata*, fip-*, the eMMC (mmcblk0), the
#    rootfs, the whole microSD device or a partition tool; nothing writes the
#    U-Boot environment.
# 2. The refusals that need no board: an image without the watchdog line,
#    one too big for the window, one whose header booti would place elsewhere,
#    a bad run name, a BOARD_UART that is neither a device nor host:port, a
#    wait below 90 s.
# 3. The real path against a fake board: an ssh stub that runs each board
#    command in a directory tree standing in for the card's /boot (with
#    findmnt, df, systemctl, who and uname of its own), and a fake console (a
#    pty for the device form of BOARD_UART, a TCP bridge for the host:port
#    form) that plays the board's boot transcript after "systemctl reboot",
#    waits at "Enter choice: " and boots whatever is typed. It checks: install
#    then uninstall gives the stock bytes back (and refuses to overwrite a
#    file someone edited by hand); a run that completes (verdict done, the
#    hand-over line, the choice typed exactly once, the staged image removed,
#    no host name or console path in the record); a choice U-Boot did not
#    take (not_chosen, the board back); a board that never returns ("needs a
#    person", one reboot, no retry, exit 5); BOARD_HOST or BOARD_UART unset is
#    refused before any command.
# 4. The scan of synthetic captures: complete, not_chosen, label_not_offered,
#    looped, no_console.
# Prints "board_stm32mp2_selftest: N checks, F failed"; exit 0 only when F is
# 0 and N is the pinned count.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
B="$HERE/board-stm32mp2.sh"
WANT_CHECKS=61

S=$(mktemp -d)
PIDS=()
# SELFTEST_KEEP=1 keeps the scratch directory (a failing check's output is in
# fail-<n>.out there) and prints its path
cleanup() {
    local p
    for p in "${PIDS[@]+"${PIDS[@]}"}"; do kill "$p" 2>/dev/null; done
    wait 2>/dev/null
    if [ "${SELFTEST_KEEP:-0}" = 1 ]; then echo "board_stm32mp2_selftest: kept $S"; else rm -rf "$S"; fi
}
trap cleanup EXIT
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
        echo "  FAIL  $what (check $n)"
        fails=$((fails + 1))
        printf '%s\n' "$out" > "$S/fail-$n.out"
    fi
}
# forbidden NAMES-FILE: nothing in it names what the runner must never touch
FORBID='fsbla|metadata|fip-|mmcblk0|rootfs|mmcblk1([^p]|$)|sgdisk|parted|fdisk|fw_setenv|uboot_env\.py'
forbidden() { grep -aqE "$FORBID" "$1"; }

echo "board_stm32mp2_selftest: build $(git -C "$REPO" describe --always --dirty 2>/dev/null || echo unknown)"
unset BOARD_HOST BOARD_UART BOARD_BOOT RUN_BOUND RETURN_BOUND BOARD_END_RE NETBOOT_NO_WATCHDOG \
    STM32MP2_ARM_S STM32MP2_BACKUP_DIR STM32MP2_SELFTEST LOCK BOARD_LOCK LOCK_WAIT HOLD_MAX

# ---- fake images: the AArch64 Image header (text_offset, image_size, flags), a marker --
mkimg() { # mkimg FILE FOOTPRINT MARKER [TEXT_OFFSET] [FLAGS]
    python3 -I - "$1" "$2" "$3" "${4:-0}" "${5:-2}" <<'EOF'
import struct, sys
path, footprint, marker, toff, flags = sys.argv[1], int(sys.argv[2]), sys.argv[3], int(sys.argv[4], 0), int(sys.argv[5], 0)
hdr = b"\x00" * 8 + struct.pack("<QQQ", toff, footprint, flags) + b"\x00" * 24 + b"ARM\x64" + b"\x00" * 4
assert len(hdr) == 64
with open(path, "wb") as f:
    f.write(hdr + b"\x00" * 1000 + marker.encode() + b"\x00*** Booting Zephyr OS build selftest ***\x00")
EOF
}
mkdir -p "$S/img"
mkimg "$S/img/mp2.bin" 4194304 'board: watchdog armed'
mkimg "$S/img/booti.bin" 4194304 'board: watchdog armed' 0x20000000
mkimg "$S/img/pie.bin" 4194304 'board: watchdog armed' 0 0xa
mkimg "$S/img/toff.bin" 4194304 'board: watchdog armed' 0x1000
mkimg "$S/img/nomarker.bin" 4194304 'nothing arms here'
mkimg "$S/img/huge.bin" $((512 * 1024 * 1024)) 'board: watchdog armed'

# ---- stubs: any of these running is a failure of --dry-run --------------------
mkdir -p "$S/stub"
for c in ssh scp curl fuser flock stty sudo; do
    printf '#!/bin/sh\necho "%s $*" >> "%s/touched"\nexit 99\n' "$c" "$S" > "$S/stub/$c"
    chmod +x "$S/stub/$c"
done
dry() { PATH="$S/stub:$PATH" bash "$B" --dry-run "$@"; }
untouched() { [ ! -s "$S/touched" ]; }
first_line_no() { grep -n -- "$1" | head -1 | cut -d: -f1; }

echo "== 1. every verb under --dry-run (stubs record any board or console access)"
out=$(dry install 2>&1)
rc=$?
printf '%s\n' "$out" > "$S/dry-install.out"
check "dry install: rc 0, DRY board lines, the lease, no stub ran" \
    eval '[ "$rc" = 0 ] && grep -q "^DRY board" <<<"$out" && grep -q "yakogl-board.lease" <<<"$out" && untouched'
check "dry install: the undo (uninstall, by hand, without Linux, no env) printed before the first board command" \
    eval '[ "$(first_line_no "UNDO, from Linux" <<<"$out")" -lt "$(first_line_no "^DRY " <<<"$out")" ] &&
          [ "$(first_line_no "UNDO, by hand" <<<"$out")" -lt "$(first_line_no "^DRY " <<<"$out")" ] &&
          [ "$(first_line_no "UNDO, without Linux" <<<"$out")" -lt "$(first_line_no "^DRY " <<<"$out")" ] &&
          [ "$(first_line_no "environment is not written" <<<"$out")" -lt "$(first_line_no "^DRY " <<<"$out")" ]'
check "dry install: the stock copies and SHA256SUMS before the configuration is rewritten; the label block has KERNEL /yakogl/once.bin and the DEFAULT label's FDT" \
    eval '[ "$(first_line_no "cp -p /boot/mmc0_extlinux/extlinux.conf /boot/yakogl/stock/" <<<"$out")" -lt "$(first_line_no "cat > /boot/mmc0_extlinux/extlinux.conf.part" <<<"$out")" ] &&
          [ "$(first_line_no "SHA256SUMS.part" <<<"$out")" -lt "$(first_line_no "cat > /boot/mmc0_extlinux/extlinux.conf.part" <<<"$out")" ] &&
          grep -q "^    LABEL yakogl$" <<<"$out" && grep -q "KERNEL /yakogl/once.bin" <<<"$out" && grep -q "^    	FDT /stm32mp257f-dk.dtb$" <<<"$out"'
check "dry install: every configuration under /boot/mmc<n>_extlinux gets the label and is read back by sha256" \
    eval '[ "$(grep -c "the yakogl label appended" <<<"$out")" = 4 ] && [ "$(grep -c "^DRY board\$ sha256sum /boot/mmc[01]_extlinux/" <<<"$out")" = 4 ]'
check "dry install: root logs in, no sudo in any command; no forbidden name" \
    eval '! grep -q "sudo" <<<"$out" && ! forbidden "$S/dry-install.out"'
out=$(BOARD_HOST=user@BOARD dry install 2>&1)
check "dry install as user@BOARD: the root commands get sudo -n" grep -q "^DRY board\$ sudo -n install -d" <<<"$out"
out=$(dry install /boot/mmc0_extlinux/stm32mp257f-dk_extlinux.conf 2>&1)
rc=$?
check "dry install CONF: only that file, rc 0" \
    eval '[ "$rc" = 0 ] && [ "$(grep -c "the yakogl label appended" <<<"$out")" = 1 ] && ! grep -q "ls /boot/mmc\[0-9\]" <<<"$out"'
out=$(dry install /etc/fstab 2>&1)
rc=$?
check "dry install of a file outside /boot: rc 3, no board command" eval '[ "$rc" = 3 ] && ! grep -q "^DRY board" <<<"$out"'
out=$(dry uninstall 2>&1)
rc=$?
printf '%s\n' "$out" > "$S/dry-uninstall.out"
check "dry uninstall: rc 0, the stock copy's sha256 checked before the restore, the restore read back, /boot/yakogl removed, no forbidden name" \
    eval '[ "$rc" = 0 ] && [ "$(first_line_no "sha256sum /boot/yakogl/stock/" <<<"$out")" -lt "$(first_line_no "cp /boot/yakogl/stock/" <<<"$out")" ] &&
          grep -q "restored byte for byte" <<<"$out" && grep -q "rm -rf /boot/yakogl && sync && test ! -e /boot/yakogl" <<<"$out" && ! forbidden "$S/dry-uninstall.out" && untouched'
out=$(dry status 2>&1)
rc=$?
printf '%s\n' "$out" > "$S/dry-status.out"
check "dry status: rc 0, the lease and the install state, BOARD_UART never printed, no forbidden name" \
    eval '[ "$rc" = 0 ] && grep -q "board lease" <<<"$out" && grep -q "^install: " <<<"$out" && ! forbidden "$S/dry-status.out"'
out=$(BOARD_UART=/dev/ttyFAKE9 dry status 2>&1)
check "dry status with BOARD_UART set: its value is not printed" eval '! grep -q "ttyFAKE9" <<<"$out" && grep -q "console: set" <<<"$out"'
out=$(dry run "$S/img/mp2.bin" "$S/logs" 2>&1)
rc=$?
printf '%s\n' "$out" > "$S/dry-run.out"
check "dry run: rc 0, the image checked, the lock and the lease, nothing on the host, no stub ran" \
    eval '[ "$rc" = 0 ] && grep -q "image sha256 $(sha256sum "$S/img/mp2.bin" | cut -c1-16)" <<<"$out" && grep -q "board_lock.sh" <<<"$out" &&
          grep -q "yakogl-board.lease" <<<"$out" && [ ! -e "$S/logs" ] && untouched'
check "dry run: the booti form (text_offset 0 -> 0x20000000), staged as /boot/yakogl/once.bin and read back after drop_caches" \
    eval 'grep -q "booti form: text_offset 0 -> 0x20000000" <<<"$out" && grep -q "cat > /boot/yakogl/once.bin.part" <<<"$out" &&
          grep -q "drop_caches >/dev/null; sha256sum /boot/yakogl/once.bin" <<<"$out"'
check "dry run: /boot checked (findmnt, df), the install state and no leftover checked, the console client started before the reboot, typing once" \
    eval 'grep -q "findmnt -n -o SOURCE,FSTYPE -T /boot" <<<"$out" && grep -q "df -B1 --output=avail /boot" <<<"$out" &&
          grep -q "the label must be in every configuration" <<<"$out" && grep -q "ls /boot/yakogl/once\*.bin" <<<"$out" &&
          [ "$(first_line_no "scan every /proc/<pid>/fd for /dev/UART" <<<"$out")" -lt "$(first_line_no "systemctl reboot" <<<"$out")" ] &&
          [ "$(first_line_no "capture /dev/UART" <<<"$out")" -lt "$(first_line_no "systemctl reboot" <<<"$out")" ] &&
          grep -q "choose yakogl --arm-s 180" <<<"$out" && grep -q "types the menu number once" <<<"$out"'
check "dry run: waits 240 s (120 + 120) for a new boot, says a person is needed otherwise, no retry; the staged image removed; no env write; no forbidden name" \
    eval 'grep -q "<= 240 s; else: the board needs a person, no retry" <<<"$out" && grep -q "rm -f /boot/yakogl/once.bin && test ! -e /boot/yakogl/once.bin" <<<"$out" &&
          ! grep -q "fw_setenv\|uboot_env\|saveenv" <<<"$out" && ! forbidden "$S/dry-run.out"'
out=$(RUN_BOUND=200 RETURN_BOUND=100 dry run "$S/img/mp2.bin" "$S/logs" 2>&1)
check "dry run: RUN_BOUND and RETURN_BOUND override the defaults" grep -q "<= 300 s;" <<<"$out"
out=$(BOARD_UART=bridge.invalid:1 dry run "$S/img/mp2.bin" "$S/logs" 2>&1)
rc=$?
check "dry run, BOARD_UART host:port: the bridge probed and captured, no fd scan, rc 0" \
    eval '[ "$rc" = 0 ] && grep -q "am62p_netboot_uart.py probe bridge.invalid:1" <<<"$out" && grep -q "capture bridge.invalid:1 <rundir>/console.raw" <<<"$out" &&
          ! grep -q "/proc/<pid>/fd" <<<"$out"'
out=$(dry run "$S/img/booti.bin" "$S/logs" 2>&1)
check "dry run of an image already in booti form: staged as built" grep -q "booti form: as built (text_offset 0x20000000)" <<<"$out"
out=$(dry verdict "$S/nothing" 2>&1)
rc=$?
check "verdict of a directory without a capture: rc 5-shaped not_back line, no board command" \
    eval 'grep -q "verdict=not_back boot=uboot-extlinux row boot_verdict=not_back" <<<"$out" && ! grep -q "^DRY board" <<<"$out"'
check "dry runs: nothing written on the host, no stub ran" eval '[ ! -e "$S/logs" ] && untouched'

echo "== 2. refusals that need no board"
out=$(dry run "$S/img/nomarker.bin" "$S/logs" 2>&1)
rc=$?
check "an image without 'board: watchdog armed': rc 3, REFUSED, names the override, no board command" \
    eval '[ "$rc" = 3 ] && grep -q "REFUSED" <<<"$out" && grep -q "NETBOOT_NO_WATCHDOG=1" <<<"$out" && ! grep -q "^DRY board" <<<"$out"'
out=$(NETBOOT_NO_WATCHDOG=1 dry run "$S/img/nomarker.bin" "$S/logs" 2>&1)
rc=$?
check "NETBOOT_NO_WATCHDOG=1: the same image runs with a WARNING" eval '[ "$rc" = 0 ] && grep -q "WARNING: NETBOOT_NO_WATCHDOG=1" <<<"$out"'
out=$(dry run "$S/img/huge.bin" "$S/logs" 2>&1)
rc=$?
check "an image past the 256 MiB window: rc 3" eval '[ "$rc" = 3 ] && grep -q "does not fit" <<<"$out"'
out=$(dry run "$S/img/pie.bin" "$S/logs" 2>&1)
rc=$?
check "a header with flag bit 3: rc 3 (booti would start it at kernel_addr_r)" eval '[ "$rc" = 3 ] && grep -q "flag bit 3" <<<"$out"'
out=$(dry run "$S/img/toff.bin" "$S/logs" 2>&1)
rc=$?
check "a header with another text_offset: rc 3 (booti would start it elsewhere)" eval '[ "$rc" = 3 ] && grep -q "text_offset 0x1000" <<<"$out"'
out=$(dry run "$S/img/mp2.bin" "$S/logs" 'bad name' 2>&1)
rc=$?
check "a bad run name: rc 2" [ "$rc" = 2 ]
out=$(BOARD_UART=/notadevice dry run "$S/img/mp2.bin" "$S/logs" 2>&1)
rc=$?
check "BOARD_UART neither a device nor host:port: rc 3, REFUSED, no board command" \
    eval '[ "$rc" = 3 ] && grep -q "REFUSED: BOARD_UART" <<<"$out" && ! grep -q "^DRY board" <<<"$out"'
out=$(RUN_BOUND=40 RETURN_BOUND=40 dry run "$S/img/mp2.bin" "$S/logs" 2>&1)
rc=$?
check "RUN_BOUND + RETURN_BOUND below 90 s: rc 2" eval '[ "$rc" = 2 ] && grep -q "never below 90 s" <<<"$out"'
check "refusals: no stub ran" untouched

# ---- the fake board -----------------------------------------------------------------
# mkfake DIR: the card's /boot (four stock configurations), /proc, /var/tmp,
# the board's own commands (bbin: inside the ssh stub only) and the host's
# ssh stub (hbin). The stub runs the command with /boot, /proc and /var/tmp
# redirected into DIR and logs it to DIR/ssh.log (and to FAKE_LOG when set,
# beside the runner's own output). After "systemctl reboot" the board is
# down (every ssh fails) until the fake console says Linux is up (DIR/back),
# then it answers with a new boot_id and a short uptime.
mkfake() {
    local d=$1 c
    mkdir -p "$d/boot/mmc0_extlinux" "$d/boot/mmc1_extlinux" "$d/proc/sys/kernel/random" "$d/proc/sys/vm" "$d/var/tmp" "$d/bbin" "$d/hbin" "$d/stock"
    for c in mmc0_extlinux/extlinux.conf mmc0_extlinux/stm32mp257f-dk_extlinux.conf mmc1_extlinux/extlinux.conf mmc1_extlinux/stm32mp257f-dk_extlinux.conf; do
        printf '# Generic Distro Configuration file generated by OpenEmbedded\nmenu title Select the boot mode\nTIMEOUT 20\nDEFAULT stm32mp257f-dk-sdcard\nLABEL stm32mp257f-dk-sdcard\n\tKERNEL /Image.gz\n\tFDT /stm32mp257f-dk.dtb\n\tAPPEND root=PARTUUID=e91c4e10-16e6-4c0e-bd0e-77becf4a3582 rootwait rw console=ttySTM0,115200\n%s' \
            "$([ "${c#mmc1}" != "$c" ] && printf 'LABEL stm32mp257f-dk-emmc\n\tKERNEL /Image.gz\n\tFDT /stm32mp257f-dk.dtb\n')" > "$d/boot/$c"
        cp "$d/boot/$c" "$d/stock/${c//\//__}"
    done
    : > "$d/boot/Image.gz"
    echo "11111111-2222-3333-4444-555555555555" > "$d/proc/sys/kernel/random/boot_id"
    echo "900.00 1800.00" > "$d/proc/uptime"
    : > "$d/proc/sys/vm/drop_caches"
    printf '#!/bin/sh\necho "/dev/mmcblk1p8 ext4 /boot" | cut -d" " -f1-$( [ "$2" = "-o" ] && echo "$3" | tr -dc , | wc -c | xargs expr 1 + )\n' > "$d/bbin/findmnt"
    printf '#!/bin/sh\necho 33000000\n' > "$d/bbin/df"
    printf '#!/bin/sh\nexit 0\n' > "$d/bbin/sync"
    printf '#!/bin/sh\necho\n' > "$d/bbin/who"
    printf '#!/bin/sh\necho fake-mp2-board\n' > "$d/bbin/uname"
    printf '#!/bin/sh\nif [ "$1" = -n ]; then shift; fi\nexec "$@"\n' > "$d/bbin/sudo"
    printf '#!/bin/sh\n[ "$1" = reboot ] || exit 1\ntouch "%s/down" "%s/reboot"\necho "fake board: rebooting"\n' "$d" "$d" > "$d/bbin/systemctl"
    cat > "$d/hbin/ssh" <<EOF
#!/bin/bash
# the fake board: options skipped, the last argument is the command
FAKE=$d
cmd=\${*: -1}
if [ -e "\$FAKE/down" ]; then
    if [ -e "\$FAKE/back" ]; then
        rm -f "\$FAKE/down" "\$FAKE/back"
        cat /proc/sys/kernel/random/uuid > "\$FAKE/proc/sys/kernel/random/boot_id"   # a new boot: a new id
        echo "1.50 3.00" > "\$FAKE/proc/uptime"
    else
        echo "ssh: connect to host: Connection refused" >&2
        exit 255
    fi
fi
echo "ssh: \$cmd" >> "\$FAKE/ssh.log"
[ -n "\${FAKE_LOG:-}" ] && echo "ssh: \$cmd" >> "\$FAKE_LOG"
cmd=\$(sed -E -e "s#(^|[^A-Za-z0-9_/])/proc/#\\1\$FAKE/proc/#g" -e "s#(^|[^A-Za-z0-9_/])/boot(/|[^A-Za-z0-9_/]|\$)#\\1\$FAKE/boot\\2#g" -e "s#(^|[^A-Za-z0-9_/])/var/tmp/#\\1\$FAKE/var/tmp/#g" <<<"\$cmd")
# the board names its files /boot/...: the fake prefix never reaches the runner
set -o pipefail
PATH="\$FAKE/bbin:\$PATH" bash -c "\$cmd" | sed "s#\$FAKE##g"
EOF
    chmod +x "$d"/bbin/* "$d/hbin/ssh"
}

# The fake console: fake_console.py FAKE pty|tcp SCENARIO READY-FILE. It
# waits for FAKE/reboot, plays TF-A, U-Boot, the menu, "Enter choice: " (1.5
# s for a line), and boots what was typed: the yakogl label runs the
# transcript of mp2_hello and resets into a second boot whose menu gets no
# key (anything typed there prints LOOP-TYPED), then Linux (FAKE/back). The
# scenario: complete; hang (the image prints one line and nothing more, the
# board never returns); notchosen (the typed key is lost, Linux boots);
# notoffered (the menu has no yakogl line). Everything received is appended
# to FAKE/console.rx. pty: a pseudo-terminal whose slave path is written to
# READY-FILE (the slave closed again, so no process holds it); tcp: a serial
# bridge on 127.0.0.1 (the port in READY-FILE, "busy" to a second client).
cat > "$S/fake_console.py" <<'EOF'
import os, pty, select, socket, sys, termios, time
FAKE, mode, scenario, ready = sys.argv[1:5]
rx = open(os.path.join(FAKE, "console.rx"), "ab", buffering=0)


class Dev:
    def __init__(self):
        self.m, s = pty.openpty()
        attr = termios.tcgetattr(s)
        attr[0] = attr[1] = attr[3] = 0
        termios.tcsetattr(s, termios.TCSANOW, attr)
        name = os.ttyname(s)
        os.close(s)
        with open(ready, "w") as f:
            f.write(name)

    def write(self, b):
        try:
            os.write(self.m, b)
        except OSError:
            pass

    def read(self, timeout):
        r, _, _ = select.select([self.m], [], [], timeout)
        if not r:
            return b""
        try:
            return os.read(self.m, 4096)
        except OSError:
            time.sleep(0.05)
            return b""


class Net:
    def __init__(self):
        self.ls = socket.socket()
        self.ls.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.ls.bind(("127.0.0.1", 0))
        self.ls.listen(4)
        self.ls.setblocking(False)
        self.c = None
        with open(ready, "w") as f:
            f.write("127.0.0.1:%d" % self.ls.getsockname()[1])

    def accept(self):
        try:
            c, _ = self.ls.accept()
        except BlockingIOError:
            return
        if self.c is not None:
            try:
                c.sendall(b"busy\r\n")
            except OSError:
                pass
            c.close()
            return
        c.setblocking(False)
        self.c = c

    def write(self, b):
        self.accept()
        if self.c is not None:
            try:
                self.c.sendall(b)
            except OSError:
                self.c = None

    def read(self, timeout):
        self.accept()
        if self.c is None:
            time.sleep(min(timeout, 0.05))
            return b""
        r, _, _ = select.select([self.c], [], [], timeout)
        if not r:
            return b""
        try:
            d = self.c.recv(4096)
        except OSError:
            d = b""
        if not d:
            self.c.close()
            self.c = None
        return d


dev = Dev() if mode == "pty" else Net()


def say(line):
    dev.write(line.encode() + b"\r\n")


def line_in(timeout):
    buf, t0 = b"", time.time()
    while time.time() - t0 < timeout:
        d = dev.read(0.05)
        if d:
            rx.write(d)
            buf += d
            if b"\r" in buf or b"\n" in buf:
                break
    return buf.replace(b"\n", b"\r").split(b"\r")[0].decode("ascii", "replace").strip()


def banner():
    say("NOTICE:  CPU: STM32MP257FAI Rev.Y")
    say("NOTICE:  BL2: v2.10-stm32mp2")
    say("U-Boot 2023.10-stm32mp (Jan 01 2026 - 00:00:00 +0000)")
    say("Hit any key to stop autoboot:  0")
    say("Found /mmc0_extlinux/stm32mp257f-dk_extlinux.conf")


def menu():
    say("Select the boot mode")
    say("1:\tstm32mp257f-dk-sdcard")
    if scenario != "notoffered":
        say("2:\tyakogl")
    dev.write(b"Enter choice: ")
    choice = line_in(1.5)
    if choice:
        dev.write(choice.encode() + b"\r\n")
    return choice


def linux():
    say("Retrieving file: /Image.gz")
    say("Starting kernel ...")
    with open(os.path.join(FAKE, "back"), "w"):
        pass


trigger = os.path.join(FAKE, "reboot")
while True:
    while not os.path.exists(trigger):
        dev.read(0.05)
    os.unlink(trigger)
    banner()
    choice = menu()
    if choice == "2" and scenario != "notchosen":
        say("2:\tyakogl")
        say("Retrieving file: /yakogl/once.bin")
        say("4195328 bytes read in 200 ms (20 MiB/s)")
        say("Moving Image from 0x8a000000 to 0xa0000000, end=a0400000")
        say("Starting kernel ...")
        say("MP2:A EL2")
        say("MP2:A1")
        say("MP2:B console up (USART2, polled)")
        say("mp2-handover: v=1 el=2 sctlr=0x30c5183d m=0 c=0 i=1 x0=0x90000000 x1=0x0 x2=0x0 x3=0x0 "
            "pc=0xa0000000 cntfrq=40000000 mpidr=0x80000000 psci=0x10001")
        say("board: watchdog armed")
        say("*** Booting Zephyr OS build v4.4.99 ***")
        if scenario == "hang":
            say("mp2_hello: 1 s (uptime 1000 ms)")
            continue
        say("mp2_hello: PASS: counted 5 s in 5002 ms")
        say("mp2: mp2_hello done: PSCI SYSTEM_RESET at 5500 ms")
        banner()
        again = menu()
        if again:
            say("LOOP-TYPED %s" % again)
        else:
            dev.write(b"1:\tstm32mp257f-dk-sdcard\r\n")
        linux()
    else:
        if not choice:
            dev.write(b"1:\tstm32mp257f-dk-sdcard\r\n")
        else:
            say("1:\tstm32mp257f-dk-sdcard")
        linux()
EOF

# console_start MODE SCENARIO: the fake console in the background; CONSOLE is
# BOARD_UART's value for it
console_start() {
    local ready=$FAKE/console.ready i
    rm -f "$ready" "$FAKE/console.rx"
    python3 -I "$S/fake_console.py" "$FAKE" "$1" "$2" "$ready" &
    CONSOLE_PID=$!
    PIDS+=("$CONSOLE_PID")
    for i in $(seq 1 100); do [ -s "$ready" ] && break; sleep 0.1; done
    CONSOLE=$(cat "$ready")
    [ -n "$CONSOLE" ]
}
console_stop() { kill "$CONSOLE_PID" 2>/dev/null; wait "$CONSOLE_PID" 2>/dev/null; }

# real VERB...: the runner against the fake board (no dry run)
real() {
    PATH="$FAKE/hbin:$PATH" BOARD_HOST=root@fake-mp2.invalid BOARD_LOCK=$S/host.lock STM32MP2_SELFTEST=1 \
        STM32MP2_BACKUP_DIR=$S/backup LOCK_WAIT=5 HOLD_MAX=120 bash "$B" "$@"
}
stock_equal() { # stock_equal FAKE: every configuration is byte for byte the stock one
    local c
    for c in mmc0_extlinux/extlinux.conf mmc0_extlinux/stm32mp257f-dk_extlinux.conf mmc1_extlinux/extlinux.conf mmc1_extlinux/stm32mp257f-dk_extlinux.conf; do
        cmp -s "$1/boot/$c" "$1/stock/${c//\//__}" || return 1
    done
}

echo "== 3. the real path against a fake board"
FAKE=$S/fake
mkfake "$FAKE"
out=$(PATH="$FAKE/hbin:$PATH" STM32MP2_SELFTEST=1 bash "$B" run "$S/img/mp2.bin" "$S/logs" 2>&1)
rc=$?
check "run with BOARD_HOST unset: rc 3, REFUSED, no board command, nothing on the host" \
    eval '[ "$rc" = 3 ] && grep -q "REFUSED: set BOARD_HOST" <<<"$out" && [ ! -e "$FAKE/ssh.log" ] && [ ! -e "$S/logs" ]'
out=$(PATH="$FAKE/hbin:$PATH" BOARD_HOST=root@fake-mp2.invalid STM32MP2_SELFTEST=1 bash "$B" run "$S/img/mp2.bin" "$S/logs" 2>&1)
rc=$?
check "run with BOARD_UART unset: rc 3, REFUSED, no board command" \
    eval '[ "$rc" = 3 ] && grep -q "REFUSED: set BOARD_UART" <<<"$out" && [ ! -e "$FAKE/ssh.log" ] && [ ! -e "$S/logs" ]'
out=$(PATH="$FAKE/hbin:$PATH" BOARD_HOST=root@fake-mp2.invalid BOARD_UART=/nonexistent/tty STM32MP2_SELFTEST=1 bash "$B" run "$S/img/mp2.bin" "$S/logs" 2>&1)
rc=$?
check "run with BOARD_UART not a character device: rc 3, no board command" eval '[ "$rc" = 3 ] && [ ! -e "$FAKE/ssh.log" ]'
# the run before the install refuses
console_start pty complete || echo "  (no fake console)"
out=$(BOARD_UART=$CONSOLE real run "$S/img/mp2.bin" "$S/logs" 2>&1)
rc=$?
check "run before install: rc 3 (not installed), no reboot, nothing staged" \
    eval '[ "$rc" = 3 ] && grep -q "not installed" <<<"$out" && ! grep -q "systemctl reboot" "$FAKE/ssh.log" && [ ! -e "$FAKE/boot/yakogl" ]'
console_stop
rm -f "$FAKE/ssh.log"
# install: the undo before the first write, the stock copies, the label
out=$(FAKE_LOG=$S/install.log real install >> "$S/install.log" 2>&1; echo "rc=$?")
rc=${out##*rc=}
check "install: rc 0, the undo printed before the first board command that writes" \
    eval '[ "$rc" = 0 ] && [ "$(first_line_no "UNDO, from Linux" < "$S/install.log")" -lt "$(grep -nE "^ssh: .*(cat >|cp -p|install -d)" "$S/install.log" | head -1 | cut -d: -f1)" ] &&
          [ "$(first_line_no "UNDO, without Linux" < "$S/install.log")" -lt "$(first_line_no "^ssh: " < "$S/install.log")" ]'
check "install: every configuration has the yakogl label and is otherwise the stock file" \
    eval '( for c in mmc0_extlinux/extlinux.conf mmc0_extlinux/stm32mp257f-dk_extlinux.conf mmc1_extlinux/extlinux.conf mmc1_extlinux/stm32mp257f-dk_extlinux.conf; do
              grep -q "^LABEL yakogl$" "$FAKE/boot/$c" && grep -q "KERNEL /yakogl/once.bin" "$FAKE/boot/$c" &&
              grep -q "^DEFAULT stm32mp257f-dk-sdcard$" "$FAKE/boot/$c" && grep -q "^TIMEOUT 20$" "$FAKE/boot/$c" &&
              [ "$(head -c "$(stat -c %s "$FAKE/stock/${c//\//__}")" "$FAKE/boot/$c" | sha256sum)" = "$(sha256sum < "$FAKE/stock/${c//\//__}")" ] || exit 1
          done )'
check "install: the stock copies on the card and on the host are the stock bytes, SHA256SUMS names all four" \
    eval '( for c in mmc0_extlinux__extlinux.conf mmc0_extlinux__stm32mp257f-dk_extlinux.conf mmc1_extlinux__extlinux.conf mmc1_extlinux__stm32mp257f-dk_extlinux.conf; do
              cmp -s "$FAKE/boot/yakogl/stock/$c" "$FAKE/stock/$c" && cmp -s "$S"/backup/before-install-*/stock-$c "$FAKE/stock/$c" || exit 1
          done ) && [ "$(wc -l < "$FAKE/boot/yakogl/stock/SHA256SUMS")" = 4 ] && (cd "$FAKE/boot/yakogl/stock" && sha256sum -c --quiet SHA256SUMS.stock)'
check "install: no forbidden name in any board command, no env write" eval '! forbidden "$FAKE/ssh.log"'
out=$(real install 2>&1)
rc=$?
check "install again: already installed, nothing written" eval '[ "$rc" = 0 ] && grep -q "already installed" <<<"$out" && ! grep -q "cat >" <<<"$out"'
out=$(real status 2>&1)
check "status: install ok" grep -q "^install: ok" <<<"$out"
# a complete run over the device form of the console
console_start pty complete || echo "  (no fake console)"
rm -f "$FAKE/ssh.log"
out=$(RUN_BOUND=20 RETURN_BOUND=20 BOARD_UART=$CONSOLE real run "$S/img/mp2.bin" "$S/logs" hello 2>&1)
rc=$?
console_stop
printf '%s\n' "$out" > "$S/run-complete.out"
RD=$(ls -d "$S"/logs/*-hello-* 2>/dev/null | head -1)
check "run (device console): rc 0, verdict done, boot complete, the end line, the hand-over line, the lease taken and released" \
    eval '[ "$rc" = 0 ] && grep -q "^board.sh stm32mp257f-dk hello rc=0 verdict=done boot=uboot-extlinux row boot_verdict=complete result=legacy:pass resets=absent record=absent back_after=[0-9]*s handover=el=2,sctlr=0x30c5183d,m=0,c=0,i=1,x0=0x90000000," <<<"$out" &&
          grep -q "yakogl-board.lease" "$FAKE/ssh.log" && [ ! -e "$FAKE/var/tmp/yakogl-board.lease" ]'
check "run: the choice typed exactly once (2 CR), nothing typed at the second menu, one reboot" \
    eval '[ "$(tr -d "\n" < "$FAKE/console.rx" | od -A n -c | tr -d " \n")" = "2\\r" ] && ! grep -aq "LOOP-TYPED" "$RD/console.ts" && [ "$(grep -c "systemctl reboot" "$FAKE/ssh.log")" = 1 ]'
check "run: the image staged in booti form, read back after drop_caches, removed after Linux came back" \
    eval 'grep -q "drop_caches >/dev/null; sha256sum /boot/yakogl/once.bin" "$FAKE/ssh.log" && [ ! -e "$FAKE/boot/yakogl/once.bin" ] &&
          grep -q "^cleanup=/boot/yakogl/once.bin removed (read back)" "$RD/meta.txt" && grep -q "^staged_sha256=$(sha256sum "$S/img/booti.bin" | cut -d" " -f1)" "$RD/meta.txt"'
check "run: the record (meta.txt, console.raw/.ts/.txt/.events, INDEX.txt) names neither the host nor the console device" \
    eval '[ -s "$RD/INDEX.txt" ] && [ -s "$RD/console.txt" ] && [ -s "$RD/console.events" ] && [ -s "$RD/meta.txt" ] &&
          ! grep -rq "fake-mp2" "$RD" && ! grep -rqF "$CONSOLE" "$RD" && grep -q "verdict: board.sh stm32mp257f-dk hello rc=0 verdict=done" "$RD/INDEX.txt"'
check "run: no forbidden name in any board command" eval '! forbidden "$FAKE/ssh.log"'
out=$(bash "$B" verdict "$RD" 2>&1)
rc=$?
check "verdict of the run directory: the same line, rc 0" \
    eval '[ "$rc" = 0 ] && [ "$(grep "^board.sh stm32mp257f-dk hello" <<<"$out")" = "$(grep "^board.sh stm32mp257f-dk hello" "$S/run-complete.out")" ]'
# the same over the host:port form (a serial bridge)
console_start tcp complete || echo "  (no fake console)"
rm -f "$FAKE/ssh.log"
out=$(RUN_BOUND=20 RETURN_BOUND=20 BOARD_UART=$CONSOLE real run "$S/img/mp2.bin" "$S/logs" bridge 2>&1)
rc=$?
console_stop
check "run (serial bridge console): rc 0, verdict done, the bridge probed first, the choice typed once" \
    eval '[ "$rc" = 0 ] && grep -q "verdict=done boot=uboot-extlinux row boot_verdict=complete" <<<"$out" && [ "$(tr -d "\n" < "$FAKE/console.rx" | od -A n -c | tr -d " \n")" = "2\\r" ]'
# a choice U-Boot did not take: Linux boots, the board is back, rc 4
console_start pty notchosen || echo "  (no fake console)"
rm -f "$FAKE/ssh.log"
out=$(RUN_BOUND=20 RETURN_BOUND=20 BOARD_UART=$CONSOLE real run "$S/img/mp2.bin" "$S/logs" lost 2>&1)
rc=$?
console_stop
check "run, the typed key lost: rc 4, verdict not_chosen, the board back, the staged image removed" \
    eval '[ "$rc" = 4 ] && grep -q "verdict=not_chosen boot=uboot-extlinux row boot_verdict=not_chosen result=none" <<<"$out" && [ ! -e "$FAKE/boot/yakogl/once.bin" ]'
# the menu without our label: nothing typed
console_start pty notoffered || echo "  (no fake console)"
out=$(RUN_BOUND=20 RETURN_BOUND=20 BOARD_UART=$CONSOLE real run "$S/img/mp2.bin" "$S/logs" nolabel 2>&1)
rc=$?
console_stop
check "run, the menu without the label: rc 4, label_not_offered, nothing typed" \
    eval '[ "$rc" = 4 ] && grep -q "boot_verdict=label_not_offered" <<<"$out" && [ ! -s "$FAKE/console.rx" ]'
# the board never returns: a person, one reboot, no retry, exit 5
console_start pty hang || echo "  (no fake console)"
rm -f "$FAKE/ssh.log"
t0=$(date +%s)
out=$(RUN_BOUND=6 RETURN_BOUND=6 BOARD_UART=$CONSOLE real run "$S/img/mp2.bin" "$S/logs" hang 2>&1)
rc=$?
console_stop
check "run, no return: rc 5, THE BOARD NEEDS A PERSON, verdict not_back, exactly one reboot, no retry, the staged image left, within the bound" \
    eval '[ "$rc" = 5 ] && grep -q "THE BOARD NEEDS A PERSON" <<<"$out" && grep -q "rc=5 verdict=not_back boot=uboot-extlinux row boot_verdict=not_back" <<<"$out" &&
          [ "$(grep -c "systemctl reboot" "$FAKE/ssh.log")" = 1 ] && [ -e "$FAKE/boot/yakogl/once.bin" ] && [ $(( $(date +%s) - t0 )) -lt 60 ]'
check "run, no return: the board lease stays held (no other run takes the board until the owner says it is back)" \
    eval 'grep -q "^row=hang" "$FAKE/var/tmp/yakogl-board.lease"'
# the owner brought the board back and cleared the lease
rm -f "$FAKE/down" "$FAKE/back" "$FAKE/reboot" "$FAKE/var/tmp/yakogl-board.lease"
# the leftover refuses the next run before anything else
console_start pty complete || echo "  (no fake console)"
rm -f "$FAKE/ssh.log"
out=$(RUN_BOUND=20 RETURN_BOUND=20 BOARD_UART=$CONSOLE real run "$S/img/mp2.bin" "$S/logs" after 2>&1)
rc=$?
console_stop
check "run with a leftover staged image: rc 3, no reboot" eval '[ "$rc" = 3 ] && grep -q "did not clean up" <<<"$out" && ! grep -q "systemctl reboot" "$FAKE/ssh.log"'
# uninstall refuses a file someone edited by hand, then restores the stock bytes
cp "$FAKE/boot/mmc1_extlinux/extlinux.conf" "$S/edited.keep"
echo "# edited by hand" > "$FAKE/boot/mmc1_extlinux/extlinux.conf"
out=$(real uninstall 2>&1)
rc=$?
check "uninstall with a configuration edited by hand: rc 3, that file left alone, the others still ours (nothing restored)" \
    eval '[ "$rc" = 3 ] && grep -q "someone changed it since the install" <<<"$out" && [ "$(cat "$FAKE/boot/mmc1_extlinux/extlinux.conf")" = "# edited by hand" ] &&
          grep -q "^LABEL yakogl$" "$FAKE/boot/mmc0_extlinux/extlinux.conf" && grep -q "^LABEL yakogl$" "$FAKE/boot/mmc1_extlinux/stm32mp257f-dk_extlinux.conf" &&
          ! grep -q "restored byte for byte" <<<"$out"'
cp "$S/edited.keep" "$FAKE/boot/mmc1_extlinux/extlinux.conf"
rm -f "$FAKE/ssh.log"
out=$(real uninstall 2>&1)
rc=$?
check "uninstall: rc 0, every configuration is the stock file byte for byte, /boot/yakogl gone (the leftover with it)" \
    eval '[ "$rc" = 0 ] && stock_equal "$FAKE" && [ ! -e "$FAKE/boot/yakogl" ] && [ "$(grep -c "restored byte for byte" <<<"$out")" = 4 ]'
check "uninstall: no forbidden name in any board command" eval '! forbidden "$FAKE/ssh.log"'
out=$(real status 2>&1)
check "status after uninstall: absent" grep -q "^install: absent" <<<"$out"
out=$(real uninstall 2>&1)
rc=$?
check "uninstall again: not installed, rc 0" eval '[ "$rc" = 0 ] && grep -q "not installed" <<<"$out"'

echo "== 4. the scan of synthetic captures"
python3 -I - "$B" "$S/scan" <<'EOF'
import os, sys
src = open(sys.argv[1]).read()
py = src.split("cat <<'PYEOF'\n", 1)[1].split("\nPYEOF\n", 1)[0]
os.makedirs(sys.argv[2], exist_ok=True)
open(os.path.join(sys.argv[2], "c.py"), "w").write(py + "\n")
boot = ["NOTICE:  CPU: STM32MP257FAI Rev.Y", "U-Boot 2023.10-stm32mp (Jan 01 2026)", "Hit any key to stop autoboot:  0",
        "Select the boot mode", "1:\tstm32mp257f-dk-sdcard", "2:\tyakogl"]
image = ["Enter choice: 2", "2:\tyakogl", "Retrieving file: /yakogl/once.bin", "4195328 bytes read in 200 ms",
         "Moving Image from 0x8a000000 to 0xa0000000, end=a0400000", "Starting kernel ...", "MP2:A EL2", "MP2:A1",
         "mp2-handover: v=1 el=2 sctlr=0x30c5183d m=0 c=0 i=1 x0=0x90000000 x1=0x0 x2=0x0 x3=0x0 pc=0xa0000000 cntfrq=40000000 mpidr=0x80000000 psci=0x10001",
         "board: watchdog armed", "*** Booting Zephyr OS build v4.4.99 ***", "mp2_hello: PASS: counted 5 s in 5002 ms",
         "mp2: mp2_hello done: PSCI SYSTEM_RESET at 5500 ms"]
linux = ["Enter choice: 1:\tstm32mp257f-dk-sdcard", "Starting kernel ..."]
cases = {
    "complete": boot + image + boot + linux,
    "notchosen": boot + linux,
    "notoffered": boot[:5] + linux,
    "looped": boot + image + boot + image,
    "empty": [],
}
for name, lines in cases.items():
    with open(os.path.join(sys.argv[2], name + ".ts"), "w") as f:
        for i, l in enumerate(lines):
            f.write("%.3f %s\n" % (1000.0 + i, l))
EOF
scan() { python3 -I "$S/scan/c.py" scan "$S/scan/$1.ts"; }
check "scan complete: verdict complete, the phases, the hand-over values, rc 0" \
    eval 'o=$(scan complete); [ $? = 0 ] && grep -q "^verdict=complete menu=.* offered=.* chosen=.* go=.* started=.* zephyr=.* armed=.* end=.* reset=.* linux=.* handover=el=2,sctlr=0x30c5183d,m=0,c=0,i=1,x0=0x90000000," <<<"$o"'
check "scan not_chosen" eval '[ "$(scan notchosen | cut -d" " -f1)" = verdict=not_chosen ]'
check "scan label_not_offered" eval '[ "$(scan notoffered | cut -d" " -f1)" = verdict=label_not_offered ]'
check "scan looped (our label taken twice)" eval '[ "$(scan looped | cut -d" " -f1)" = verdict=looped ]'
check "scan of an empty capture: no_console" eval '[ "$(scan empty | cut -d" " -f1)" = verdict=no_console ]'

echo "board_stm32mp2_selftest: $n checks, $fails failed"
[ "$n" = "$WANT_CHECKS" ] || { echo "board_stm32mp2_selftest: $n checks ran, $WANT_CHECKS pinned: a check was added or silently stopped running; update WANT_CHECKS with the change that did it"; exit 1; }
[ "$fails" = 0 ]

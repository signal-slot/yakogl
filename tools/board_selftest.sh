#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# board_selftest.sh - tools/board.sh without a board (tools/check.sh runs it).
#
#   tools/board_selftest.sh
#
# 1. Every verb on every board and backend under --dry-run (uboot-mmc, and
#    default/nodefault, included), against fake
#    images, with ssh, scp, curl, ping, fuser, flock, stty, sudo and kexec
#    replaced by stubs that only record that they were called: each verb must
#    exit 0 and print its board commands as "DRY ..." lines (the boot command
#    of its backend among them), and no stub may have run.
# 2. The refusals that need no board: an image without the watchdog marker,
#    one too big for the board's RAM window, a backend the board has not, an
#    unknown board, a bad run name.
# 3. Replays of real captures through the verdict, tally and diff, each against
#    what the old runners printed for the same input. The old runners are gone
#    (Step 1, driver/docs/one-method.md section 5); what they printed was
#    captured by running them at c8436f0d into tools/testdata/old-runner-outputs.txt:
#    - AM62P: the three U-Boot captures of 2026-10-03 in tools/testdata
#      (tools/am62p_netboot_uart.py scan, which stays, and the old AM62P
#      runner's probe rule);
#    - i.MX8MP: SNVS records (the old netboot runner's "verdict"), under
#      uboot-tftp and kexec-dev;
#    - Pi 4: three records of the 29460f96 full tree in tools/testdata (the old
#      tryboot runner's end-line rule; the old Pi 4 tree runner's tally and
#      baseline diff);
#    - i.MX8MP tree: tools/cts/imx8mp-full-tree-29460f96.results.tsv turned
#      back into records and tallied: results.tsv against the committed one,
#      rows.md, the TALLY line and the diff against the old tree runner's.
# 4. The record's own status (no old runner read it): APP and PROBES records
#    with a pass, a fail and an undefined status, and the gl_test set's CASE
#    records (CASE_SET passed, failed and inconsistent; the earlier
#    CASE_SET_FAIL form), on the Pi 4 console and in the i.MX8MP SNVS: done,
#    failed (rc 4) and bad_record (rc 4).
# Prints "board_selftest: N checks, F failed"; exit 0 only when F is 0 and N is
# the pinned count (a check that silently stops running fails the pin).
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
B="$HERE/board.sh"
TD="$HERE/testdata"
WANT_CHECKS=215

S=$(mktemp -d)
trap 'rm -rf "$S"' EXIT
n=0
fails=0
# (the description is not called d: an eval'd check reads the caller's $d)
# gold SECTION: the lines of one section of tools/testdata/old-runner-outputs.txt
gold() { awk -v h="### $1" '$0 == h { on = 1; next } /^### / { on = 0 } on' "$TD/old-runner-outputs.txt"; }
check() { # check DESCRIPTION COMMAND...
    local what=$1
    shift
    n=$((n + 1))
    if "$@"; then echo "  PASS  $what"; else echo "  FAIL  $what"; fails=$((fails + 1)); fi
}
same() { # same DESCRIPTION FILE-A FILE-B: the files are equal (the diff printed when not)
    local what=$1
    n=$((n + 1))
    if [ -s "$2" ] && cmp -s "$2" "$3"; then
        echo "  PASS  $what"
    else
        echo "  FAIL  $what"
        diff "$2" "$3" | head -20 | sed 's/^/        /'
        fails=$((fails + 1))
    fi
}
field() { sed -n "s/.* $1=\([^ ]*\).*/\1/p" | head -1; }
# says BOARD DIR PATTERN: board.sh's tally of DIR matches PATTERN (captured
# first: under pipefail a pipe would take board.sh's own exit status 4 or 5)
says() {
    local o
    o=$("$B" "$1" tally "$2" 2>&1)
    grep -q -- "$3" <<<"$o"
}

echo "board_selftest: build $(git -C "$REPO" describe --always --dirty 2>/dev/null || echo unknown)"
# yakogl-tools once, for every board.sh call below
(cd "$REPO" && cargo build -q -p yakogl-tools) || { echo "board_selftest: cannot build yakogl-tools"; exit 1; }
td=$(cd "$REPO" && cargo metadata --no-deps --format-version 1 | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
export YAKOGL_TOOLS=$td/debug/yakogl-tools
[ -x "$YAKOGL_TOOLS" ] || { echo "board_selftest: no $YAKOGL_TOOLS"; exit 1; }
# nothing of the operator's environment reaches the runs
unset BOARD_HOST BOARD_UART BOARD_BOOT PI4_HOST PI4_UART UART_ADDR NETBOOT_SERVER NETBOOT_TFTP_ROOT \
    NETBOOT_TFTP_LOG NETBOOT_ROUTE NETBOOT_NO_WATCHDOG SEL TREE_IMAGE TREE_IMAGE_DIR TREE_TAG TREE_ROWS \
    TREE_PREV LOCK BOARD_LOCK RUN_BOUND RETURN_BOUND BOARD_END_RE PI4_END_RE PI4_HDMI_TIMINGS BOARD_HDMI_TIMINGS

# ---- fake images: the AArch64 Image header, image_size, a marker ------------
mkimg() { # mkimg FILE FOOTPRINT MARKER
    local le
    le=$(printf '%016x' "$2" | sed -E 's/(..)(..)(..)(..)(..)(..)(..)(..)/\\x\8\\x\7\\x\6\\x\5\\x\4\\x\3\\x\2\\x\1/')
    {
        head -c 16 /dev/zero
        printf '%b' "$le"
        head -c 32 /dev/zero
        printf 'ARMd'
        head -c 1000 /dev/zero
        printf '%s\0*** Booting Zephyr OS build selftest ***\0' "$3"
    } > "$1"
}
mkdir -p "$S/img"
mkimg "$S/img/pi4.bin" 4194304 'board: PM watchdog resets the SoC ~50 s after boot'
mkimg "$S/img/am62p.bin" 4194304 'rti_arm: armed preload 0xef'
mkimg "$S/img/imx8mp.bin" 4194304 'gc_wdog_arm: armed'
mkimg "$S/img/layer.bin" 4194304 'board: watchdog armed'
mkimg "$S/img/nomarker.bin" 4194304 'nothing arms here'
mkimg "$S/img/huge.bin" $((512 * 1024 * 1024)) 'rti_arm: armed'

# ---- stubs: any of these running is a failure of --dry-run --------------------
mkdir -p "$S/stub"
for c in ssh scp curl ping fuser flock stty sudo kexec; do
    printf '#!/bin/sh\necho "%s $*" >> "%s/touched"\nexit 99\n' "$c" "$S" > "$S/stub/$c"
    chmod +x "$S/stub/$c"
done
dry() { PATH="$S/stub:$PATH" "$B" --dry-run "$@"; }
untouched() { [ ! -s "$S/touched" ]; }

echo "== 1. every verb on every board under --dry-run (stubs record any board, TFTP or bridge access)"
# board:backend:the backend's boot command, as the dry run prints it
COMBOS='pi4:tryboot:sudo reboot "0 tryboot"
am62p:uboot-tftp:sync; sudo -n systemctl reboot
am62p:uboot-mmc:sync; sudo -n systemctl reboot
am62p:kexec-dev:kexec_run.sh
imx8mp:uboot-tftp:sync; sudo -n systemctl reboot
imx8mp:uboot-mmc:sync; sudo -n systemctl reboot
imx8mp:kexec-dev:kexec-zephyr 0x60000000 --crash'
while IFS=: read -r board boot bootcmd; do
    img="$S/img/$board.bin"
    for verb in status install uninstall disarm; do
        out=$(BOARD_BOOT=$boot dry "$board" "$verb" 2>&1)
        check "$board $boot $verb: rc 0" [ $? = 0 ]
    done
    out=$(BOARD_BOOT=$boot dry "$board" probe "$S/logs" 2>&1)
    rc=$?
    check "$board $boot probe: rc 0, DRY board lines, the lock" \
        eval '[ "$rc" = 0 ] && grep -q "^DRY board" <<<"$out" && grep -q "board_lock.sh" <<<"$out"'
    out=$(BOARD_BOOT=$boot dry "$board" run "$img" "$S/logs" 2>&1)
    rc=$?
    check "$board $boot run: rc 0, the image checked, the lock, the boot command '$bootcmd'" \
        eval '[ "$rc" = 0 ] && grep -q "image sha256 $(sha256sum "$img" | cut -c1-16)" <<<"$out" &&
              grep -q "board_lock.sh" <<<"$out" && grep -qF "$bootcmd" <<<"$out"'
    check "$board $boot run: nothing written on the host (no run directory)" [ ! -e "$S/logs" ]
done <<<"$COMBOS"
out=$(dry imx8mp run "$S/img/imx8mp.bin" "$S/logs" 2>&1)
check "imx8mp: the SNVS record cleared and read back before the boot" grep -q "od -A n -t x4 /sys/bus/nvmem" <<<"$out"
out=$(SEL=0x8003 dry imx8mp run "$S/img/imx8mp.bin" "$S/logs" 2>&1)
check "imx8mp: SEL=0x8003 is written as w1 = 53458003" grep -q "00000000 53458003 00000000 00000000" <<<"$out"
out=$(dry pi4 run "$S/img/pi4.bin" "$S/logs" 2>&1)
check "pi4: /boot/firmware free space read, tryboot.txt written and put back" \
    eval 'grep -q "df -B1 --output=avail /boot/firmware" <<<"$out" && [ "$(grep -c "tee /boot/firmware/tryboot.txt" <<<"$out")" = 2 ]'
out=$(NETBOOT_ROUTE=typed dry am62p run "$S/img/am62p.bin" "$S/logs" 2>&1)
check "am62p typed route: nothing written to the env, the line typed" \
    eval 'grep -q "am62p_netboot_uart.py typed" <<<"$out" && ! grep -q "fw_setenv -s" <<<"$out"'
out=$(dry am62p run "$S/img/layer.bin" "$S/logs" 2>&1)
check "the board layer's 'board: watchdog armed' marker is accepted" [ $? = 0 ]
# /dev/pvrgpu went with pvrgpu.ko: no AM62P command may look for it (it was the
# run's holder check and the kexec-dev status line); the run must still reach
# the lock, so an early exit cannot pass this
out=$(dry am62p run "$S/img/am62p.bin" "$S/logs" 2>&1; BOARD_BOOT=kexec-dev dry am62p status 2>&1)
check "am62p: no /dev/pvrgpu command in run or kexec-dev status (pvrgpu.ko retired)" \
    eval '! grep -q "/dev/pvrgpu" <<<"$out" && grep -q "board_lock.sh" <<<"$out"'

# uboot-mmc (docs/standalone-boot.md): U-Boot loads the image from the eMMC,
# where Linux staged it; no TFTP server, no network
for bm in am62p:0:1:/dev/mmcblk0p1 imx8mp:2:1:/dev/mmcblk2p1; do
    IFS=: read -r board mdev mpart mlinux <<<"$bm"
    out=$(BOARD_BOOT=uboot-mmc dry "$board" run "$S/img/$board.bin" "$S/logs" 2>&1)
    check "$board uboot-mmc run: staged in /var/yakogl on $mlinux, read back from the eMMC, no TFTP canary" \
        eval 'grep -q "findmnt -n -v -o SOURCE,TARGET,FSROOT -T /var/yakogl" <<<"$out" &&
              grep -q "cat > /var/yakogl/once-.*\.bin\.part" <<<"$out" && grep -q "drop_caches.*sha256sum /var/yakogl/once-" <<<"$out" &&
              grep -q "mmc_where\|mmc $mdev:$mpart\|$mlinux" <<<"$out$(BOARD_BOOT=uboot-mmc dry "$board" install 2>&1)" &&
              ! grep -q "curl\|canary\|TFTP_ROOT" <<<"$out"'
    check "$board uboot-mmc run: armed yako_src=mmc with the file as U-Boot names it, removed after the cycle" \
        eval 'grep -q "^    yako_src=mmc$" <<<"$out" && grep -q "^    yako_file=/ostree/deploy/torizon/var/yakogl/once-.*\.bin$" <<<"$out" &&
              grep -q "rm -f /var/yakogl/once-" <<<"$out" && ! grep -q "yako_server" <<<"$out"'
    out=$(BOARD_BOOT=uboot-mmc dry "$board" install 2>&1)
    check "$board install: yako_mmc loads with ext4load from mmc $mdev:$mpart; the hook runs yako_\${yako_src}, then the default" \
        eval 'grep -qF "yako_mmc=if ext4load mmc $mdev:$mpart \${yako_addr} \${yako_file}; then" <<<"$out" &&
              grep -qF "run yako_\${yako_src};" <<<"$out" && grep -qF "if test -n \"\${yako_default}\"; then run yako_dflt; fi" <<<"$out" &&
              grep -qF "setexpr yako_default_left \${yako_default_left} - 1; if saveenv; then" <<<"$out"'
    out=$(BOARD_BOOT=uboot-mmc dry "$board" probe "$S/logs" 2>&1)
    check "$board uboot-mmc probe: a real cycle, armed, nothing staged" \
        eval 'grep -q "systemctl reboot" <<<"$out" && grep -q "^    yako_once=probe-" <<<"$out" && ! grep -q "cat > /var/yakogl" <<<"$out"'
    out=$(dry "$board" default "$S/img/$board.bin" 2>&1)
    rc=$?
    check "$board default without a boot count: usage, rc 2" eval '[ "$rc" = 2 ] && ! grep -q "^DRY board" <<<"$out"'
    out=$(dry "$board" default "$S/img/$board.bin" forever 2>&1)
    rc=$?
    check "$board default forever without BOARD_PERSON_PRESENT=1: rc 3, nothing done, a count offered" \
        eval '[ "$rc" = 3 ] && grep -q "BOARD_PERSON_PRESENT=1" <<<"$out" && grep -q "give a boot count" <<<"$out" && ! grep -q "^DRY board" <<<"$out"'
    out=$(dry "$board" default "$S/img/$board.bin" 2 2>&1)
    rc=$?
    check "$board default 2 (no person needed): rc 0, the undo printed before any board command, staged and read back, yako_default and yako_default_left=2 in one write, no reboot" \
        eval '[ "$rc" = 0 ] && [ "$(grep -n "UNDO" <<<"$out" | head -1 | cut -d: -f1)" -lt "$(grep -n "^DRY board" <<<"$out" | head -1 | cut -d: -f1)" ] &&
              grep -q "UNDO, without Linux: none needed: after 2 boots" <<<"$out" &&
              grep -q "cat > /var/yakogl/default.bin.part" <<<"$out" && grep -q "sha256sum /var/yakogl/default.bin" <<<"$out" &&
              grep -A2 "fw_setenv -s" <<<"$out" | grep -q "^    yako_default=/ostree/deploy/torizon/var/yakogl/default.bin$" &&
              grep -A3 "fw_setenv -s" <<<"$out" | grep -q "^    yako_default_left=2$" &&
              grep -q "fw_printenv -n yako_default_left" <<<"$out" && ! grep -q "systemctl reboot" <<<"$out"'
    out=$(BOARD_PERSON_PRESENT=1 dry "$board" default "$S/img/$board.bin" forever 2>&1)
    rc=$?
    check "$board default forever with a person: rc 0, the count cleared, the U-Boot prompt undo printed" \
        eval '[ "$rc" = 0 ] && grep -q "^    yako_default_left=$" <<<"$out" && grep -q "UNDO, without Linux (a person at the U-Boot prompt.*setenv yako_default; saveenv; boot" <<<"$out"'
    out=$(dry "$board" default "$S/img/$board.bin" 10 2>&1)
    rc=$?
    check "$board default 10: rc 2 (1-9: U-Boot's setexpr counts in hex, test -gt in decimal)" eval '[ "$rc" = 2 ]'
    out=$(dry "$board" nodefault 2>&1)
    rc=$?
    check "$board nodefault: yako_default and its count removed in one write and read back, default.bin removed" \
        eval '[ "$rc" = 0 ] && grep -q "^    yako_default=$" <<<"$out" && grep -q "^    yako_default_left=$" <<<"$out" && grep -q "rm -f /var/yakogl/default.bin" <<<"$out"'
    out=$(BOARD_BOOT=uboot-mmc dry "$board" run "$S/img/$board.bin" "$S/logs" 2>&1; BOARD_BOOT=uboot-tftp dry "$board" run "$S/img/$board.bin" "$S/logs" 2>&1)
    check "$board run (both U-Boot backends) checks that no default is set" \
        eval '[ "$(grep -c "yako_once and yako_default must be absent" <<<"$out")" = 2 ]'
done
# a real uboot-mmc user sets no NETBOOT_* at all (the dry run fills them in
# only for uboot-tftp): every verb of the backend must run under set -u
# without them (2026-10-04: the AM62P probe died on NETBOOT_SERVER)
for board in am62p imx8mp; do
    out=$(for v in "probe $S/logs" "run $S/img/$board.bin $S/logs" status install uninstall disarm nodefault "default $S/img/$board.bin 2"; do
        # shellcheck disable=SC2086  # the verb and its arguments
        env -u NETBOOT_SERVER -u NETBOOT_TFTP_ROOT -u NETBOOT_TFTP_LOG -u NETBOOT_ROUTE BOARD_BOOT=uboot-mmc \
            PATH="$S/stub:$PATH" "$B" --dry-run "$board" $v > "$S/mmc-noenv.out" 2>&1
        echo "rc=$? $v"
        grep -E "unbound variable|NETBOOT" "$S/mmc-noenv.out"
    done)
    check "$board uboot-mmc, every verb with no NETBOOT_* set: rc 0 each, no unbound variable, no NETBOOT name printed" \
        eval '[ "$(grep -c "^rc=0 " <<<"$out")" = 8 ] && ! grep -q "unbound variable\|NETBOOT" <<<"$out"'
done
out=$(NETBOOT_ROUTE=typed BOARD_BOOT=uboot-mmc dry am62p run "$S/img/am62p.bin" "$S/logs" 2>&1)
rc=$?
check "am62p uboot-mmc refuses the typed route (rc 2)" eval '[ "$rc" = 2 ] && grep -q "typed is a TFTP route" <<<"$out"'
out=$(dry pi4 default "$S/img/pi4.bin" 2 2>&1)
rc=$?
check "pi4 default with a boot count: rc 2 (the firmware has none)" eval '[ "$rc" = 2 ] && grep -q "forever. only" <<<"$out"'
out=$(dry pi4 default "$S/img/pi4.bin" forever 2>&1)
rc=$?
check "pi4 default forever without BOARD_PERSON_PRESENT=1: rc 3" eval '[ "$rc" = 3 ] && grep -q BOARD_PERSON_PRESENT <<<"$out"'
out=$(BOARD_PERSON_PRESENT=1 BOARD_HDMI_TIMINGS="1 2 3" dry pi4 default "$S/img/pi4.bin" forever 2>&1)
rc=$?
check "pi4 default forever: rc 0, the SD-card undo printed first, config.txt kept as config-linux.txt, config.txt names yakogl-default.bin, no reboot" \
    eval '[ "$rc" = 0 ] && grep -q "UNDO, without Linux (a person): the SD card" <<<"$out" &&
          grep -q "cp /boot/firmware/config.txt /boot/firmware/config-linux.txt" <<<"$out" &&
          grep -q "sha256sum /boot/firmware/yakogl-default.bin" <<<"$out" && grep -q "tee /boot/firmware/config.txt" <<<"$out" &&
          ! grep -q "0 tryboot" <<<"$out"'
# the default's config.txt and a tryboot's tryboot.txt are the same lines (pi4_config), past the comment
eval "$(sed -n '/^pi4_config() {/,/^}/p' "$B")"
BOARD_HDMI_TIMINGS="1 2 3"
check "pi4_config: [all], the kernel, arm_64bit, the UART, and the panel's hdmi lines when set" \
    eval '[ "$(pi4_config k.bin | tr "\n" "|")" = "[all]|kernel=k.bin|arm_64bit=1|enable_uart=1|uart_2ndstage=1|hdmi_group=2|hdmi_mode=87|hdmi_timings=1 2 3|" ]'
out=$(dry pi4 nodefault 2>&1)
rc=$?
check "pi4 nodefault: config-linux.txt put back as config.txt, the image removed" \
    eval '[ "$rc" = 0 ] && grep -q "mv /boot/firmware/config-linux.txt /boot/firmware/config.txt" <<<"$out"'
out=$(dry pi4 run "$S/img/pi4.bin" "$S/logs" 2>&1)
check "pi4 tryboot run refuses when a default is set (config-linux.txt)" grep -q "test ! -e /boot/firmware/config-linux.txt" <<<"$out"

# tree: the three table forms
mkdir -p "$S/pi4img/AA/zephyr" "$S/pi4img/BB/zephyr" "$S/amimg/img1-t/zephyr" "$S/amimg/img2-t/zephyr"
cp "$S/img/pi4.bin" "$S/pi4img/AA/zephyr/zephyr.bin"
cp "$S/img/pi4.bin" "$S/pi4img/BB/zephyr/zephyr.bin"
cp "$S/img/am62p.bin" "$S/amimg/img1-t/zephyr/zephyr.bin"
cp "$S/img/am62p.bin" "$S/amimg/img2-t/zephyr/zephyr.bin"
printf '# a table\n# image AA: one\ndEQP-GLES2.functional.a.*\n\n# image BB: two\ndEQP-GLES2.functional.b.*\n' > "$S/pi4.txt"
printf '# a table\ndEQP-GLES2.functional.a.*\ndEQP-GLES2.functional.b.*\n' > "$S/am.txt"
out=$(TREE_IMAGE_DIR=$S/pi4img dry pi4 tree "$S/pi4.txt" "$S/tree" 2>&1)
rc=$?
check "pi4 tree (# image labels): rc 0, rows AA and BB each booted" \
    eval '[ "$rc" = 0 ] && grep -q "^=== AA " <<<"$out" && grep -q "^=== BB " <<<"$out" && [ "$(grep -c "0 tryboot\"$" <<<"$out")" = 2 ]'
out=$(TREE_IMAGE_DIR=$S/amimg TREE_TAG=t dry am62p tree "$S/am.txt" "$S/tree" 2>&1)
rc=$?
check "am62p tree (img<N>-<tag>): rc 0, rows img1-t and img2-t each booted" \
    eval '[ "$rc" = 0 ] && grep -q "^=== img1-t " <<<"$out" && grep -q "^=== img2-t " <<<"$out"'
out=$(TREE_IMAGE=$S/img/imx8mp.bin TREE_ROWS=1 dry imx8mp tree "$S/am.txt" "$S/tree" 2>&1)
rc=$?
check "imx8mp tree (one image, selector): rc 0, TREE_ROWS=1 runs only e01, with selector 1" \
    eval '[ "$rc" = 0 ] && grep -q "^=== e01 .*selector 1" <<<"$out" && ! grep -q "^=== e00" <<<"$out" && grep -q "53450001" <<<"$out"'
check "tree wrote nothing on the host" [ ! -e "$S/tree" ]

# The baseline (tools/cts/cts-baseline.sh, tools/cts/baseline-tree.txt): the
# five labelled rows on every board, through the same tree runner.
CB="$HERE/cts/cts-baseline.sh"
for lab in BI BP BN BQ BF; do
    # each board's own image (its watchdog marker), in a labelled directory
    # with the builder's verdict, as am62p-deqp-images.sh and
    # pi4-deqp-images.sh leave them
    mkdir -p "$S/amb/$lab/zephyr" "$S/pib/$lab/zephyr"
    cp "$S/img/am62p.bin" "$S/amb/$lab/zephyr/zephyr.bin"
    cp "$S/img/pi4.bin" "$S/pib/$lab/zephyr/zephyr.bin"
    echo "$lab rc=0 sha256=0 build=none" > "$S/amb/$lab.built"
    echo "$lab rc=0 sha256=0 build=none" > "$S/pib/$lab.built"
done
mkdir -p "$S/imb/baseline/zephyr" && cp "$S/img/imx8mp.bin" "$S/imb/baseline/zephyr/zephyr.bin"
out=$("$HERE/cts/am62p-deqp-images.sh" --dry-run "$HERE/cts/baseline-tree.txt" "$S/amb" t 2>&1)
check "baseline table: the AM62P builder names the five rows by label (BI BP BN BQ BF)" \
    eval '[ "$(cut -d" " -f2 <<<"$out" | tr "\n" " ")" = "BI BP BN BQ BF " ]'
out=$("$HERE/cts/am62p-deqp-images.sh" --dry-run "$HERE/cts/am62p-full-tree.txt" "$S/amb" t 2>&1)
check "an unlabelled table keeps img<N>-<tag> (am62p-full-tree.txt, 17 rows)" \
    eval '[ "$(wc -l <<<"$out")" = 17 ] && [ "$(head -1 <<<"$out" | cut -d" " -f2)" = img1-t ] && [ "$(tail -1 <<<"$out" | cut -d" " -f2)" = img17-t ]'
out=$("$HERE/cts/am62p-deqp-images.sh" --dry-run "$HERE/cts/am62p-ms4-tree.txt" "$S/amb" t 2>&1)
check "the AM62P multisample table: one MS4 image carrying the rgba8888d24s8ms4 render config" \
    eval '[ "$out" = "1 MS4 dEQP-GLES2.functional.multisample.* [DEQP_GL_CONFIG_NAME=rgba8888d24s8ms4]" ]'
check "dEQP image recipes: symbols, conformance options, carve-outs, tables (check-deqp-images.py)" \
    python3 -I "$HERE/cts/check-deqp-images.py"
check "check-deqp-images.py self-test" python3 -I "$HERE/cts/check-deqp-images.py" --self-test
out=$(PATH="$S/stub:$PATH" "$CB" --dry-run am62p run "$S/amb" "$S/tree" 2>&1)
rc=$?
check "baseline am62p run: rc 0, the five labelled rows each booted" \
    eval '[ "$rc" = 0 ] && for l in BI BP BN BQ BF; do grep -q "^=== $l " <<<"$out" || exit 1; done'
out=$(PATH="$S/stub:$PATH" "$CB" --dry-run pi4 run "$S/pib" "$S/tree" 2>&1)
rc=$?
check "baseline pi4 run: rc 0, the five labelled rows each booted by tryboot" \
    eval '[ "$rc" = 0 ] && [ "$(grep -c "0 tryboot\"$" <<<"$out")" = 5 ] && grep -q "^=== BF " <<<"$out"'
out=$(PATH="$S/stub:$PATH" "$CB" --dry-run imx8mp run "$S/imb" "$S/tree" 2>&1)
rc=$?
check "baseline imx8mp run: rc 0, one image, rows e00..e04 by selector, none past e04" \
    eval '[ "$rc" = 0 ] && grep -q "^=== e00 " <<<"$out" && grep -q "^=== e04 .*selector 4" <<<"$out" && ! grep -q "^=== e05" <<<"$out"'
check "the baseline dry runs wrote nothing on the host" [ ! -e "$S/tree" ]
# The common set (tools/cts/cts-common.sh, tools/cts/common-tree.txt): five
# labelled rows, one image each, on every board through the same tree runner.
CC="$HERE/cts/cts-common.sh"
CT="$HERE/cts/common-tree.txt"
for b in am62p pi4 imx8mp; do
    for lab in GM SO SV CL GT; do
        mkdir -p "$S/cm-$b/$lab/zephyr"
        # each row its own bytes, so its run directory's sha8 names the image it booted
        { cat "$S/img/$b.bin"; printf '%s' "$lab"; } > "$S/cm-$b/$lab/zephyr/zephyr.bin"
        echo "$lab rc=0 sha256=0 build=none" > "$S/cm-$b/$lab.built"
    done
done
out=$("$HERE/cts/common-images.sh" --dry-run pi4 "$CT" "$S/cmb" 2>&1)
check "common table: the builder names GM SO SV CL GT, the scan-out view on SV only, GT is v3d_gl_test on the Pi 4" \
    eval '[ "$(cut -d" " -f1 <<<"$out" | tr "\n" " ")" = "GM SO SV CL GT " ] && [ "$(grep -c SCANOUT_VIEW=y <<<"$out")" = 1 ] &&
          grep -q "^SV slint .*SCANOUT_VIEW=y" <<<"$out" && grep -q "^GT v3d_gl_test " <<<"$out"'
out=$("$HERE/cts/common-images.sh" --dry-run am62p "$CT" "$S/cmb" 2>&1)
check "common builder: every AM62P row self-arms the RTI without the inherited feeder; GT is gl_test" \
    eval '[ "$(grep -c "RTI_SELF_ARM=y -DCONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=n" <<<"$out")" = 5 ] && grep -q "^GT gl_test " <<<"$out"'
out=$("$HERE/cts/common-images.sh" --dry-run imx8mp "$CT" "$S/cmb" 2>&1)
check "common builder: every i.MX8MP row self-arms WDOG1; GT is gc_gl_test" \
    eval '[ "$(grep -c "GC_BOARD_WDOG_SELF_ARM=y" <<<"$out")" = 5 ] && grep -q "^GT gc_gl_test " <<<"$out"'
check "common builder: the CL row builds the cluster sample with its UI and Slint inputs named" \
    eval 'grep -q "^CL cluster " <<<"$out" && grep -q "\-DAGL_SLINT_CLUSTER_DIR=" <<<"$out" &&
          grep -q "\-DSLINT_NOSTD_DIR=" <<<"$out"'
printf '# image XX: x\nfoo\n' > "$S/cm-bad.txt"
printf 'glmark2\n' > "$S/cm-nolabel.txt"
out=$("$HERE/cts/common-images.sh" --dry-run pi4 "$S/cm-bad.txt" "$S/cmb" 2>&1)
out2=$("$HERE/cts/common-images.sh" --dry-run pi4 "$S/cm-nolabel.txt" "$S/cmb" 2>&1)
rc=$?
check "common builder: an unknown row is named, an unlabelled row refused (rc 2)" \
    eval 'grep -q "^XX: .foo. is not glmark2" <<<"$out" && [ "$rc" = 2 ] && grep -q "without a label" <<<"$out2"'
for b in am62p pi4 imx8mp; do
    out=$(PATH="$S/stub:$PATH" "$CC" --dry-run "$b" run "$S/cm-$b" "$S/tree" 2>&1)
    rc=$?
    check "common $b run: rc 0, the five labelled rows each booted from their own directory" \
        eval '[ "$rc" = 0 ] && [ "$(grep -cE "^=== (GM|SO|SV|CL|GT) \\(zephyr.bin" <<<"$out")" = 5 ] &&
              [ "$(for l in GM SO SV CL GT; do grep -c "^DRY host\$ mkdir -p .*-$l-$(sha256sum "$S/cm-$b/$l/zephyr/zephyr.bin" | cut -c1-8) " <<<"$out"; done | tr "\n" " ")" = "1 1 1 1 1 " ]'
done
check "the common dry runs wrote nothing on the host" eval '[ ! -e "$S/tree" ] && [ ! -e "$S/cmb" ]'
# board.sh refuses an AM62P image without "rti_arm: armed"; the dEQP sample's
# AM62P images (the baseline's and the full tree's) must arm the RTI themselves
check "the dEQP sample's AM62P config arms the RTI itself (not the inherited feeder)" \
    eval 'f="$REPO/driver/samples/zephyr/deqp/boards/verdin_am62p_am62p54_a53.conf";
          grep -qx "CONFIG_YAKOGL_AM62P_RTI_SELF_ARM=y" "$f" && ! grep -qx "CONFIG_SOC_AM62PX_A53_FEED_INHERITED_WDT=y" "$f"'
check "the dEQP sample's i.MX8MP config arms WDOG1 itself (board.sh needs gc_wdog_arm: armed)" \
    eval 'f="$REPO/driver/samples/zephyr/deqp/boards/verdin_imx8mp_mimx8ml8_a53.conf";
          grep -qx "CONFIG_GC_BOARD_WDOG_SELF_ARM=y" "$f" && ! grep -qx "CONFIG_GC_SAMPLE_WDOG_TIMER_FEED=y" "$f"'
check "no stub ran in any dry run (no board, TFTP root or bridge touched)" untouched
[ -s "$S/touched" ] && sed 's/^/        /' "$S/touched"

echo "== 2. refusals that need no board"
out=$(dry am62p run "$S/img/nomarker.bin" "$S/logs" 2>&1)
rc=$?
check "an image without the watchdog marker: rc 3, REFUSED" eval '[ "$rc" = 3 ] && grep -q REFUSED <<<"$out"'
out=$(NETBOOT_NO_WATCHDOG=1 dry am62p run "$S/img/nomarker.bin" "$S/logs" 2>&1)
rc=$?
check "NETBOOT_NO_WATCHDOG=1 lets it through, with a WARNING" eval '[ "$rc" = 0 ] && grep -q WARNING <<<"$out"'
out=$(dry am62p run "$S/img/huge.bin" "$S/logs" 2>&1)
rc=$?
check "an image past the RAM window: rc 3" eval '[ "$rc" = 3 ] && grep -q "does not fit" <<<"$out"'
out=$(BOARD_BOOT=kexec-dev dry pi4 status 2>&1)
check "pi4 has no kexec-dev: rc 2" [ $? = 2 ]
out=$(dry rpi5 status 2>&1)
check "an unknown board: rc 2" [ $? = 2 ]
out=$(dry pi4 run "$S/img/pi4.bin" "$S/logs" 'bad name' 2>&1)
check "a run name with a space: rc 2" [ $? = 2 ]
check "no stub ran" untouched

echo "== 2b. the copies' redaction (board.sh's own redact_dir, read from its source)"
eval "$(sed -n '/^redact_dir() {/,/^}/p' "$B")"
mkdir -p "$S/red"
printf '%s\n' 'DHCP client bound to address 10.11.12.13 (785 ms)' \
    'TFTP from server 10.11.12.1; our IP address is 10.11.12.13' 'verdin-imx8mp-00000000 login:' \
    'eth0 aa:bb:cc:dd:ee:0f up' 'twice 172.16.0.9 172.16.0.9' 'keep 192.0.2.7, U-Boot 2024.04 and pi4' > "$S/red/console.txt"
printf 'raw\0bytes 10.11.12.13\0\n' > "$S/red/console.raw"
printf '%s\n' 'DHCP client bound to address 192.0.2.1 (785 ms)' \
    'TFTP from server 192.0.2.2; our IP address is 192.0.2.1' '<board-host> login:' \
    'eth0 02:00:00:00:00:01 up' 'twice 192.0.2.3 192.0.2.3' 'keep 192.0.2.7, U-Boot 2024.04 and pi4' > "$S/red-want.txt"
redact_dir "$S/red" verdin-imx8mp-00000000 pi4
same "IPv4 -> 192.0.2.n per address, MAC, host name (>= 6 characters, whole word); 192.0.2.x, versions and 'pi4' kept" \
    "$S/red-want.txt" "$S/red/console.txt"
check "the binary capture is redacted too, NULs kept" eval '[ "$(tr -dc "\\000" < "$S/red/console.raw" | wc -c)" = 2 ] && grep -aq "192.0.2.1" "$S/red/console.raw"'

echo "== 2b1. one gl_test case run alone is a result (board.sh single_case)"
# gl_test_run's start line names the case: N >= 0 and not 7 is one case, whose
# own PASS/FAIL/TIMEOUT line is the verdict; the set (7) and a list (-1) are not
eval "$(sed -n '/^plain() /p;/^single_case() {/,/^}/p' "$B")"
sc_con() { printf '11:39:10 gl_test: start (case=%s pretransform=0 ppp_ctrl=0x0 debug=0 no_coalesce=0)\n' "$1"; shift; printf '11:39:10 %s\n' "$@"; }
sc_con 176 "gl_test: case 176 mapbuf: PASS" "gl_test: OK (rc=0)" > "$S/sc-pass.txt"
sc_con 176 "gl_test: case 176 mapbuf: FAIL" "gl_test: FAILED (rc=-1)" > "$S/sc-fail.txt"
sc_con 25 "gl_test: case 25 stress: TIMEOUT" > "$S/sc-timeout.txt"
sc_con 7 "gl_test: case 1 uniform: PASS" > "$S/sc-setdied.txt"
sc_con -1 "gl_test: case 5 scissor: PASS" > "$S/sc-list.txt"
sc_con 176 "gl_test: case 17 ctx2: PASS" > "$S/sc-other.txt"
check "single_case: PASS is 'pass 176', FAIL and TIMEOUT are fail" \
    eval '[ "$(single_case "$S/sc-pass.txt")" = "pass 176" ] && [ "$(single_case "$S/sc-fail.txt")" = "fail 176" ] &&
          [ "$(single_case "$S/sc-timeout.txt")" = "fail 25" ]'
# every occurrence is read (v3d_gl_test's frozen record replays the live
# lines): a FAIL anywhere is fail, whichever copy comes last
sc_con 169 "gl_test: case 169 dstex: FAIL" "v3d_gl_test RECORD, build 0123abcd, 0 s after boot: DONE" \
    "gl_test: case 169 dstex: PASS" "v3d_gl_test RECORD end, build 0123abcd" > "$S/sc-failpass.txt"
sc_con 169 "gl_test: case 169 dstex: PASS" "v3d_gl_test RECORD, build 0123abcd, 0 s after boot: DONE" \
    "gl_test: case 169 dstex: FAIL" "v3d_gl_test RECORD end, build 0123abcd" > "$S/sc-passfail.txt"
sc_con 169 "gl_test: case 169 dstex: PASS" "v3d_gl_test RECORD, build 0123abcd, 0 s after boot: DONE" \
    "gl_test: case 169 dstex: PASS" "v3d_gl_test RECORD end, build 0123abcd" > "$S/sc-passpass.txt"
check "single_case: a live FAIL and a replayed PASS, and the inverse, are fail; PASS twice is pass" \
    eval '[ "$(single_case "$S/sc-failpass.txt")" = "fail 169" ] && [ "$(single_case "$S/sc-passfail.txt")" = "fail 169" ] &&
          [ "$(single_case "$S/sc-passpass.txt")" = "pass 169" ]'
check "single_case: the set that died after case 1, a list run, another case's line, no console: no result" \
    eval '[ -z "$(single_case "$S/sc-setdied.txt")" ] && [ -z "$(single_case "$S/sc-list.txt")" ] &&
          [ -z "$(single_case "$S/sc-other.txt")" ] && [ -z "$(single_case "$S/none")" ]'
# through board.sh's own verdict: a Pi 4 tryboot run that ended (v3d_gl_test's record end line)
for v in pass fail; do
    d="$S/scrun-$v/20261004-1200-c176-0123abcd"
    mkdir -p "$d"
    printf 'board=pi4\nboot=tryboot\nname=c176\nprobe=0\nback=1\nt_reboot=100\nt_back=200\n' > "$d/meta.txt"
    { cat "$S/sc-$v.txt"; echo "11:39:11 v3d_gl_test RECORD end, build 0123abcd"; } > "$d/console.txt"
done
check "board.sh tally: a single case's PASS is verdict done, its FAIL verdict failed, both result=legacy:gl_test_case" \
    eval 'says pi4 "$S/scrun-pass/20261004-1200-c176-0123abcd" "rc=0 verdict=done .*result=legacy:gl_test_case" &&
          says pi4 "$S/scrun-fail/20261004-1200-c176-0123abcd" "rc=4 verdict=failed .*result=legacy:gl_test_case"'
# the one verdict rule for a set or list (board.sh gl_test_set_verdict): any
# failed or timed-out case is failed; every case passed is done
eval "$(sed -n '/^gl_test_set_verdict() {/,/^}/p' "$B")"
printf '10:00:01 gl_test: all: 0 of 40 cases failed (40 passed; 0 timed out; first failure case -1 at index -1)\n' > "$S/set-pass.txt"
printf '10:00:01 gl_test: all: 1 of 40 cases failed (39 passed; 0 timed out; first failure case 48 at index 31)\n' > "$S/set-fail.txt"
printf '10:00:01 gl_test: all: 0 of 40 cases failed (39 passed; 1 timed out; first failure case -1 at index -1)\n' > "$S/set-timeout.txt"
check "gl_test_set_verdict: 0 failed is done; 1 failed, or 1 timed out alone, is failed; no all: line is done (the result kind decides)" \
    eval '[ "$(gl_test_set_verdict "$S/set-pass.txt")" = done ] && [ "$(gl_test_set_verdict "$S/set-fail.txt")" = failed ] &&
          [ "$(gl_test_set_verdict "$S/set-timeout.txt")" = failed ] && [ "$(gl_test_set_verdict "$S/none")" = done ]'
echo "== 2b2. the i.MX8MP crash route's readiness (board.sh imx8mp_kexec_ready)"
# what the fixed waits stood for, read from the board: systemd settled, the M7
# offline, no jump in progress, no client, the crash window; the uptime is not
# a gate (a board 12 s up whose boot has finished is ready)
eval "$(sed -n '/^imx8mp_kexec_ready() {/,/^}/p' "$B")"
footprint=1000
check "imx8mp readiness: settled at uptime 12 s is ready (no 180 s floor); 'degraded' too" \
    eval 'imx8mp_kexec_ready running 12.31 offline 0 0 268435456 0 && imx8mp_kexec_ready degraded 12.31 offline 0 0 268435456 0'
check "imx8mp readiness refuses: systemd starting, the M7 running, a jump loaded or running, a client, a small or absent crash window" \
    eval '! imx8mp_kexec_ready starting 999.0 offline 0 0 268435456 0 >/dev/null &&
          ! imx8mp_kexec_ready running 999.0 running 0 0 268435456 0 >/dev/null &&
          ! imx8mp_kexec_ready running 999.0 offline 0 0 268435456 1 >/dev/null &&
          ! imx8mp_kexec_ready running 999.0 offline 1 0 268435456 0 >/dev/null &&
          ! imx8mp_kexec_ready running 999.0 offline 0 1 268435456 0 >/dev/null &&
          ! imx8mp_kexec_ready running 999.0 offline 0 0 999 0 >/dev/null &&
          ! imx8mp_kexec_ready running 999.0 offline 0 0 "" 0 >/dev/null &&
          [ "$(imx8mp_kexec_ready starting 1 offline 0 0 268435456 0)" = "systemd is '\''starting'\'', not settled (is-system-running)" ]'
out=$(BOARD_BOOT=kexec-dev dry imx8mp run "$S/img/imx8mp.bin" "$S/logs" 2>&1)
check "imx8mp kexec-dev dry run: one readiness read, no uptime floor, no wait on the staged image's age" \
    eval 'grep -q "systemctl is-system-running --wait" <<<"$out" && grep -q "kexec_crash_loaded" <<<"$out" &&
          ! grep -q "older than 200\|stat -c %Y" <<<"$out"'
echo "== 2c. the image's build id and the build line it printed (board.sh's own functions)"
eval "$(sed -n '/^plain() /p;/^image_build() {/,/^}/p;/^boot_build() {/,/^}/p' "$B")"
mkdir -p "$S/bd/zephyr"
cp "$S/img/am62p.bin" "$S/bd/zephyr/zephyr.bin"
printf 'build x: phony\n  DEFINES = -DPVR_SAMPLE_TREE=\\"71145c16\\" -DPVR_SAMPLE_TABLE=\\"1449eb0f\\"\n' > "$S/bd/build.ninja"
check "image_build: the ids of the build.ninja beside the image" \
    eval '[ "$(image_build "$S/bd/zephyr/zephyr.bin")" = "PVR_SAMPLE_TABLE=1449eb0f PVR_SAMPLE_TREE=71145c16" ]'
check "image_build without a build.ninja: absent with the reason, never empty" \
    eval '[ "$(image_build "$S/img/am62p.bin")" = "absent (no build id in a build.ninja beside the image)" ]'
check "boot_build: the line a real capture printed at boot" \
    eval 'boot_build "$TD/am62p-netboot-rtiproof-20261003.uart.ts" | grep -q "sample: build tree=[0-9a-f]\{8\} table="'
# the Pi 4 firmware's last line has no newline, so the first Zephyr line (the
# board layer's PRE_KERNEL_2 build line) arrives glued to it (the common set's
# Pi 4 consoles, 2026-10-04); a printf format in the console is no build line
printf '08:12:29 MESS:00:00:06.208753:0: uart: Baud rate change done...\n08:12:29 MESS:00:00:06.210776:0:sample: build tree=abcdef12 table=12345678\n08:12:29 board: watchdog armed\n' > "$S/glued.txt"
printf '08:12:29 sample: build tree=%%s table=%%s\n08:12:29 slint: frame 1 (build + default state): rc 0, ok\n' > "$S/fmt.txt"
check "boot_build: a build line glued to the firmware's last line is found; a format string or '(build + ...' is not one" \
    eval '[ "$(boot_build "$S/glued.txt")" = "sample: build tree=abcdef12 table=12345678" ] &&
          [ "$(boot_build "$S/fmt.txt")" = "absent (the console has no build line)" ]'
# The AM62P's GPU recoveries (pvr_resets, driver/rust/src/gpu/pvr/render.rs): the
# run-end summary first (its N, not the raw counter C), else the HWR lines' last B
# minus the first A (one line may cover several recoveries), else 0 only for a run
# that reached its own end, absent for one cut short
eval "$(sed -n '/^legacy_result() {/,/^}/p;/^pvr_resets() {/,/^}/p;/^ytools() {/,/^}/p' "$B")"
LEGACY_RE='^(deqp: summary: |gl_test: all: )'
END_GT='09:00:09 gl_test: all: 0 of 40 cases failed (40 passed; 0 timed out; first failure case -1 at index -1)'
printf '09:00:01 E: render: HWR during fence 46999 (counter 0 -> 2)\n%s\n' "$END_GT" > "$S/hwr2.txt"
printf '09:00:01 E: render: HWR during fence 100 (counter 0 -> 1)\n09:00:08 I: render freelists: 3 grow requests, 2 served, 1 unserved, 2072 KiB taken on demand; GPU recoveries 3 (HWR counter 3)\n%s\n' "$END_GT" > "$S/hwrsum.txt"
printf '09:00:01 E: render: HWR during fence 7 (counter 4 -> 5)\n09:00:02 E: render: HWR during fence 9 (counter 5 -> 6)\n' > "$S/hwrbase.txt"
printf '09:00:08 I: render freelists: 0 grow requests, 0 served, 0 unserved, 0 KiB taken on demand; GPU recoveries 2 (HWR counter 5)\n' > "$S/sumbase.txt"
printf '09:00:01 glmark2: window 800x600\n09:00:20 W: rti_arm: the sample stopped saying it is alive; the RTI resets the SoC\n' > "$S/cut.txt"
printf '09:00:01 glmark2: window 800x600\n09:00:02 *** ZEPHYR FATAL ERROR 0: CPU exception\n' > "$S/fatal.txt"
check "AM62P resets: a 'counter 0 -> 2' line reads 2 (one line, two recoveries)" eval '[ "$(pvr_resets "$S/hwr2.txt")" = 2 ]'
check "AM62P resets: the run-end summary wins over an earlier counter line (3, not 1)" eval '[ "$(pvr_resets "$S/hwrsum.txt")" = 3 ]'
check "AM62P resets: a counter that started above 0 counts from its start (4 -> 6: 2), the summary's N not its C (2, not 5)" \
    eval '[ "$(pvr_resets "$S/hwrbase.txt")" = 2 ] && [ "$(pvr_resets "$S/sumbase.txt")" = 2 ]'
check "AM62P resets: 0 only for a run that reached its end; cut short or fatal is absent" \
    eval 'printf "%s\n" "$END_GT" > "$S/end.txt"; [ "$(pvr_resets "$S/end.txt")" = 0 ] &&
          [ "$(pvr_resets "$S/cut.txt")" = absent ] && [ "$(pvr_resets "$S/fatal.txt")" = absent ] && [ "$(pvr_resets "$S/none")" = absent ]'
check "boot_build: absent when the console has none, and without a console" \
    eval '[ "$(boot_build "$TD/pi4-tryboot-TF-29460f96.console.txt")" = "absent (the console has no build line)" ] &&
          boot_build "$S/none" | grep -q "^absent (no console"'

# ---- the replays ------------------------------------------------------------------
# mkrun DIR KEY=VALUE...: a run directory's meta.txt, as tools/board.sh run leaves it
mkrun() {
    local d=$1
    shift
    mkdir -p "$d"
    printf '%s\n' "$@" > "$d/meta.txt"
}

echo "== 3a. AM62P U-Boot captures: board.sh's verdict against the old phase scan and probe rule"
while read -r cap token probe; do
    if [ "$probe" = 1 ]; then d="$S/am62p/20261003-1200-$cap-probe"; else d="$S/am62p/20261003-1200-$cap-0123abcd"; fi
    mkrun "$d" board=am62p boot=uboot-tftp "name=$cap" "token=$token" "probe=$probe" back=1 t_reboot=100 t_back=170
    cp "$TD/am62p-netboot-$cap-20261003.uart.ts" "$d/console.ts"
    oldline=$(python3 "$HERE/am62p_netboot_uart.py" scan "$d/console.ts" --token "$token")
    old=$(field verdict <<<" $oldline")
    new=$("$B" am62p tally "$d" | head -1)
    rcn=$(field rc <<<"$new")
    if [ "$probe" = 1 ]; then
        # the old AM62P runner's probe rule (its "run|probe" step 9): the probe
        # passes (rc 0) only on fetch_failed with the DHCP and "File not found" lines
        oldrc=4
        if [ "$old" = fetch_failed ] && grep -aq 'DHCP client bound' "$d/console.ts" && grep -aq 'File not found' "$d/console.ts"; then oldrc=0; fi
        check "$cap: old verdict=$old rc=$oldrc; new boot_verdict=$(field boot_verdict <<<"$new") verdict=$(field verdict <<<"$new") rc=$rcn" \
            eval '[ "$(field boot_verdict <<<"$new")" = "$old" ] && [ "$rcn" = "$oldrc" ] && [ "$(field verdict <<<"$new")" = probe_ok ]'
    else
        # the boot verdict is the old runner's; the run's verdict is not: both
        # captures end "gl_test: all: 1 of 38 cases failed", which the old
        # runner called done and the one verdict rule (gl_test_set_verdict,
        # 2026-10-04) calls failed, as the i.MX8MP's CASE record of a set does
        check "$cap: old verdict=$old; new boot_verdict=$(field boot_verdict <<<"$new"), verdict=$(field verdict <<<"$new") (1 of 38 failed) result=$(field result <<<"$new") rc=$rcn" \
            eval '[ "$(field boot_verdict <<<"$new")" = "$old" ] && [ "$old" = done ] && [ "$(field verdict <<<"$new")" = failed ] &&
                  [ "$rcn" = 4 ] && [ "$(field result <<<"$new")" = legacy:gl_test_all ]'
    fi
    echo "        old: ${oldline:0:110}..."
    echo "        new: $new"
done <<'CAPS'
rtiproof rtiproof-691c3e985b1f068d-1791013366 0
selfarm selfarmC-f047d34c0b11cb05-1791013634 0
probe probe-none-1791009398 1
CAPS

# the same captures through the eMMC route: U-Boot's ext4load line in place of
# the TFTP lines (the rest of each capture unchanged)
d="$S/am62pm/20261003-1200-selfarm-0123abcd"
mkrun "$d" board=am62p boot=uboot-mmc name=selfarm token=selfarmC-f047d34c0b11cb05-1791013634 probe=0 back=1 t_reboot=100 t_back=170
# (lines 55-72: the network bring-up, DHCP and TFTP between "cleared" and the byte count)
sed -E '55,72d; s/ Bytes transferred = ([0-9]+) .*/ \1 bytes read in 360 ms (43 MiB\/s)/' \
    "$TD/am62p-netboot-selfarm-20261003.uart.ts" > "$d/console.ts"
check "am62p uboot-mmc, the selfarm capture with ext4load's line: boot_verdict done, verdict failed (1 of 38), as over TFTP" \
    eval 'grep -q " 16234368 bytes read in " "$d/console.ts" && ! grep -q "TFTP from" "$d/console.ts" &&
          says am62p "$d" "rc=4 verdict=failed boot=uboot-mmc row boot_verdict=done result=legacy:gl_test_all"'
d="$S/am62pm/20261003-1200-probe-probe"
mkrun "$d" board=am62p boot=uboot-mmc name=probe token=probe-none-1791009398 probe=1 back=1 t_reboot=100 t_back=170
sed -E '55,73d; s/yakogl-netboot: fetch failed, booting Linux/yakogl-netboot: mmc load failed, booting Linux/' \
    "$TD/am62p-netboot-probe-20261003.uart.ts" > "$d/console.ts"
check "am62p uboot-mmc probe: U-Boot's 'mmc load failed' line -> boot_verdict load_failed, probe_ok rc 0" \
    eval 'grep -q "mmc load failed" "$d/console.ts" && ! grep -q "DHCP client" "$d/console.ts" &&
          says am62p "$d" "rc=0 verdict=probe_ok boot=uboot-mmc row boot_verdict=load_failed"'
sed -i 's/mmc load failed/fetch failed/' "$d/console.ts"
check "am62p uboot-mmc probe without the 'mmc load failed' line: probe_failed (TFTP's line is not evidence for the eMMC)" \
    says am62p "$d" "verdict=probe_failed"
d="$S/imxm/20261003-1200-m1-0123abcd"
mkrun "$d" board=imx8mp boot=uboot-mmc name=m1 probe=0 back=1 up=40 t_reboot=100 t_back=200 "rec0=00000000 00000000 00000000 00000000"
echo "594b0202 00000001 00000001 00000000" > "$d/snvs.txt"
check "imx8mp uboot-mmc: an APP pass record after the cycle -> done, a row, boot_verdict cycled, no tftp field" \
    eval 'says imx8mp "$d" "rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:APP" && ! says imx8mp "$d" "tftp="'
echo "00000000 00000000 00000000 00000000" > "$d/snvs.txt"
check "imx8mp uboot-mmc: the record still the cleared one -> no_record rc 4" says imx8mp "$d" "rc=4 verdict=no_record"
mkrun "$d" board=imx8mp boot=uboot-mmc name=probe probe=1 back=1 up=40 "rec0=00000000 00000000 00000000 00000000"
check "imx8mp uboot-mmc probe: back, the one-shot cleared, the record untouched -> probe_ok rc 0" says imx8mp "$d" "rc=0 verdict=probe_ok"
mkrun "$d" board=imx8mp boot=uboot-mmc name=probe probe=1 back=1 rc_cycle=6 up=40 "rec0=00000000 00000000 00000000 00000000"
check "imx8mp uboot-mmc probe: yako_once still set after the cycle (the hook never ran) -> not ok, rc 6" \
    says imx8mp "$d" "rc=6 verdict=probe_failed"

# a live run derives console.txt from the bridge's console.ts (board.sh's own console_from_ts)
eval "$(sed -n '/^console_from_ts() {/,/^}/p' "$B")"
RD="$S/am62p/20261003-1200-rtiproof-0123abcd"
console_from_ts
check "console_from_ts: one 'HH:MM:SS line' per console.ts row, the verdict unchanged with it" \
    eval '[ "$(wc -l < "$RD/console.txt")" = "$(wc -l < "$RD/console.ts")" ] &&
          grep -qE "^[0-9]{2}:[0-9]{2}:[0-9]{2} gl_test: all: 1 of 38" "$RD/console.txt" &&
          says am62p "$RD" "rc=4 verdict=failed .*result=legacy:gl_test_all"'

echo "== 3b. i.MX8MP SNVS records: board.sh's verdict against the old netboot runner's (old-runner-outputs.txt)"
i=0
while read -r w0 w1 w2 w3 ov orc; do
    i=$((i + 1))
    old="$ov $orc"
    for boot in uboot-tftp kexec-dev; do
        d="$S/imx/$boot/20261003-1200-r$i-0123abcd"
        mkrun "$d" board=imx8mp "boot=$boot" "name=r$i" probe=0 back=1 lost=3 up=40 t_reboot=100 t_back=200 \
            "rec0=00000000 00000000 00000000 00000000"
        echo "$w0 $w1 $w2 $w3" > "$d/snvs.txt"
        echo "1791000000.000 tftp: done peer=192.0.2.3:1024 file=yakogl/r$i.bin bytes=1048576 sha256=0 seconds=0.900 mib_per_s=1.1" > "$d/tftp.txt"
        new=$(BOARD_BOOT=$boot "$B" imx8mp tally "$d" | head -1)
        check "$boot $w0 $w1 $w2 $w3: old '$old'; new verdict=$(field verdict <<<"$new") rc=$(field rc <<<"$new")" \
            eval '[ "verdict=$(field verdict <<<"$new") rc=$(field rc <<<"$new")" = "$old" ]'
        [ "$i" = 3 ] && echo "        new: $new"
    done
done < <(gold "imx8mp netboot runner: verdict <w0..w3>")
check "kexec-dev says DEV and never a row" grep -q " DEV .*never a row" <<<"$new"
# the TFTP and boot verdicts the old netboot runner gave without a record (its step 9)
d="$S/imx/x/20261003-1200-nr-0123abcd"
mkrun "$d" board=imx8mp boot=uboot-tftp name=nr probe=0 back=1 up=40 "rec0=00000000 00000000 00000000 00000000"
check "imx8mp: no TFTP line for the token -> no_request rc 4" says imx8mp "$d" "rc=4 verdict=no_request"
echo "1791000000.000 tftp: done peer=x file=yakogl/nr.bin bytes=10 sha256=0 seconds=0.1 mib_per_s=0" > "$d/tftp.txt"
echo "00000000 00000000 00000000 00000000" > "$d/snvs.txt"
check "imx8mp: fetched, the record still the cleared one -> no_record rc 4" says imx8mp "$d" "rc=4 verdict=no_record"
mkrun "$d" board=imx8mp boot=uboot-tftp name=nr probe=0 rc_cycle=5 up=4000 "rec0=00000000 00000000 00000000 00000000"
check "imx8mp: the same Linux still up -> no_reboot rc 5" says imx8mp "$d" "rc=5 verdict=no_reboot"
mkrun "$d" board=imx8mp boot=uboot-tftp name=probe probe=1 back=1 up=40 "rec0=00000000 00000000 00000000 00000000"
echo "1791000000.000 tftp: refused peer=x file=yakogl/p.bin cause=not_found" > "$d/tftp.txt"
check "imx8mp probe: not_found and the record untouched -> probe_ok rc 0" says imx8mp "$d" "rc=0 verdict=probe_ok"

echo "== 3c. Pi 4 records (29460f96 full tree): board.sh's verdict against the old tryboot runner's end-line rule"
# what the old runner's END_RE said for each record (complete = end line seen and Linux back)
endsec="pi4 tryboot runner: end line seen (its END_RE on the record): TF XT GT21 TF-without-its-end-lines"
check "old-runner-outputs.txt has the end-line section (4 records)" [ "$(gold "$endsec" | grep -c .)" = 4 ]
for L in TF XT GT21; do
    d="$S/pi4/full/20261003-1300-$L-0123abcd"
    mkrun "$d" board=pi4 boot=tryboot "name=$L" probe=0 back=1 t_reboot=100 t_back=370
    cp "$TD/pi4-tryboot-$L-29460f96.console.txt" "$d/console.txt"
    oldend=$(gold "$endsec" | awk -v l="$L" '$1 == l { print $2 }')
    new=$("$B" pi4 tally "$d" | head -1)
    check "$L: old end line seen=$oldend, Linux back (old runner: complete, rc 0); new verdict=$(field verdict <<<"$new") result=$(field result <<<"$new") resets=$(field resets <<<"$new") rc=$(field rc <<<"$new")" \
        eval '[ "$oldend" = 1 ] && [ "$(field verdict <<<"$new")" = done ] && [ "$(field rc <<<"$new")" = 0 ] && [ "$(field resets <<<"$new")" = 0 ]'
done
d="$S/pi4/cut/20261003-1300-TFcut-0123abcd"
mkrun "$d" board=pi4 boot=tryboot name=TFcut probe=0 back=1
grep -v -e 'deqp: summary' -e 'v3d_deqp: build' "$TD/pi4-tryboot-TF-29460f96.console.txt" > "$d/console.txt"
oldend=$(gold "$endsec" | awk '$1 == "TFcut" { print $2 }')
check "TF without its end lines: old end line seen=$oldend (rc 1, partial); new no_end_line rc 4" \
    eval '[ "$oldend" = 0 ] && says pi4 "$d" "rc=4 verdict=no_end_line"'
mkrun "$d" board=pi4 boot=tryboot name=TFcut probe=0 back=0
check "Linux not back: old rc 1 'look at the board'; new not_back rc 5" says pi4 "$d" "rc=5 verdict=not_back"
f="$S/pi4/full/20261003-1300-TF-0123abcd/console.txt"
printf '12:00:00 osal(v3d): job 7: GPU reset (recovery 1)\n' >> "$f"
check "a GPU reset line turns done into done_with_resets (never a clean pass)" \
    says pi4 "${f%/*}" "verdict=done_with_resets .*resets=1"
sed -i '$d' "$f"
d="$S/pi4/yk/20261003-1300-yk-0123abcd"
mkrun "$d" board=pi4 boot=tryboot name=yk probe=0 back=1
printf '12:00:00 yk-record: 594b0e13 00000000 000012e8 00000000\n12:00:01 v3d_gl_test RECORD end, build x\n' > "$d/console.txt"
d2="$S/pi4/fp/20261003-1300-fp-0123abcd"
mkrun "$d2" board=pi4 boot=tryboot name=fp probe=0 back=1
printf '12:00:00 yk-record: 594b0f01 1a2b3c4d 00000002 04000258\n' > "$d2/console.txt"
check "a lone FINGERPRINT record is no result and no end: no_end_line" says pi4 "$d2" "verdict=no_end_line"
printf '12:00:01 yk-record: 594b0601 1234abcd 5678ef01 9abc2345\n12:00:01 yk-record: 594b0f02 1a2b3c4d 00000002 04000258\n' >> "$d2/console.txt"
check "PROBES then FINGERPRINT: the verdict is the PROBES record" says pi4 "$d2" "verdict=done .*result=record:PROBES"
check "a yk-record breadcrumb wins over the legacy end line: died_after=console_lcdif_read" \
    says pi4 "$d" "verdict=died_after=console_lcdif_read .*result=record:STAGE"
# a GLMARK2 record (the Pi 4's of 2026-10-03, ddd58c27): the unsupported
# scenes are their own count beside the verdict, never folded into it
d3="$S/pi4/gm2/20261003-2309-glmark2-0123abcd"
mkrun "$d3" board=pi4 boot=tryboot name=glmark2 probe=0 back=1
printf '23:16:00 glmark2: score 201: 29 scenes ran, 0 failed, 4 unsupported (33 list entries)\n23:16:00 yk-record: 594b1201 000000c9 021d0004 03200258\n' > "$d3/console.txt"
check "a GLMARK2 record: the verdict line says unsupported=4 beside the result" \
    says pi4 "$d3" "result=record:GLMARK2 .* unsupported=4"

# The record's own status (yakogl-tools record --line status=): a pass or a
# kind without one is done, a fail is failed (rc 4), an undefined value is
# bad_record. Each on the console (Pi 4) and in the SNVS (i.MX8MP).
st=0
while read -r want rc w0 w1 w2 w3 what; do
    st=$((st + 1))
    d="$S/pi4/st/20261003-1300-st$st-0123abcd"
    mkrun "$d" board=pi4 boot=tryboot "name=st$st" probe=0 back=1
    printf '12:00:00 yk-record: 594b0401 ff403f80 35706c4e 04000258\n12:00:00 yk-record: %s %s %s %s\n' \
        "$w0" "$w1" "$w2" "$w3" > "$d/console.txt"
    check "pi4 console, PICTURE then $what: verdict=$want rc=$rc" says pi4 "$d" "rc=$rc verdict=$want .*result=record:"
    d="$S/imx/st/20261003-1200-st$st-0123abcd"
    mkrun "$d" board=imx8mp boot=uboot-tftp "name=st$st" probe=0 back=1 lost=3 up=40 t_reboot=100 t_back=200 \
        "rec0=00000000 00000000 00000000 00000000"
    echo "$w0 $w1 $w2 $w3" > "$d/snvs.txt"
    echo "1791000000.000 tftp: done peer=192.0.2.3:1024 file=yakogl/st$st.bin bytes=1048576 sha256=0 seconds=0.900 mib_per_s=1.1" > "$d/tftp.txt"
    check "imx8mp SNVS, $what: verdict=$want rc=$rc" says imx8mp "$d" "rc=$rc verdict=$want "
done <<'EOF_ST'
done 0 594b0202 00000001 00000001 00000000 APP status 1 (pass)
failed 4 594b0202 00000602 00000001 00000000 APP status 2 (fail, the triangle's check failure 6)
done 0 594b0601 08420882 00210842 98080883 PROBES tag 1 (pass)
failed 4 594b0602 08420882 00210842 98080883 PROBES tag 2 (fail)
bad_record 4 594b0202 00000003 00000001 00000000 APP status 3 (undefined)
done 0 594b010e ffff0107 ffff0030 00100030 CASE_SET 48 of 48 run, case 16 skipped by name (pass)
failed 4 594b010e 002c0107 ffff002f 00100030 CASE_SET 47 of 48 run, case 16 skipped, failing 44 (fail)
bad_record 4 594b010e 002c0107 ffff0030 00100030 CASE_SET a failure named while 48 of 48 passed (inconsistent)
failed 4 594b010e 0010ff06 0000002f 00000031 CASE_SET_FAIL the earlier set form, c4ccc966 (fail)
EOF_ST

echo "== 3d. Pi 4 tree tally and diff against the old Pi 4 tree runner's (old-runner-outputs.txt)"
"$B" pi4 tally "$S/pi4/full" > "$S/new-tally.txt" 2>&1
P="pi4 tree runner: tally (TF, XT)"
# the per-record lines name the file: compare from ": " on, for the dEQP records
gold "$P: per-record lines from ': '" > "$S/a"
grep -E '/20261003-1300-(TF|XT)-0123abcd/console.txt: ' "$S/new-tally.txt" | sed 's/^[^:]*: //' > "$S/b"
same "per-record dEQP verdicts (deqp-uart-tally.py) equal for TF and XT" "$S/a" "$S/b"
gold "$P: recoveries lines without the name" > "$S/a"
sed -nE '/^20261003-1300-(TF|XT)-0123abcd /s/^[^ ]* *//p' "$S/new-tally.txt" > "$S/b"
same "recoveries lines equal" "$S/a" "$S/b"
sed -n '/^== mustpass:/,$p' "$S/new-tally.txt" > "$S/blk"
# The old runner tallied the whole mustpass (17,136 of it "none": groups TF and
# XT never selected). The tally is scoped to the records' case filters now, so
# the golden's result-bearing rows are the reference: every group row with a
# result, column for column, must be ours, and ours has no "none" left.
gold "$P: the mustpass block's group table" |
    awk 'NF == 7 && $1 != "group" && $1 != "TOTAL" && ($2 + $3 + $4 + $5 + $6) > 0 { print $1, $2, $3, $4, $5, $6 }' > "$S/a"
awk 'NF == 7 && $1 != "group" && $1 != "TOTAL" { print $1, $2, $3, $4, $5, $6 }' "$S/blk" > "$S/b"
same "the scoped table's rows equal the old runner's result-bearing rows (pass/fail/n.s./QW/other)" "$S/a" "$S/b"
check "the scoped table: 29 in scope (the golden's 29 with a result), TOTAL none 0, the rest outside" \
    eval 'grep -q "^== mustpass: 29 cases in these records. case filters (17136 outside them" "$S/blk" &&
          grep -qE "^TOTAL +19 +1 +9 +0 +0 +0$" "$S/blk"'
# the diff: one case of TF changed (its first Fail made Pass)
cp -r "$S/pi4/full" "$S/pi4/full2"
sed -i '0,/deqp: Fail dEQP-GLES2/s//deqp: Pass dEQP-GLES2/' "$S/pi4/full2/20261003-1300-TF-0123abcd/console.txt"
gold "pi4 tree runner: tally <B> <baseline>: the per-case diff (TF's first Fail made Pass)" | tail -n +2 > "$S/a"
"$B" pi4 diff "$S/pi4/full" "$S/pi4/full2" | sed 's/ of .* against [^:]*:/:/' > "$S/b"
tail -n +2 "$S/b" > "$S/b2"
same "per-case diff names the old runner's case, and only it" "$S/a" "$S/b2"
check "the diff counts over the scope: 1 of 29 (the old runner said 1 of 17165)" \
    grep -qx "== per-case diff: 1 of 29 mustpass cases differ" "$S/b"
check "the diff names the changed case" grep -q "Fail -> Pass" "$S/b"

echo "== 3e. i.MX8MP tree: imx8mp-full-tree-29460f96.results.tsv as records, board.sh against the old tree runner (old-runner-outputs.txt)"
T="$S/imxtree"
mkdir -p "$T"
cp "$S/img/imx8mp.bin" "$S/one.bin"
sha8=$(sha256sum "$S/one.bin" | cut -c1-8)
awk -v img="$S/one.bin" '/^[[:space:]]*(#|$)/ { next } { printf "e%02d\t%s\t%d\t%s\n", n, img, n, $0; n++ }' \
    "$HERE/cts/imx8mp-full-tree.txt" > "$T/tree.txt"
crc() { "$YAKOGL_TOOLS" record crc16 "$1"; }
# one kind DEQP record per row, and the DEQP_FAILS pages (six name checks each) of the named rows
while IFS=$'\t' read -r idx filt begun pass fail ns other nr names; do
    case "$idx" in \#*) continue ;; esac
    e=$(printf 'e%02d' "$idx")
    d="$T/20261003-1400-$e-$sha8"
    mkrun "$d" board=imx8mp boot=uboot-tftp "name=$e" probe=0 back=1 up=40
    first=ffff
    [ "$names" != - ] && first=$(crc "${names%%;*}")
    printf '594b0801 %04x%04x %04x%04x %s%04x\n' "$fail" "$pass" "$other" "$ns" "$first" "$begun" > "$d/snvs.txt"
    : > "$d/INDEX.txt"
    [ "$names" = - ] && continue
    IFS=';' read -ra nm <<<"$names"
    page=0
    while [ $((page * 6)) -lt ${#nm[@]} ]; do
        ws=()
        for k in 0 2 4; do
            lo=ffff
            hi=ffff
            [ $((page * 6 + k)) -lt ${#nm[@]} ] && lo=$(crc "${nm[page * 6 + k]}")
            [ $((page * 6 + k + 1)) -lt ${#nm[@]} ] && hi=$(crc "${nm[page * 6 + k + 1]}")
            ws+=("$hi$lo")
        done
        p="$T/20261003-1400-$e-p$page-$sha8"
        mkrun "$p" board=imx8mp boot=uboot-tftp "name=$e-p$page" probe=0 back=1
        printf '594b0a%02x %s %s %s\n' "${#nm[@]}" "${ws[@]}" > "$p/snvs.txt"
        : > "$p/INDEX.txt"
        page=$((page + 1))
    done
done < "$HERE/cts/imx8mp-full-tree-29460f96.results.tsv"
"$B" imx8mp tally "$T" > "$S/new-imx.txt" 2>&1
same "board.sh results.tsv equals the committed imx8mp-full-tree-29460f96.results.tsv" \
    "$HERE/cts/imx8mp-full-tree-29460f96.results.tsv" "$T/results.tsv"
mkdir -p "$S/prevdir"
awk -F'\t' 'BEGIN { OFS = FS } !/^#/ && $1 == 1 { $4 = 18 } { print }' "$HERE/cts/imx8mp-full-tree-29460f96.results.tsv" > "$S/prevdir/results.tsv"
gold "imx8mp tree runner: rows.md" > "$S/a"
check "old-runner-outputs.txt has the old tree runner's rows.md (29 rows)" [ "$(grep -c '^| [0-9]' "$S/a")" = 29 ]
same "rows.md: board.sh equals the old tree runner's" "$S/a" "$T/rows.md"
gold "imx8mp tree runner: TALLY" > "$S/a"
grep '^TALLY' "$S/new-imx.txt" > "$S/b"
same "the TALLY line equal: $(cat "$S/b")" "$S/a" "$S/b"
gold "imx8mp tree runner: --prev (e01 passed 18): DIFF" > "$S/a"
"$B" imx8mp diff "$S/prevdir" "$T" | sed '1s/ against .*:/:/' > "$S/b"
same "the per-row diff equal ($(tail -1 "$S/b" | sed 's/^ *//'))" "$S/a" "$S/b"
check "the diff names the changed row e01" grep -q '^  e01 now' "$S/b"

echo "board_selftest: $n checks, $fails failed (pinned: $WANT_CHECKS)"
[ "$fails" = 0 ] && [ "$n" = "$WANT_CHECKS" ]

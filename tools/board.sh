#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# board.sh - one runner for the three boards (driver/docs/one-method.md section 4).
#
#   tools/board.sh [--dry-run] <board> status | install | uninstall | disarm
#   tools/board.sh [--dry-run] <board> probe <logdir>
#   tools/board.sh [--dry-run] <board> run <image> <logdir> [name]
#   tools/board.sh [--dry-run] <board> tree <table> <outdir>
#   tools/board.sh [--dry-run] <board> default <image> <boots|forever> | nodefault
#   tools/board.sh <board> tally <rundir> | diff <rundir-a> <rundir-b>
#      board = pi4 | am62p | imx8mp
#      BOARD_BOOT = tryboot | uboot-tftp | uboot-mmc | kexec-dev
#
# Only the boot backend differs between the boards; everything else is one
# sequence. The backends: tryboot (pi4; the firmware boots the image once
# from the SD card), uboot-tftp (am62p, imx8mp, the default there; U-Boot
# fetches the image over TFTP once: tools/uboot-netboot-lib.sh,
# tools/tftp_oneshot.py), uboot-mmc (am62p, imx8mp; the same one-shot, but
# U-Boot loads the image from the eMMC, where Linux staged it: the standalone
# boot, docs/standalone-boot.md) and kexec-dev
# (am62p, imx8mp; driver/tools/kexec-zephyr). A kexec-dev run is a development
# run: its verdict line says DEV and it is never a row (a bootloader boot is
# the bar). --dry-run prints every command that would touch a board, the TFTP
# root or a serial bridge and runs none of them (board reads included); it
# still checks the image on the host. tools/board_selftest.sh runs every verb
# on every board this way.
#
# "run" does, in order, and stops at the first failure:
#   1. checks the image: the AArch64 Image header, its footprint inside the
#      board's RAM window, and the watchdog marker (the board's literal, or the
#      board layer's "board: watchdog armed"; NETBOOT_NO_WATCHDOG=1 overrides);
#   2. takes the board lock (driver/tools/kexec-zephyr/board_lock.sh: one lock
#      per BOARD_HOST in /run/user/<uid>/yakogl-locks) and makes the pre-checks:
#      ssh and sudo -n, no session on the board from another machine (`who`),
#      nobody holds the console or the GPU device, and the backend's own
#      (free space on /boot/firmware; the TFTP server and its log; the U-Boot
#      hook installed, no one-shot pending, no default set, no kexec loaded;
#      the staging directory on the eMMC partition U-Boot reads, its free
#      space, no leftover; uptime, the M7, no client running, the crash window);
#   3. clears the result sink (i.MX8MP: the SNVS record, plus SEL's selector);
#   4. stages the image (/boot/firmware + tryboot.txt | the TFTP root |
#      /var/yakogl on the eMMC | ~/kexec) and reads it back by sha256 (the
#      eMMC copy after dropping the page cache);
#   5. starts the console capture (Pi 4 UART | AM62P serial bridge; the
#      i.MX8MP has none, its TFTP log and SNVS record are read afterwards);
#   6. boots (tryboot reboot | the U-Boot one-shot and a reboot | kexec);
#   7. waits for the image's end (a result `yk-record:` line, or the sample's
#      own end line) or the bound, then for Linux from a new boot;
#   8. reads the records (yk-record: lines, the SNVS record) and decodes them
#      with `yakogl-tools record`;
#   9. prints one verdict line (below) and writes the run's INDEX.txt;
#  10. cleans up (the staged image, tryboot.txt, the one-shot: yako_once gone).
#
# "default <image> <boots|forever>" makes the image the board's boot (the
# product form, docs/standalone-boot.md). am62p, imx8mp: it is staged as
# /var/yakogl/default.bin and yako_default names it, so U-Boot loads it from
# the eMMC at power-on. With a count (1-9) U-Boot boots it that many times and
# then deletes the default and boots Linux: an image that ends in a watchdog
# reset gives the board back by itself, so no person is needed. "forever" is
# the product: only a person at the U-Boot prompt brings Linux back, so it
# refuses unless BOARD_PERSON_PRESENT=1. pi4: config.txt names the image
# (Linux's kept as config-linux.txt); the firmware has no boot count, so only
# "forever", with a person. "default" prints the exact undo before it changes
# anything, and reboots nothing. "nodefault" undoes it from Linux.
#
# The verdict line, one format on every board:
#   board.sh <board> <name> rc=<rc> verdict=<v> boot=<backend> <row|DEV> boot_verdict=<bv>
#       result=<record:<KIND>|legacy:<end line>|none> resets=<n|absent> record=<w0,w1,w2,w3|absent>
#       back_after=<s>s
# verdict=done only when the boot completed AND a result is there: a result
# record (not a breadcrumb) whose own status is a pass or which has none
# (`yakogl-tools record --line` status=pass|n/a), or, until the board layer
# prints yk-record: on a board, the sample's own result line (dEQP's summary,
# gl_test's "all:" or "RECORD end"). A result record with status=fail (APP,
# PROBES, CASE, FAULT, GLMARK2) is verdict=failed; status=invalid is bad_record. A GPU
# reset count other than 0 is never a clean done
# (done_with_resets). A breadcrumb gives died_after=<stage>. boot_verdict is
# the backend's own word (the AM62P phase scan's, the i.MX8MP TFTP log's).
#
# Exit codes: 0 done (probe: passed); 2 usage; 3 refused before the board was
# changed; 4 the verdict is not done; 5 the board did not come back, or an env
# write failed; 6 the U-Boot one-shot was still set after the cycle (cleared).
#
# Records: run writes <logdir>/<YYYYMMDD-HHMM>-<name>-<sha8>/ with meta.txt
# (what the run measured, key=value), console.txt ("HH:MM:SS line"), the
# backend's own files (console.raw and console.ts from the AM62P bridge,
# tftp.txt and snvs.txt on the i.MX8MP, kexec.log) and INDEX.txt (the runner's
# build, the image's sha256 and build line, board, backend, times, the verdict
# line, the decoded records, every file with its sha256 and line count;
# records/README.md). Addresses in the copies are redacted (IPv4 -> 192.0.2.n,
# MAC -> 02:00:00:00:00:nn, the board's host name -> <board-host>); BOARD_HOST
# is never written.
#
# "tree" runs a table of images one after the other (resumable: a row whose
# run directory has an INDEX.txt is skipped), then tallies. The table is one
# dEQP filter per line ('#' lines skipped), the form of the three
# tools/cts/*-full-tree.txt files, and the image of row i is
#   TREE_IMAGE=<zephyr.bin>  one image for every row, the row picked by the
#                            selector i (i.MX8MP: CONFIG_DEQP_CASE_TABLE);
#                            name e<i>; failing rows get their DEQP_FAILS pages
#                            (e<i>-p<page>, selector 0x8000 | page << 8 | i)
#                            unless NAMES=0
#   TREE_IMAGE_DIR=<dir>     an image per row: <dir>/<label>/zephyr/zephyr.bin,
#                            label from the "# image <LABEL>:" line above the
#                            row (pi4-full-tree.txt) or img<i+1>-$TREE_TAG
#                            (am62p-deqp-images.sh); a builder's <label>.built
#                            or build.out "img<N> rc=" line is waited for and a
#                            failed build skipped
#   neither                  the table's lines are "<name> <image> [<selector>]"
# TREE_ROWS=3,5-9 runs only those rows. A refusal (rc 3) is retried
# TREE_RETRIES times TREE_RETRY_S apart (i.MX8MP 60 x 30 s, the others 0:
# the chain stops); a board that did not come back (rc 5) always stops it.
# TREE_PREV=<outdir> adds a diff against an earlier tree. The tally and the
# diff count only the mustpass cases the records' own case filters select
# ("deqp: case filter" lines): a case no row ran is "outside, not run", never
# NoResult, so a five-row baseline diffs cleanly against a whole tree
# (deqp-uart-tally.py, the scope).
#
# "tally <dir>" re-reads a run directory (its verdict line, recomputed from
# its files) or a tree's (every run's line, then the dEQP tally: per console
# tools/cts/deqp-uart-tally.py's verdict, the recoveries, the mustpass table
# and the cases not Pass; for the selector form results.tsv and rows.md).
# "diff <a> <b>" compares two trees: per mustpass case (consoles) or per row
# (results.tsv).
#
# Environment (no default for a machine or an address):
#   BOARD_HOST         user@host of the board's Linux (pi4: PI4_HOST also read)
#   BOARD_UART         pi4: the serial device wired to the Pi (PI4_UART also
#                      read); am62p: host:port of the serial bridge (UART_ADDR
#                      also read)
#   BOARD_BOOT         the boot backend (default: pi4 tryboot, else uboot-tftp)
#   BOARD_PERSON_PRESENT  1: a person is at the board ("default ... forever" refuses without)
#   NETBOOT_SERVER, NETBOOT_TFTP_ROOT, NETBOOT_TFTP_LOG (i.MX8MP), NETBOOT_ROUTE
#                      (AM62P env|typed), NETBOOT_NO_WATCHDOG, NETBOOT_BACKUP_DIR:
#                      as driver/docs/am62p-netboot-plan.md and imx8mp-netboot-plan.md describe
#                      (uboot-mmc needs no server, TFTP root, log or route)
#   RUN_BOUND, RETURN_BOUND  seconds to the image's end and to Linux after it
#                      (pi4 420/300, am62p U-Boot 420/180, imx8mp U-Boot 600
#                      in all, am62p kexec-dev 1500/600, imx8mp kexec-dev 1500
#                      in all)
#   BOARD_END_RE       extended regex of the image's last line (pi4: PI4_END_RE
#                      also read); BOARD_TAIL_S seconds kept after it (5)
#   BOARD_HDMI_TIMINGS pi4: the panel's hdmi_timings (PI4_HDMI_TIMINGS also read)
#   BOARD_MUSTPASS     the mustpass list the console tally and diff read
#                      (default tools/cts/gles2-mustpass.txt; a dEQP-GLES3
#                      tree names tools/cts/gles3-mustpass.txt)
#   SEL                i.MX8MP: the selector written into the cleared record
#   LOCK_WAIT, HOLD_MAX  see board_lock.sh
#   YAKOGL_TOOLS       the yakogl-tools binary (default: built with cargo here)
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
SELF="$HERE/board.sh"

DRY=0
args=()
for a in "$@"; do
    if [ "$a" = --dry-run ]; then DRY=1; else args+=("$a"); fi
done
set -- "${args[@]+"${args[@]}"}"
DRYFLAG=()
[ "$DRY" = 1 ] && DRYFLAG=(--dry-run)
usage() { sed -n '6,13p' "$SELF" >&2; exit 2; }
[ $# -ge 2 ] || usage
BOARD=$1 VERB=$2
shift 2

# ---- the board facts: everything that differs per board is here ----------
NEW_MARKER='board: watchdog armed'   # the board layer's literal (one-method.md section 4, step 1)
NV=
GPU_DEV=
RESET_RE=
case "$BOARD" in
pi4)
    BOOTS="tryboot"
    NB_MMC=-
    # driver/samples/zephyr/v3d_*/boards/rpi_4b.overlay: sram0 at 0x200000; the
    # largest window in the tree is v3d_deqp's 256 MiB (reg <0x200000 0x10000000>)
    YAKO_ADDR=0x200000; LOAD_END=0x10200000
    MARKER='board: PM watchdog resets'   # driver/zephyr/lib/board/rpi4/rpi_wdog.c:93
    NB_WATCHDOG="the PM watchdog"
    RESET_RE='GPU reset (recovery'       # src/osal/zephyr_v3d.c:887, one line per recovery
    END_RE_DEFAULT='^(deqp: summary: |v3d_deqp: build [^ ]+ (DONE|INCOMPLETE|STOPPED)|v3d_gl_test RECORD end)'
    LEGACY_RE=$END_RE_DEFAULT
    BOARD_HOST=${BOARD_HOST:-${PI4_HOST:-}}
    BOARD_UART=${BOARD_UART:-${PI4_UART:-}}
    BOARD_END_RE=${BOARD_END_RE:-${PI4_END_RE:-}}
    BOARD_HDMI_TIMINGS=${BOARD_HDMI_TIMINGS:-${PI4_HDMI_TIMINGS:-}}
    TREE_RETRIES_DEFAULT=0
    ;;
am62p)
    BOOTS="uboot-tftp uboot-mmc kexec-dev"
    # the eMMC's first user-area partition (Torizon's ext4 root): U-Boot's
    # "mmc 0:1" (SafeUI's bootcmd loads from it with ext4load), Linux's mmcblk0p1
    NB_MMC=0:1; MMC_PART=/dev/mmcblk0p1
    # the SafeUI A53 board DTS: the image's link address up to the GPU carve-out (344 MiB)
    YAKO_ADDR=0x82000000; LOAD_END=0x97800000
    MARKER='rti_arm: armed'              # driver/zephyr/lib/board/am62p/rti_arm.c:235
    NB_WATCHDOG="the RTI"
    # no GPU_DEV: /dev/pvrgpu went with pvrgpu.ko (driver/tools/pvrlab/README.md),
    # so a holder check could never find one
    # RESET_RE stays empty: the PowerVR recovery lines are not one per recovery,
    # so pvr_resets reads the count from them (driver/rust/src/gpu/pvr/render.rs)
    # the gl_test sample's own end, "gl_test: OK|FAILED (rc=N)", ends a
    # single-case run too (it prints no "all:" line)
    END_RE_DEFAULT='^deqp: summary|ZEPHYR FATAL ERROR|RTI feeder lifetime over|^gl_test: all: |^gl_test: (OK|FAILED) \(rc='
    LEGACY_RE='^(deqp: summary: |gl_test: all: )'
    BOARD_UART=${BOARD_UART:-${UART_ADDR:-}}
    TREE_RETRIES_DEFAULT=0
    ;;
imx8mp)
    BOOTS="uboot-tftp uboot-mmc kexec-dev"
    # the eMMC is U-Boot's mmc 2 (its env's "mmc dev 2 1"; the distro boot
    # finds Torizon on partition 1) and Linux's mmcblk2
    NB_MMC=2:1; MMC_PART=/dev/mmcblk2p1
    # driver/boards/toradex/verdin_imx8mp a53_ddr_section: 256 MiB at 0x60000000
    YAKO_ADDR=0x60000000; LOAD_END=0x70000000
    MARKER='gc_wdog_arm: armed'          # CONFIG_GC_BOARD_WDOG_SELF_ARM
    NB_WATCHDOG=WDOG1
    NV=/sys/bus/nvmem/devices/30370000.snvs:snvs-lpgpr0/nvmem
    END_RE_DEFAULT=
    LEGACY_RE=
    TREE_RETRIES_DEFAULT=60
    ;;
*) usage ;;
esac
BOOT=${BOARD_BOOT:-${BOOTS%% *}}
case " $BOOTS " in *" $BOOT "*) ;; *) echo "board.sh $BOARD: BOARD_BOOT=$BOOT is not one of: $BOOTS" >&2; exit 2 ;; esac
if [ "$BOOT" = kexec-dev ]; then ROW=DEV; else ROW=row; fi
END_RE=${BOARD_END_RE:-$END_RE_DEFAULT}
TAIL_S=${BOARD_TAIL_S:-5}

# The U-Boot side and the primitives every backend uses (die, b, act,
# need_board, env_get, check_install, setenv_script, nb_*, TMP):
# tools/uboot-netboot-lib.sh.
NB_TAG="board.sh $BOARD"
NB_RECORDS="records/$BOARD/netboot"
# shellcheck source=tools/uboot-netboot-lib.sh
. "$HERE/uboot-netboot-lib.sh"

note() { echo "board.sh $BOARD: $*"; }

# bget VAR CMD [DRY-VALUE]: a board read into VAR. In --dry-run the command is
# printed (a capture would swallow b's own print) and VAR gets DRY-VALUE.
bget() {
    local __v=$1 __c=$2 __d=${3:-} __o __r=0
    if [ "$DRY" = 1 ]; then
        echo "DRY board\$ $__c"
        printf -v "$__v" '%s' "$__d"
        return 0
    fi
    __o=$(b "$__c") || __r=$?
    printf -v "$__v" '%s' "$__o"
    return "$__r"
}

# ytools ARGS...: yakogl-tools, built once from this checkout unless
# YAKOGL_TOOLS names the binary.
ytools() {
    if [ -z "${YAKOGL_TOOLS:-}" ]; then
        (cd "$REPO" && cargo build -q -p yakogl-tools) >&2 || die "cannot build yakogl-tools" 2
        local td
        td=$(cd "$REPO" && cargo metadata --no-deps --format-version 1 2>/dev/null |
            sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
        YAKOGL_TOOLS=$td/debug/yakogl-tools
        [ -x "$YAKOGL_TOOLS" ] || die "no yakogl-tools binary at $YAKOGL_TOOLS" 2
        export YAKOGL_TOOLS
    fi
    "$YAKOGL_TOOLS" "$@"
}

# ---- the run directory -------------------------------------------------------
RD=
# meta KEY VALUE: one measured fact of this run (meta.txt, key=value; the last wins)
meta() {
    if [ "$DRY" = 1 ]; then return 0; fi
    printf '%s=%s\n' "$1" "$2" >> "$RD/meta.txt"
}
# mget KEY [DIR]: a fact of a run, empty when absent
mget() { sed -n "s/^$1=//p" "${2:-$RD}/meta.txt" 2>/dev/null | tail -1; }

# is_run_of NAME DIR [SHA8]: DIR's name is <YYYYMMDD-HHMM>-NAME-<sha8|probe>
# (a glob would take "e01-p0"'s directory for "e01"'s)
is_run_of() {
    local base=${2%/}
    base=${base##*/}
    [[ $base =~ ^[0-9]{8}-[0-9]{4}-(.+)-([0-9a-f]{8}|probe)$ ]] || return 1
    [ "${BASH_REMATCH[1]}" = "$1" ] || return 1
    [ -z "${3:-}" ] || [ "${BASH_REMATCH[2]}" = "$3" ]
}

# redact_dir DIR STRING...: the copies never carry an address. Every IPv4
# address outside 192.0.2.0/24 becomes 192.0.2.<n> (one n per address, in the
# order first seen), every MAC 02:00:00:00:00:<nn>, and each STRING (the board's
# host name, BOARD_HOST's host part) <board-host>. Binary files included (GNU
# sed in the C locale passes NULs through).
redact_dir() {
    local dir=$1 f n=0 ip mac s esc
    shift
    local files=()
    for f in "$dir"/*; do [ -f "$f" ] && [ "${f##*/}" != INDEX.txt ] && files+=("$f"); done
    [ ${#files[@]} -gt 0 ] || return 0
    for s in "$@"; do
        # a short alias ("pi4") would rewrite ordinary console text: whole words
        # of six characters or more only (an address is caught below anyway)
        [ ${#s} -ge 6 ] || continue
        esc=$(printf '%s' "$s" | sed 's/[][\\.*^$/|(){}+?]/\\&/g')
        # twice: a match consumes the character after it, which the next
        # match needs in front ("a.b.c.d a.b.c.d")
        LC_ALL=C sed -i -E "s/(^|[^A-Za-z0-9_.-])$esc([^A-Za-z0-9_-]|$)/\\1<board-host>\\2/g;
            s/(^|[^A-Za-z0-9_.-])$esc([^A-Za-z0-9_-]|$)/\\1<board-host>\\2/g" "${files[@]}"
    done
    while read -r ip; do
        n=$((n + 1))
        LC_ALL=C sed -i -E "s/(^|[^0-9.])${ip//./\\.}([^0-9]|$)/\\1192.0.2.$n\\2/g;
            s/(^|[^0-9.])${ip//./\\.}([^0-9]|$)/\\1192.0.2.$n\\2/g" "${files[@]}"
    done < <(LC_ALL=C grep -aohE '(^|[^0-9.])([0-9]{1,3}\.){3}[0-9]{1,3}([^0-9.]|$)' "${files[@]}" |
        LC_ALL=C grep -aoE '([0-9]{1,3}\.){3}[0-9]{1,3}' | grep -v '^192\.0\.2\.' | awk '!seen[$0]++')
    n=0
    while read -r mac; do
        n=$((n + 1))
        LC_ALL=C sed -i "s/$mac/02:00:00:00:00:$(printf '%02x' "$n")/g" "${files[@]}"
    done < <(LC_ALL=C grep -aohE '([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}' "${files[@]}" |
        grep -v '^02:00:00:00:00:' | awk '!seen[$0]++')
}

# plain CONSOLE: the console's lines without their stamp ("HH:MM:SS ", or the
# bridge capture's "<unix time> "), NULs and CRs
plain() { sed -E 's/^([0-9:]{8}|[0-9]{9,}\.[0-9]+) //' "$1" | tr -d '\0\r'; }

# end_seen CONSOLE: the image's end is in the console: the board's end line, or
# a yk-record line whose class is a result (the board layer's last record).
end_seen() {
    local c=$1
    [ -s "$c" ] || return 1
    if [ -n "$END_RE" ] && plain "$c" | grep -aqE "$END_RE"; then return 0; fi
    if grep -aq 'yk-record: ' "$c"; then
        # (a FINGERPRINT, class fingerprint, is written beside the verdict, never as the end)
        case "$(ytools record --log "$c" --classify 2>/dev/null)" in
        verdict | ended_no_picture) return 0 ;;
        esac
    fi
    return 1
}

# legacy_result CONSOLE: the sample's own result line, until the board layer
# prints yk-record: on this board. Prints a label, or nothing.
legacy_result() {
    local c=$1 l
    { [ -n "$LEGACY_RE" ] && [ -s "$c" ]; } || return 0
    l=$(plain "$c" | grep -aE "$LEGACY_RE" | tail -1)
    case "$l" in
    "") ;;
    "deqp: summary: "*) echo deqp_summary ;;
    "gl_test: all: "*) echo gl_test_all ;;
    *"RECORD end"*) echo gl_test_record_end ;;
    v3d_deqp:*) echo "v3d_deqp_${l##* }" ;;
    *) echo other ;;
    esac
}

# pvr_resets CONSOLE: the AM62P's GPU recoveries from the console, strongest
# source first (driver/rust/src/gpu/pvr/render.rs):
#  1. the run-end summary "render freelists: ... GPU recoveries N (HWR counter C)"
#     (grow_report: N = the counter now minus the counter when the render service
#     started, so N, not C, is this run's count);
#  2. else "render: HWR during fence F (counter A -> B)" lines, printed when a wait
#     sees the counter change (one line may cover several recoveries; the first A
#     is the counter at the service's start, hwr_seen): the last B minus the first A;
#  3. else 0, but only when the console shows the run reached its own end (the
#     sample's result line, legacy_result, or a result record); a run cut short is
#     absent, and so is one that ended in a fatal error or the RTI feeder's end
#     (END_RE matches those too, so end_seen is not this test).
pvr_resets() {
    local c=$1 n a0 b
    [ -s "$c" ] || { echo absent; return; }
    n=$(plain "$c" | sed -nE 's/.*render freelists: .*GPU recoveries ([0-9]+) \(HWR counter [0-9]+\).*/\1/p' | tail -1)
    if [ -n "$n" ]; then echo "$n"; return; fi
    a0=$(plain "$c" | sed -nE 's/.*render: HWR during fence [0-9]+ \(counter ([0-9]+) -> [0-9]+\).*/\1/p' | head -1)
    b=$(plain "$c" | sed -nE 's/.*render: HWR during fence [0-9]+ \(counter [0-9]+ -> ([0-9]+)\).*/\1/p' | tail -1)
    if [ -n "$b" ]; then echo $((b - a0)); return; fi
    if [ -n "$(legacy_result "$c")" ]; then echo 0; return; fi
    if grep -aq 'yk-record: ' "$c"; then
        case "$(ytools record --log "$c" --classify 2>/dev/null)" in
        verdict | ended_no_picture) echo 0; return ;;
        esac
    fi
    echo absent
}

# gl_test_set_verdict CONSOLE: the verdict of a run whose result is a legacy
# line: "failed" when the console's last "gl_test: all: F of T cases failed
# (P passed; X timed out ...)" (gl_test.c, the set or a list) has F or X above
# 0, else "done". Completion alone is not a pass: the i.MX8MP's CASE record of
# the same set reads failed (CASE_SET_FAIL), and one verdict rule holds on all
# three boards (2026-10-04; before it, a set with failures read done here).
gl_test_set_verdict() {
    local l f x
    l=$(plain "$1" 2>/dev/null | grep -aoE 'gl_test: all: -?[0-9]+ of [0-9]+ cases failed \([0-9]+ passed; [0-9]+ timed out' | tail -1)
    if [ -n "$l" ]; then
        f=$(sed -E 's/gl_test: all: (-?[0-9]+) of .*/\1/' <<<"$l")
        x=$(sed -E 's/.*passed; ([0-9]+) timed out/\1/' <<<"$l")
        if [ "$f" != 0 ] || [ "$x" != 0 ]; then echo failed; return; fi
    fi
    echo done
}

# single_case CONSOLE: "pass N" or "fail N" when the console is one gl_test case
# run alone, else nothing. gl_test_run prints "gl_test: start (case=N ...)":
# N >= 0 and not 7 is one case (7 is the regression set, -1 a list, whose
# per-case lines are not a run's verdict); its own "gl_test: case N <name>:
# PASS|FAIL|TIMEOUT" line (gl_test.c run_case) is the result. Such a run prints
# no "gl_test: all:" line, so without this it read as no result even when the
# case passed (AM62P gl_test case 176, 2026-10-04).
# The line can occur more than once: v3d_gl_test's frozen record (Pi 4,
# "v3d_gl_test RECORD" ... "RECORD end") replays the run's last 22 log lines
# after the live run. Every occurrence is read, with the set verdict's rule
# (gl_test_set_verdict): any FAIL or TIMEOUT is "fail", "pass" only when every
# occurrence is PASS. The replay ring holds 22 lines, so a long case's early
# lines can be missing from it; the live copy is still read.
single_case() {
    local c=$1 n l
    [ -s "$c" ] || return 0
    n=$(plain "$c" | sed -nE 's/.*gl_test: start \(case=([0-9]+) .*/\1/p' | tail -1)
    { [ -n "$n" ] && [ "$n" != 7 ]; } || return 0
    l=$(plain "$c" | grep -aoE "gl_test: case $n [^:]*: (PASS|FAIL|TIMEOUT)")
    [ -n "$l" ] || return 0
    if grep -qE ': (FAIL|TIMEOUT)$' <<<"$l"; then
        echo "fail $n"
    else
        echo "pass $n"
    fi
}

# verdict_line DIR: the verdict of a run, from its files alone (run and tally
# both call it). Sets V_LINE, V_RC, V_VERDICT, V_SCAN.
verdict_line() {
    local dir=$1 name boot probe rcc bv="" verdict="" result=none resets=absent rec="" shown="" rec0 line cls kname unsup="" status
    local console=$dir/console.txt scan="" tftp=none tl up="" ba=? recf=absent sc
    # a capture whose console.txt was never derived (an imported bridge log)
    [ -f "$console" ] || [ ! -f "$dir/console.ts" ] || console=$dir/console.ts
    name=$(mget name "$dir"); boot=$(mget boot "$dir"); probe=$(mget probe "$dir"); rcc=$(mget rc_cycle "$dir")
    rec0=$(mget rec0 "$dir")
    case "$boot" in
    tryboot)
        if [ "$(mget back "$dir")" != 1 ]; then bv=not_back; rcc=${rcc:-5}
        elif end_seen "$console"; then bv=complete
        else bv=no_end_line; fi
        ;;
    uboot-tftp | uboot-mmc)
        if [ "$BOARD" = am62p ]; then
            # the AM62P phase scan: tools/am62p_netboot_uart.py stays the one reader
            scan=$(python3 "$HERE/am62p_netboot_uart.py" scan "$dir/console.ts" --token "$(mget token "$dir")" 2>/dev/null)
            bv=$(sed -n 's/.*verdict=\([a-z_]*\).*/\1/p' <<<"$scan")
            bv=${bv:-scan_failed}
            if [ "$probe" = 1 ]; then
                # the probe passes on the failure path, and only with the evidence
                # that each stage it exists to prove was reached: TFTP, the DHCP
                # lease and the server's refusal; eMMC, U-Boot's own line that
                # its ext4load of the absent file failed (the scan's load_failed)
                if [ "$boot" = uboot-mmc ]; then
                    if [ -z "$rcc" ] && [ "$bv" = load_failed ]; then verdict=probe_ok; else verdict=probe_failed; fi
                elif [ -z "$rcc" ] && [ "$bv" = fetch_failed ] && grep -aq 'DHCP client bound' "$dir/console.ts" &&
                    grep -aq 'File not found' "$dir/console.ts"; then verdict=probe_ok; else verdict=probe_failed; fi
            fi
        elif [ "$boot" = uboot-mmc ]; then
            # no console and no server log: the board came back from a new boot
            # with the one-shot cleared (rc_cycle 6 otherwise), and the record
            rec=$(cat "$dir/snvs.txt" 2>/dev/null); shown=$rec
            up=$(mget up "$dir")
            if [ "$rcc" = 5 ]; then
                if [ -n "$up" ]; then bv=no_reboot; else bv=no_answer; fi
            elif [ "$probe" = 1 ]; then
                if [ -z "$rcc" ] && [ "$rec" = "$rec0" ]; then bv=probe_ok; else bv=probe_failed; fi
                verdict=$bv
            else bv=cycled; fi
        else
            tl=$(cat "$dir/tftp.txt" 2>/dev/null)
            if grep -q " done " <<<"$tl"; then
                tftp="done:$(sed -n 's/.* done .* bytes=\([0-9]*\) .*seconds=\([0-9.]*\).*/\1B\/\2s/p' <<<"$tl" | head -1)"
            elif grep -q " refused .*cause=not_found" <<<"$tl"; then tftp=not_found
            elif [ -n "$tl" ]; then tftp=failed; fi
            rec=$(cat "$dir/snvs.txt" 2>/dev/null); shown=$rec
            up=$(mget up "$dir")
            if [ "$rcc" = 5 ]; then
                if [ -n "$up" ]; then bv=no_reboot; else bv=no_answer; fi
            elif [ "$probe" = 1 ]; then
                if [ "$tftp" = not_found ] && [ "$rec" = "$rec0" ]; then bv=probe_ok; else bv=probe_failed; fi
                verdict=$bv
            elif [ "$tftp" = none ]; then bv=no_request
            elif [ "${tftp%%:*}" != done ]; then bv=fetch_failed
            else bv=fetched; fi
        fi
        ;;
    kexec-dev)
        if [ "$(mget jump "$dir")" = failed ]; then bv=no_jump; rcc=${rcc:-5}
        elif [ "$(mget back "$dir")" != 1 ]; then bv=not_back; rcc=${rcc:-5}
        elif [ "$BOARD" = imx8mp ]; then
            if [ -z "$(mget lost "$dir")" ]; then bv=no_jump; else bv=complete; fi
            rec=$(cat "$dir/snvs.txt" 2>/dev/null); shown=$rec
        # a fatal error is never done, summary or not; the RTI feeder's end
        # (the image outlived its bounded feeding) only counts without a result
        elif plain "$console" 2>/dev/null | grep -aq 'ZEPHYR FATAL ERROR'; then bv=fatal
        elif [ -n "$(legacy_result "$console")" ] || [ -n "$(single_case "$console")" ]; then bv=complete
        elif plain "$console" 2>/dev/null | grep -aq 'RTI feeder lifetime over'; then bv=watchdog_end
        elif end_seen "$console"; then bv=complete
        else bv=no_end_line; fi
        ;;
    *) bv=unknown_boot ;;
    esac
    # the result: a record (SNVS, or the console's yk-record: lines), else the
    # sample's own result line
    if [ -z "$verdict" ]; then
        case "$bv" in
        complete | done | fetched | cycled)
            if [ -n "$rec" ] && [ "$rec" = "$rec0" ]; then rec=""; fi
            if [ -z "$rec" ] && [ -s "$console" ] && grep -aq 'yk-record: ' "$console"; then
                rec=$(ytools record --log "$console" | sed -n 's/^verdict record: //p')
                shown=$rec
            fi
            if [ -n "$rec" ]; then
                # shellcheck disable=SC2086  # four words
                line=$(ytools record --line $rec) || line=
                cls=$(sed -n 's/^class=\([^ ]*\).*/\1/p' <<<"$line")
                kname=$(sed -n 's/.* name=\([^ ]*\).*/\1/p' <<<"$line")
                resets=$(sed -n 's/.* resets=\([^ ]*\).*/\1/p' <<<"$line")
                resets=${resets:-absent}
                unsup=$(sed -n 's/.* unsupported=\([^ ]*\).*/\1/p' <<<"$line")
                result="record:$kname"
                status=$(sed -n 's/.* status=\([^ ]*\).*/\1/p' <<<"$line")
                case "$cls" in
                verdict)
                    # the record's own pass/fail (record --line status=):
                    # done only for a pass or a kind without one (n/a)
                    case "$status" in
                    pass | n/a) verdict=done ;;
                    fail) verdict=failed ;;
                    *) verdict=bad_record ;; # invalid, or no status word
                    esac
                    ;;
                died_after=* | ended_no_picture) verdict=$cls ;;
                fingerprint) verdict=no_record ;; # a checksum beside a verdict, never one (record.rs)
                no_magic) verdict=bad_record ;;
                *) verdict=classify_failed ;;
                esac
            elif [ "$BOARD" = imx8mp ]; then
                verdict=no_record
            else
                sc=$(single_case "$console")
                if [ -n "$sc" ]; then
                    # one gl_test case run alone: its own line is the verdict
                    result=legacy:gl_test_case
                    if [ "${sc%% *}" = pass ]; then verdict=done; else verdict=failed; fi
                else
                    result=$(legacy_result "$console")
                    if [ -n "$result" ]; then verdict=$(gl_test_set_verdict "$console"); result="legacy:$result"; else verdict=no_record; result=none; fi
                fi
            fi
            ;;
        *) verdict=$bv ;;
        esac
    fi
    # the GPU reset count: the record's, else the board's console line
    if [ "$resets" = absent ] && [ -n "$RESET_RE" ] && [ -f "$console" ]; then
        resets=$(grep -acF "$RESET_RE" "$console")
    fi
    if [ "$resets" = absent ] && [ "$BOARD" = am62p ] && [ -f "$console" ]; then
        resets=$(pvr_resets "$console")
    fi
    if [ "$verdict" = done ] && [ "$resets" != absent ] && [ "$resets" -gt 0 ]; then verdict=done_with_resets; fi
    if [ -n "$rcc" ]; then V_RC=$rcc
    elif [ "$verdict" = done ] || [ "$verdict" = probe_ok ]; then V_RC=0
    else V_RC=4; fi
    if [ -n "$(mget t_back "$dir")" ] && [ -n "$(mget t_reboot "$dir")" ]; then
        ba=$(($(mget t_back "$dir") - $(mget t_reboot "$dir")))
    fi
    [ -n "$shown" ] && recf=${shown// /,}
    V_VERDICT=$verdict
    V_LINE="board.sh $BOARD $name rc=$V_RC verdict=$verdict boot=$boot $ROW boot_verdict=$bv result=$result resets=$resets record=$recf back_after=${ba}s"
    # what the run could not measure, its own outcome beside the verdict (kind GLMARK2)
    if [ -n "${unsup:-}" ] && [ "$unsup" != absent ]; then V_LINE+=" unsupported=$unsup"; fi
    if [ "$BOARD" = imx8mp ] && [ "$boot" = uboot-tftp ]; then V_LINE+=" tftp=$tftp uptime=${up:-?}s"; fi
    if [ "$BOARD" = imx8mp ] && [ "$boot" = uboot-mmc ]; then V_LINE+=" uptime=${up:-?}s"; fi
    if [ "$ROW" = DEV ]; then V_LINE+=" (kexec-dev: a development run, never a row)"; fi
    V_SCAN=$scan
    return 0
}

# write_index DIR: INDEX.txt (records/README.md): the run, the verdict line,
# the decoded records, and every file with its sha256 and line count.
write_index() {
    local dir=$1 f rec kind
    if [ "$ROW" = DEV ]; then kind="DEV: a development run, never a row"; else kind="a row"; fi
    {
        echo "# tools/board.sh run record (driver/docs/one-method.md section 4; records/README.md)"
        echo "runner: tools/board.sh build $(git -C "$REPO" describe --always --dirty 2>/dev/null || echo unknown)"
        echo "board: $BOARD"
        echo "boot: $(mget boot "$dir") ($kind)"
        echo "name: $(mget name "$dir")"
        echo "image: $(mget image "$dir") sha256 $(mget image_sha256 "$dir") size $(mget image_size "$dir") footprint $(mget footprint "$dir")"
        echo "image build: $(mget image_build "$dir")"
        echo "image zephyr: $(mget image_zephyr "$dir")"
        local con=$dir/console.txt
        [ -f "$con" ] || [ ! -f "$dir/console.ts" ] || con=$dir/console.ts
        echo "build line printed at boot: $(boot_build "$con")"
        echo "started: $(mget started "$dir")"
        echo "ended: $(mget ended "$dir")"
        echo "verdict: $V_LINE"
        [ -n "$V_SCAN" ] && echo "phase scan: $V_SCAN"
        echo "records:"
        rec=$(cat "$dir/snvs.txt" 2>/dev/null)
        if [ -n "$rec" ]; then
            # shellcheck disable=SC2086  # four words
            ytools record $rec 2>&1 | sed 's/^/  /'
        elif [ -s "$dir/console.txt" ] && grep -aq 'yk-record: ' "$dir/console.txt"; then
            ytools record --log "$dir/console.txt" 2>&1 | sed 's/^/  /'
        elif [ "$BOARD" = imx8mp ]; then
            echo "  none (no SNVS record was read)"
        else
            echo "  none (the console has no yk-record line: the image prints none)"
        fi
        echo "files (sha256, lines):"
        for f in "$dir"/*; do
            { [ -f "$f" ] && [ "${f##*/}" != INDEX.txt ]; } || continue
            printf '%s  %s  %s\n' "$(sha256sum "$f" | cut -d' ' -f1)" "$(wc -l < "$f")" "${f##*/}"
        done
    } > "$dir/INDEX.txt"
}

# finish: redact, compute the verdict from the files, write INDEX.txt, print
# the line, exit with its code.
finish() {
    meta ended "$(date '+%F %T')"
    redact_dir "$RD" "${BHOSTNAME:-}" "${BOARD_HOST#*@}"
    verdict_line "$RD"
    write_index "$RD"
    # what the record says (the summary lines the old tryboot runner printed; every console board)
    if [ -s "$RD/console.txt" ]; then
        plain "$RD/console.txt" | grep -aE 'Booting Zephyr|: build [0-9a-f]|deqp: (done|summary|fatal)|cases passed|recoveries: |GPU reset \(recovery|RECORD end|DONE|INCOMPLETE|STOPPED|gl_test: all: |yk-record: |ZEPHYR FATAL|RTI feeder lifetime over' |
            sed 's/^/  /'
    fi
    sed -n '/^records:/,/^files/p' "$RD/INDEX.txt" | sed '$d'
    echo "$V_LINE"
    note "record $RD/INDEX.txt"
    exit "$V_RC"
}

CAP=
stop_cap() { if [ -n "${CAP:-}" ]; then kill "$CAP" 2>/dev/null; wait "$CAP" 2>/dev/null; CAP=; fi; }

# ---- the image (step 1) -------------------------------------------------------
check_image() {
    local img=$1 marker=$MARKER
    grep -aqF "$NEW_MARKER" "$img" 2>/dev/null && marker=$NEW_MARKER
    nb_check_image "$img" "$LOAD_END" "$marker"
    IMG_SHA=$(sha256sum "$img" | cut -d' ' -f1)
    IMG_SIZE=$(stat -c %s "$img")
    IMG_BUILD=$(image_build "$img")
    IMG_ZEPHYR=$(grep -aoE 'Booting Zephyr OS build [0-9A-Za-z._+-]+' "$img" | head -1 | sed 's/.* build //')
    IMG_ZEPHYR=${IMG_ZEPHYR:-absent (no Zephyr banner in the image)}
}

# image_build IMAGE: the build ids of the image's own build directory
# (<build>/zephyr/zephyr.bin -> <build>/build.ninja; the samples' defines:
# PVR_SAMPLE_TREE/TABLE on the AM62P, V3D_BUILD on the Pi 4, GC_BUILD_STAMP on
# the i.MX8MP; tools/cts/pi4-deqp-images.sh reads V3D_BUILD the same way). The image bytes
# hold only the printf formats ("sample: build tree=%s ..."), so a scan of the
# image cannot name the tree; without a build.ninja the value is absent.
image_build() {
    local ninja ids
    ninja=$(dirname "$(dirname "$(readlink -f "$1")")")/build.ninja
    if [ -f "$ninja" ]; then
        ids=$(grep -oE '(PVR_SAMPLE_TREE|PVR_SAMPLE_TABLE|V3D_BUILD|GC_BUILD_STAMP)=\\"[^\\"]*' "$ninja" |
            sed 's/\\"//' | sort -u | paste -sd' ' -)
    fi
    echo "${ids:-absent (no build id in a build.ninja beside the image)}"
}

# boot_build CONSOLE: the build line the image printed at boot ("sample:
# build tree=...", "<sample>: build <hash>"), or absent with the reason
boot_build() {
    local c=$1 l
    if [ ! -s "$c" ]; then echo "absent (no console: the i.MX8MP has no UART)"; return; fi
    # Not anchored at the line's start: the Pi 4 firmware's last line has no
    # newline, so the first Zephyr line (sample_glue.c's PRE_KERNEL_2 build
    # line) arrives glued to it ("MESS:00:00:06.210776:0:sample: build tree=...",
    # the common set of 2026-10-04); the match starts at the build line itself.
    l=$(plain "$c" | grep -aoE '[A-Za-z0-9_][A-Za-z0-9_ ]*: build (tree=)?[0-9A-Za-z][0-9A-Za-z._:+-]{6,}.*' | grep -v '%s' | head -1)
    echo "${l:-absent (the console has no build line)}"
}

# ---- the pre-checks every board makes (step 2) ---------------------------------
common_prechecks() {
    local who mine others junk holders
    bget junk 'sudo -n true' || die "$BOARD_HOST does not answer (ssh key login and sudo -n)"
    bget BOOT0 'cat /proc/sys/kernel/random/boot_id' DRY-BOOT-ID || die "cannot read the board's boot_id"
    bget BHOSTNAME 'uname -n' DRY-HOSTNAME || die "cannot read the board's host name"
    bget who 'who' '' || die "cannot run who on the board"
    # A remote session is ours only when it comes from one of this host's addresses.
    mine=$(ip -o addr 2>/dev/null | awk '{sub(/\/.*/, "", $4); print $4}')
    # shellcheck disable=SC2086  # one address per word
    others=$(awk '/\(.*\)/ { a = $NF; gsub(/[()]/, "", a); print a }' <<<"$who" | sort -u |
        grep -vxF -f <(printf '%s\n' $mine) || true)
    [ -z "$others" ] || die "the board has sessions from other machines ($others); ask before taking it:
$who"
    meta who_sessions "$(grep -c . <<<"$who")"
    if [ -n "$GPU_DEV" ]; then
        bget holders "sudo -n fuser $GPU_DEV 2>/dev/null   # must print nothing" ''
        [ -z "$holders" ] || die "the board is in use: $GPU_DEV is held by pid(s)$holders"
    fi
}

take_lock() {
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ . driver/tools/kexec-zephyr/board_lock.sh   # HOLD_MAX=$HOLD_MAX, one lock per BOARD_HOST"
    else
        # shellcheck source=/dev/null
        . "$REPO/driver/tools/kexec-zephyr/board_lock.sh"
    fi
}

# ---- the i.MX8MP record: the SNVS LPGPR words through nvmem (gc_result.h) --------
# snvs_clear: zeroed (w1 = the selector when SEL is set), read back
snvs_clear() {
    local got w
    if [ -n "${SEL:-}" ]; then
        REC0="00000000 $(printf '%08x' $((0x53450000 | SEL))) 00000000 00000000"
    else
        REC0="00000000 00000000 00000000 00000000"
    fi
    meta rec0 "$REC0"
    [ -n "${SEL:-}" ] && meta sel "$SEL"
    if [ "$DRY" = 1 ]; then
        b "printf <the 16 bytes '$REC0'> | sudo -n tee $NV; sudo -n od -A n -t x4 $NV"
        return 0
    fi
    for w in $REC0; do printf "\\x${w:6:2}\\x${w:4:2}\\x${w:2:2}\\x${w:0:2}"; done |
        b "sudo -n tee $NV >/dev/null" || die "cannot write the SNVS record"
    got=$(snvs_read)
    [ "$got" = "$REC0" ] || die "SNVS record read back '$got', not '$REC0'"
}
# snvs_read: the four words, "w0 w1 w2 w3", empty when the read failed
snvs_read() { b "sudo -n od -A n -t x4 $NV" 2>/dev/null | xargs; }

# ---- tryboot (pi4) ----------------------------------------------------------------
pi4_uart_free() {
    local uart p f holders=
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ scan every /proc/<pid>/fd for $BOARD_UART (nobody may hold it)"
        return 0
    fi
    [ -c "$BOARD_UART" ] || die "$BOARD_UART is not a character device"
    uart=$(readlink -f "$BOARD_UART")
    # Who holds the UART: every fd of every process, resolved, compared exactly.
    for p in /proc/[0-9]*; do
        for f in "$p"/fd/*; do
            [ "$(readlink "$f" 2>/dev/null)" = "$uart" ] &&
                holders+=" ${p#/proc/}($(tr '\0' ' ' < "$p/cmdline" 2>/dev/null | cut -c1-60))"
        done
    done
    [ -z "$holders" ] || die "$uart is held by:$holders"
}

pi4_prechecks() {
    local avail
    pi4_uart_free
    bget avail 'df -B1 --output=avail /boot/firmware | tail -1' 999999999999 || die "df on the board failed"
    avail=${avail// /}
    [[ $avail =~ ^[0-9]+$ ]] || die "cannot read the free space of /boot/firmware ('$avail')"
    # a full boot partition once left the board at the firmware splash, with no watchdog
    [ "$avail" -ge $((2 * IMG_SIZE)) ] || die "/boot/firmware has $avail bytes free, less than twice the image ($IMG_SIZE)"
    meta boot_firmware_free "$avail"
    if [ "$DRY" = 1 ]; then
        b "test ! -e /boot/firmware/$STAGED   # a leftover of a run that did not clean up refuses"
        b "test ! -e /boot/firmware/config-linux.txt   # a permanent default refuses"
    else
        b "test ! -e /boot/firmware/$STAGED" || die "/boot/firmware/$STAGED exists (a run that did not clean up): look at it first"
        # after a tryboot the firmware boots config.txt: with a default set, that is not Linux
        b "test ! -e /boot/firmware/config-linux.txt" ||
            die "a permanent default is set (/boot/firmware/config-linux.txt exists): the board would not come back to Linux ('nodefault')"
    fi
}

# pi4_config KERNEL: the configuration that boots KERNEL, the same lines for a
# tryboot (tryboot.txt) and for the permanent default (config.txt)
pi4_config() {
    printf '%s\n' '[all]' "kernel=$1" arm_64bit=1 enable_uart=1 uart_2ndstage=1
    if [ -n "$BOARD_HDMI_TIMINGS" ]; then
        printf '%s\n' hdmi_group=2 hdmi_mode=87 "hdmi_timings=$BOARD_HDMI_TIMINGS"
    fi
}

pi4_undo() {
    b "sudo rm -f /boot/firmware/$STAGED"
    b 'sudo tee /boot/firmware/tryboot.txt >/dev/null && sync' <<<"$PREV"
}

pi4_stage() {
    local tmpd got gottry
    bget PREV 'cat /boot/firmware/tryboot.txt 2>/dev/null' '<the tryboot.txt read from the board>'
    TRY="# tools/board.sh: $NAME, one-shot boot (reboot \"0 tryboot\"); config.txt stays the Linux default
$(pi4_config "$STAGED")"
    # A fresh directory: a copy into a fixed /tmp name would write through a
    # symlink someone left there (tools/cts/README.md, "Deploying").
    bget tmpd 'mktemp -d' /tmp/DRY.board
    [[ $tmpd == /tmp/* ]] || die "mktemp -d on the board failed"
    b "cat > $tmpd/img" < "$IMG" || { b "rm -rf $tmpd"; die "copy to the board failed"; }
    b "sudo install -m 0755 $tmpd/img /boot/firmware/$STAGED; r=\$?; rm -rf $tmpd; sync; exit \$r" ||
        { pi4_undo; die "install into /boot/firmware failed (removed again)"; }
    # a here-string, not "printf | b": under pipefail a printf whose reader
    # exits first (the dry run's b reads nothing) failed the step at random
    b 'sudo tee /boot/firmware/tryboot.txt >/dev/null && sync' <<<"$TRY" ||
        { pi4_undo; die "writing tryboot.txt failed (previous one put back)"; }
    bget got "sha256sum /boot/firmware/$STAGED" "$IMG_SHA  /boot/firmware/$STAGED"
    got=${got%% *}
    [ "$got" = "$IMG_SHA" ] || { pi4_undo; die "staged image reads back as '$got', not $IMG_SHA (removed again; no reboot)"; }
    bget gottry 'cat /boot/firmware/tryboot.txt' "$TRY"
    [ "$gottry" = "$TRY" ] || { pi4_undo; die "tryboot.txt reads back different (previous one put back; no reboot)"; }
    meta staged "/boot/firmware/$STAGED, read back equal"
}

pi4_capture_start() {
    act stty -F "$BOARD_UART" 115200 cs8 -cstopb -parenb raw -echo || { pi4_undo; die "stty on $BOARD_UART failed"; }
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ read $BOARD_UART line by line into <rundir>/console.txt as 'HH:MM:SS line' &"
        return 0
    fi
    # One reader (two would split the bytes): every line, CRs and NULs dropped,
    # with the time it arrived.
    (
        exec 2>/dev/null
        while IFS= read -r line || [ -n "$line" ]; do
            printf '%(%H:%M:%S)T %s\n' -1 "${line//$'\r'/}"
        done < "$BOARD_UART" > "$RD/console.txt"
    ) &
    CAP=$!
    sleep 1
}

run_tryboot() {
    local t0 ended=0 back=0 bid left
    local bound=${RUN_BOUND:-420} backs=${RETURN_BOUND:-300}
    HOLD_MAX=${HOLD_MAX:-$((bound + backs + 300))}
    STAGED="yakogl-$NAME.bin"
    take_lock
    common_prechecks
    pi4_prechecks
    pi4_stage
    pi4_capture_start
    trap 'stop_cap; rm -rf "$TMP"; [ -n "${BOARD_LOCK_WATCHDOG:-}" ] && kill "$BOARD_LOCK_WATCHDOG" 2>/dev/null' EXIT
    t0=$(date +%s)
    meta t_reboot "$t0"
    if [ "$DRY" = 1 ]; then b 'sudo reboot "0 tryboot"'; else b 'sudo reboot "0 tryboot"' > /dev/null 2>&1; fi
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ wait for the end line ($END_RE) or a result yk-record: line, <= $bound s; then $TAIL_S s more"
        echo "DRY host\$ stop the reader; wait for a boot_id other than the one read (<= $backs s more)"
        pi4_undo
        exit 0
    fi
    note "$NAME: tryboot at $(date +%T), waiting for the record's end (at most $bound s)"
    while [ $(($(date +%s) - t0)) -lt "$bound" ]; do
        if end_seen "$RD/console.txt"; then ended=1; break; fi
        sleep 2
    done
    [ "$ended" = 1 ] && sleep "$TAIL_S"
    stop_cap
    meta end_seen "$ended"
    meta capture_stopped "$(date '+%F %T'), $(wc -l < "$RD/console.txt") lines"
    while [ $(($(date +%s) - t0)) -lt $((bound + backs)) ]; do
        bid=$(b 'cat /proc/sys/kernel/random/boot_id' 2>/dev/null) || bid=
        if [ -n "$bid" ] && [ "$bid" != "$BOOT0" ]; then back=1; break; fi
        sleep 10
    done
    meta back "$back"
    if [ "$back" = 1 ]; then
        meta t_back "$(date +%s)"
        pi4_undo
        left=$(b "ls /boot/firmware/$STAGED 2>/dev/null; cat /boot/firmware/tryboot.txt")
        if [ "$left" = "$PREV" ]; then
            meta cleanup "staged image removed, tryboot.txt put back (read back)"
        else
            meta cleanup "did NOT read back as expected"
            note "warning: clean-up did not read back as expected" >&2
        fi
    else
        meta cleanup "not done: Linux not back"
    fi
    finish
}

# ---- uboot-tftp (am62p, imx8mp) --------------------------------------------------
uboot_on_exit() {
    if [ "${ARMED:-0}" = 1 ]; then
        b 'sudo -n fw_setenv yako_once' && note "disarmed after a failure before the reboot" >&2
        [ "$BOOT" = uboot-tftp ] && rm -f "$NETBOOT_TFTP_ROOT/$UB_FILE"
    fi
    # an eMMC copy staged for a boot that never came is removed again
    if [ -n "${MMC_STAGED:-}" ] && [ "${REBOOTED:-0}" = 0 ]; then
        b "sudo -n rm -f $MMC_STAGED" && note "removed $MMC_STAGED (no reboot)" >&2
    fi
    stop_cap
    rm -rf "$TMP"
    [ -n "${BOARD_LOCK_WATCHDOG:-}" ] && kill "$BOARD_LOCK_WATCHDOG" 2>/dev/null
    return 0
}

# console_from_ts: console.txt ("HH:MM:SS line", the form deqp-uart-tally.py
# reads) from the bridge capture's "<unix time> line" rows.
console_from_ts() {
    [ -f "$RD/console.ts" ] || return 0
    if command -v gawk > /dev/null 2>&1; then
        gawk '{ t = $1; sub(/^[0-9.]+ ?/, ""); printf "%s %s\n", strftime("%H:%M:%S", int(t)), $0 }' \
            "$RD/console.ts" > "$RD/console.txt"
        return
    fi
    # No gawk (strftime is its extension; mawk and busybox awk have none): bash's
    # printf %(fmt)T is the same strftime, so the same lines come out, only slower.
    local line t rest
    while IFS= read -r line || [ -n "$line" ]; do
        t=${line%% *}
        rest=
        [ "$t" != "$line" ] && rest=${line#* }
        printf '%(%H:%M:%S)T %s\n' "${t%%.*}" "$rest"
    done < "$RD/console.ts" > "$RD/console.txt"
}

# ---- the eMMC staging (uboot-mmc, and "default" on am62p/imx8mp) ------------------
# Linux writes the image into STAGE_DIR on the eMMC's user-area filesystem;
# U-Boot reads the same file with "ext4load mmc NB_MMC". On Torizon /var is a
# bind mount of a directory of the root filesystem
# (/ostree/deploy/<os>/var), so the path U-Boot sees is the mount's FSROOT
# plus the path below the mount point, both read from the board (findmnt).
STAGE_DIR=/var/yakogl

# mmc_where: sets UB_DIR (STAGE_DIR as U-Boot names it) after checking that
# STAGE_DIR is on MMC_PART, the partition NB_MMC names; and MMC_FREE.
mmc_where() {
    local m src tgt root rel
    bget m "sudo -n mkdir -p $STAGE_DIR && set -- \$(findmnt -n -v -o SOURCE,TARGET,FSROOT -T $STAGE_DIR) && echo \$(readlink -f \$1) \$2 \$3" \
        "$MMC_PART /var /ostree/deploy/torizon/var" || die "cannot read the mount of $STAGE_DIR on the board"
    read -r src tgt root <<<"$m"
    [ "$src" = "$MMC_PART" ] ||
        die "$STAGE_DIR is on '$src', not $MMC_PART (U-Boot's mmc $NB_MMC): U-Boot would not find the image there"
    [ -n "$root" ] && [ -n "$tgt" ] || die "findmnt gave no mount point or FSROOT for $STAGE_DIR ('$m')"
    if [ "$tgt" = / ]; then rel=$STAGE_DIR; else rel=${STAGE_DIR#"$tgt"}; fi
    UB_DIR=${root%/}$rel
    meta mmc_where "$STAGE_DIR = mmc $NB_MMC $UB_DIR ($src)"
    bget MMC_FREE "df -B1 --output=avail $STAGE_DIR | tail -1" 999999999999 || die "df on the board failed"
    MMC_FREE=${MMC_FREE// /}
    [[ $MMC_FREE =~ ^[0-9]+$ ]] || die "cannot read the free space of $STAGE_DIR ('$MMC_FREE')"
    [ "$MMC_FREE" -ge $((2 * IMG_SIZE)) ] || die "$STAGE_DIR has $MMC_FREE bytes free, less than twice the image ($IMG_SIZE)"
}

# mmc_stage FILE: IMG copied to STAGE_DIR/FILE, read back from the eMMC (the
# page cache dropped first) by sha256; sets MMC_STAGED. Removed on a mismatch.
mmc_stage() {
    local dst=$STAGE_DIR/$1 got
    if [ "$DRY" = 1 ]; then
        echo "DRY board\$ sudo -n sh -c 'cat > $dst.part && sync && mv $dst.part $dst && sync' < $IMG"
    else
        timeout 900 ssh "${SSH_OPTS[@]}" "$BOARD_HOST" "sudo -n sh -c 'cat > $dst.part && sync && mv $dst.part $dst && sync'" < "$IMG" ||
            { b "sudo -n rm -f $dst.part $dst"; die "copying the image to $dst failed"; }
    fi
    MMC_STAGED=$dst
    bget got "echo 3 | sudo -n tee /proc/sys/vm/drop_caches >/dev/null && sha256sum $dst" "$IMG_SHA  $dst"
    got=${got%% *}
    [ "$got" = "$IMG_SHA" ] || { b "sudo -n rm -f $dst"; MMC_STAGED=; die "the eMMC copy $dst reads back as '$got', not $IMG_SHA (removed)"; }
    meta staged "$dst on the eMMC, read back equal"
}

run_uboot() {
    local file route typed_line net back left st pend kx rcc="" el up="" src
    # --dry-run stands in for what a real run would refuse without; only what
    # the backend reads: a uboot-mmc dry run sees no NETBOOT_* (set -u catches
    # a TFTP-only variable on the eMMC path, as on a board)
    if [ "$DRY" = 1 ]; then
        BOARD_UART=${BOARD_UART:-BRIDGE:PORT}
        if [ "$BOOT" = uboot-tftp ]; then
            NETBOOT_SERVER=${NETBOOT_SERVER:-HOST_IP}
            NETBOOT_TFTP_ROOT=${NETBOOT_TFTP_ROOT:-/TFTP_ROOT}; NETBOOT_TFTP_LOG=${NETBOOT_TFTP_LOG:-/TFTP_LOG}
        fi
    fi
    if [ "$BOOT" = uboot-tftp ]; then
        src=net
        : "${NETBOOT_SERVER:?set NETBOOT_SERVER=the address of this host on the board LAN}"
        : "${NETBOOT_TFTP_ROOT:?set NETBOOT_TFTP_ROOT=the root directory of the TFTP server}"
    else
        src=mmc
    fi
    if [ "$BOARD" = am62p ]; then
        : "${BOARD_UART:?set BOARD_UART (or UART_ADDR)=host:port of the serial bridge}"
        RUN_BOUND=${RUN_BOUND:-420}; RETURN_BOUND=${RETURN_BOUND:-180}
        HOLD_MAX=${HOLD_MAX:-$((RUN_BOUND + RETURN_BOUND + 300))}
        route=${NETBOOT_ROUTE:-env}
        case "$route" in env | typed) ;; *) die "NETBOOT_ROUTE must be env or typed, not '$route'" 2 ;; esac
        [ "$route" = env ] || [ "$src" = net ] || die "NETBOOT_ROUTE=typed is a TFTP route; uboot-mmc arms through the env" 2
    else
        [ "$src" = mmc ] || : "${NETBOOT_TFTP_LOG:?set NETBOOT_TFTP_LOG=the --log file of the TFTP server}"
        RUN_BOUND=${RUN_BOUND:-600}
        HOLD_MAX=${HOLD_MAX:-$((RUN_BOUND + 300))}
        route=env
    fi
    TOKEN=$(printf '%s' "$NAME-${IMG_SHA:0:16}-$(date +%s)" | tr -c 'A-Za-z0-9._-' '_')
    if [ "$src" = net ]; then file="yakogl/$TOKEN.bin"; else file="once-$TOKEN.bin"; fi
    meta token "$TOKEN"
    meta route "$route"
    note "$NAME: route $route source $src token $TOKEN"

    # 2. the TFTP server answers on port 69 at the address U-Boot will use (on
    #    the i.MX8MP its log is the one given); then the board
    [ "$src" = net ] && nb_tftp_canary "$NETBOOT_SERVER" "$NETBOOT_TFTP_ROOT" "$TOKEN"
    if [ "$BOARD" = imx8mp ] && [ "$src" = net ]; then
        if [ "$DRY" = 1 ]; then
            echo "DRY host\$ grep 'done .*file=yakogl/canary-$TOKEN ' $NETBOOT_TFTP_LOG   # the canary must be in the log"
        else
            grep -q "done .*file=yakogl/canary-$TOKEN " "$NETBOOT_TFTP_LOG" 2>/dev/null ||
                die "NETBOOT_TFTP_LOG=$NETBOOT_TFTP_LOG has no 'done' line for the canary: it is not the log of the server on $NETBOOT_SERVER:69"
        fi
    fi
    take_lock
    bget st 'systemctl is-system-running --wait >/dev/null 2>&1; cat /proc/uptime' || die "the board's Linux does not answer"
    common_prechecks
    if [ "$BOARD" = imx8mp ]; then
        bget kx 'cat /sys/kernel/kexec_loaded /sys/kernel/kexec_crash_loaded   # both must be 0' $'0\n0'
        kx=${kx//$'\n'/}
        [ "$kx" = 00 ] || die "a kexec image is loaded on the board (kexec_loaded, kexec_crash_loaded = '$kx'): someone else's jump is pending"
    fi
    if [ "$DRY" = 1 ]; then
        b 'sudo -n fw_printenv   # env route: the install must match the library; yako_once and yako_default must be absent'
    else
        if [ "$route" = env ]; then
            st=$(check_install)
            [ "$st" = ok ] || die "the U-Boot hook is not installed as tools/uboot-netboot-lib.sh writes it ($st; 'install' upgrades a stale one)"
        fi
        pend=$(b 'sudo -n fw_printenv -n yako_once 2>/dev/null')
        [ -z "$pend" ] || die "a one-shot is already pending (yako_once=$pend); find its owner, or 'disarm'"
        # with a default set, the boot after the image is the default image, not Linux
        pend=$(b 'sudo -n fw_printenv -n yako_default 2>/dev/null')
        [ -z "$pend" ] || die "a permanent default is set (yako_default=$pend): the board would not come back to Linux ('nodefault')"
    fi
    if [ "$src" = mmc ]; then
        mmc_where
        if [ "$DRY" = 1 ]; then
            b "ls $STAGE_DIR/once-*.bin 2>/dev/null   # must print nothing"
        else
            st=$(b "ls $STAGE_DIR/once-*.bin 2>/dev/null")
            [ -z "$st" ] || die "$STAGE_DIR holds a one-shot image of a run that did not clean up ($st): look at it, or 'disarm'"
        fi
    fi

    # 3. the i.MX8MP record: cleared (and the selector), read back
    [ "$BOARD" = imx8mp ] && snvs_clear

    # From here on any exit runs uboot_on_exit: it disarms an armed env and
    # removes an eMMC copy whose boot never came (MMC_STAGED, REBOOTED).
    ARMED=0
    REBOOTED=0
    UB_FILE=$file
    trap uboot_on_exit EXIT

    # 4. stage: served once, then renamed .served by the server (TFTP), or
    #    written to the eMMC (mmc); read back
    if [ "$PROBE" = 1 ]; then
        note "probe: $file is deliberately not staged"
    elif [ "$src" = mmc ]; then
        mmc_stage "$file"
    else
        act cp "$IMG" "$NETBOOT_TFTP_ROOT/$file"
        if [ "$DRY" = 1 ]; then
            echo "DRY host\$ sha256sum $NETBOOT_TFTP_ROOT/$file   # must be $IMG_SHA"
        else
            [ "$(sha256sum "$NETBOOT_TFTP_ROOT/$file" | cut -d' ' -f1)" = "$IMG_SHA" ] ||
                { rm -f "$NETBOOT_TFTP_ROOT/$file"; die "the staged copy reads back different"; }
            meta staged "TFTP root $file, read back equal"
        fi
    fi

    # 5. the capture, before anything can print (AM62P; the i.MX8MP has no UART)
    if [ "$BOARD" = am62p ]; then
        # the typed route's one command line (TFTP only): the installed hook's
        # yako_net with its three variables written out, then "boot" when the
        # fetch fails
        if [ "$route" = typed ]; then
            net=${YAKO_NET//'${yako_addr}'/$YAKO_ADDR}
            net=${net//'${yako_server}'/$NETBOOT_SERVER}
            net=${net//'${yako_file}'/$file}
            typed_line="echo yakogl-netboot: once=$TOKEN; echo yakogl-netboot: typed; $net; boot"
        fi
        if [ "$DRY" = 1 ] && [ "$route" = typed ]; then
            echo "DRY host\$ python3 tools/am62p_netboot_uart.py typed $BOARD_UART <rundir>/console.raw <rundir>/console.ts '<line>' 2> <rundir>/typed.log &"
            echo "    line (${#typed_line} bytes): $typed_line"
        elif [ "$DRY" = 1 ]; then
            echo "DRY host\$ python3 tools/am62p_netboot_uart.py capture $BOARD_UART <rundir>/console.raw <rundir>/console.ts &"
        else
            if [ "$route" = typed ]; then
                python3 "$HERE/am62p_netboot_uart.py" typed "$BOARD_UART" "$RD/console.raw" "$RD/console.ts" "$typed_line" \
                    2> "$RD/typed.log" &
            else
                python3 "$HERE/am62p_netboot_uart.py" capture "$BOARD_UART" "$RD/console.raw" "$RD/console.ts" &
            fi
            CAP=$!
            sleep 3
            if ! kill -0 "$CAP" 2>/dev/null; then
                CAP=
                # (an eMMC copy is removed by uboot_on_exit)
                [ "$src" = net ] && rm -f "$NETBOOT_TFTP_ROOT/$file"
                die "the UART capture ended at once (bridge busy or unreachable)"
            fi
        fi
    fi

    # 6. arm: one env write, read back. Until the reboot is issued, any exit
    #    disarms, so an armed env never outlives this script.
    if [ "$route" = env ]; then
        if [ "$src" = net ]; then
            printf 'yako_once=%s\nyako_src=net\nyako_file=%s\nyako_server=%s\n' "$TOKEN" "$file" "$NETBOOT_SERVER" > "$TMP/arm"
        else
            printf 'yako_once=%s\nyako_src=mmc\nyako_file=%s\n' "$TOKEN" "$UB_DIR/$file" > "$TMP/arm"
        fi
        if [ "$DRY" = 0 ]; then ARMED=1; fi
        setenv_script "$TMP/arm" || die "fw_setenv (arm) failed" 5
        if [ "$DRY" = 0 ]; then
            back=$(b 'sudo -n fw_printenv -n yako_once')
            [ "$back" = "$TOKEN" ] || die "read-back of yako_once: '$back', not '$TOKEN'" 5
        fi
    else
        note "$NAME: typed route: nothing is written to the env"
    fi
    meta t_arm "$(date +%s)"

    # 7. reboot; the ssh session ends with it
    b 'sync; sudo -n systemctl reboot' || true
    ARMED=0   # the board owns the one-shot now; U-Boot clears it
    REBOOTED=1
    T_REBOOT=$(date +%s)
    meta t_reboot "$T_REBOOT"
    if [ "$DRY" = 1 ]; then
        if [ "$BOARD" = am62p ]; then
            echo "DRY host\$ wait until a kernel starts after the cycle in <rundir>/console.ts (<= $RUN_BOUND s from the reboot), then ssh (<= $RETURN_BOUND s more)"
        else
            echo "DRY host\$ wait until ssh answers from a new boot (uptime < time since the reboot), <= $RUN_BOUND s"
        fi
        b 'sudo -n fw_printenv -n yako_once   # must print nothing'
        if [ "$BOARD" = imx8mp ]; then
            b "sudo -n od -A n -t x4 $NV"
            [ "$src" = net ] && echo "DRY host\$ grep ' file=$file ' $NETBOOT_TFTP_LOG > <rundir>/tftp.txt"
        fi
        [ "$src" = mmc ] && [ "$PROBE" = 0 ] && b "sudo -n rm -f $STAGE_DIR/$file"
        echo "DRY host\$ the verdict line from the run directory (yakogl-tools record)"
        exit 0
    fi

    # 8. the cycle is over
    if [ "$BOARD" = am62p ]; then
        # the capture's scan says the log has a kernel start after the first SPL,
        # and either no image ran or the image's reset came before it
        until python3 "$HERE/am62p_netboot_uart.py" scan "$RD/console.ts" --back 2>/dev/null; do
            if [ $(($(date +%s) - T_REBOOT)) -gt "$RUN_BOUND" ]; then break; fi
            sleep 5
        done
        until timeout 8 ssh "${SSH_OPTS[@]}" -o ConnectTimeout=5 "$BOARD_HOST" true 2>/dev/null; do
            if [ $(($(date +%s) - T_REBOOT)) -gt $((RUN_BOUND + RETURN_BOUND)) ]; then rcc=5; break; fi
            sleep 5
        done
    else
        # a NEW boot answers: its uptime is shorter than the time since the reboot
        while :; do
            sleep 5
            el=$(($(date +%s) - T_REBOOT))
            up=$(timeout 8 ssh "${SSH_OPTS[@]}" -o ConnectTimeout=5 "$BOARD_HOST" 'cut -d. -f1 /proc/uptime' 2>/dev/null)
            if [ -n "$up" ] && [ "$up" -lt "$el" ]; then break; fi
            if [ "$el" -gt "$RUN_BOUND" ]; then rcc=5; break; fi
        done
        [ -n "$up" ] && meta up "$up"
    fi
    meta t_back "$(date +%s)"

    # 9. the one-shot must be gone; then what the server and the record say
    if [ "$rcc" != 5 ]; then
        meta back 1
        left=$(b 'sudo -n fw_printenv -n yako_once 2>/dev/null')
        if [ -n "$left" ]; then
            note "yako_once is still '$left' after the cycle: disarming" >&2
            b 'sudo -n fw_setenv yako_once'
            rcc=6
        fi
    fi
    [ -n "$rcc" ] && meta rc_cycle "$rcc"
    stop_cap
    if [ "$BOARD" = imx8mp ]; then
        if [ "$src" = net ]; then
            grep -F " file=$file " "$NETBOOT_TFTP_LOG" > "$RD/tftp.txt" 2>/dev/null
            [ -s "$RD/tftp.txt" ] || rm -f "$RD/tftp.txt"
        fi
        snvs_read > "$RD/snvs.txt"
        [ -s "$RD/snvs.txt" ] && [ "$(wc -w < "$RD/snvs.txt")" = 4 ] || rm -f "$RD/snvs.txt"
    else
        console_from_ts
    fi
    if [ "$src" = net ] && [ -f "$NETBOOT_TFTP_ROOT/$file" ]; then
        note "the image was never fetched; removed from the TFTP root" >&2
        rm -f "$NETBOOT_TFTP_ROOT/$file"
    fi
    if [ -n "${MMC_STAGED:-}" ]; then
        if [ "$rcc" = 5 ]; then
            meta cleanup "not done: Linux not back ($MMC_STAGED left on the eMMC)"
        elif b "sudo -n rm -f $MMC_STAGED && ! test -e $MMC_STAGED"; then
            meta cleanup "$MMC_STAGED removed (read back)"
        else
            meta cleanup "$MMC_STAGED NOT removed"
            note "warning: $MMC_STAGED was not removed" >&2
        fi
    fi
    finish
}

# ---- kexec-dev (am62p, imx8mp): development runs, never rows ----------------------
run_kexec() {
    RUN_BOUND=${RUN_BOUND:-1500}; RETURN_BOUND=${RETURN_BOUND:-600}
    if [ "$BOARD" = am62p ]; then
        HOLD_MAX=${HOLD_MAX:-$((RUN_BOUND + RETURN_BOUND + 300))}
        if [ "$DRY" = 1 ]; then BOARD_UART=${BOARD_UART:-BRIDGE:PORT}; fi
        : "${BOARD_UART:?set BOARD_UART (or UART_ADDR)=host:port of the serial bridge}"
        # kexec_run.sh copies this loader to the board; it is built per checkout
        # (driver/tools/kexec-zephyr/README.md) and never committed.
        [ "$DRY" = 1 ] || [ -x "$REPO/driver/tools/kexec-zephyr/kexec_zephyr" ] ||
            die "no loader $REPO/driver/tools/kexec-zephyr/kexec_zephyr (build it: driver/tools/kexec-zephyr/README.md)"
        take_lock
        common_prechecks
        kexec_am62p
    else
        # 1500 s for the board to come back, 600 s of settling, the rest
        HOLD_MAX=${HOLD_MAX:-$((1500 + 600 + 300))}
        take_lock
        common_prechecks
        kexec_imx8mp
    fi
}

kexec_am62p() {
    local try t0 rcc=""
    # 5. the capture: the bridge holds one client; "busy" is the previous one,
    #    retried up to 12 times 10 s apart
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ python3 tools/am62p_netboot_uart.py capture $BOARD_UART <rundir>/console.raw <rundir>/console.ts &   # 'busy' retried 12 x 10 s"
    else
        for try in $(seq 1 12); do
            python3 "$HERE/am62p_netboot_uart.py" capture "$BOARD_UART" "$RD/console.raw" "$RD/console.ts" &
            CAP=$!
            sleep 3
            if kill -0 "$CAP" 2>/dev/null; then break; fi
            wait "$CAP" 2>/dev/null
            CAP=
            note "bridge busy or unreachable (try $try of 12)" >&2
            sleep 10
        done
        [ -n "$CAP" ] || die "serial bridge $BOARD_UART busy or unreachable after 12 tries"
        trap 'stop_cap; rm -rf "$TMP"; [ -n "${BOARD_LOCK_WATCHDOG:-}" ] && kill "$BOARD_LOCK_WATCHDOG" 2>/dev/null' EXIT
    fi
    # 4.+6. kexec_run.sh copies the image and the loader, releases the GPU
    #    modules, checks the RTI watchdog and jumps; it exits 0 once the jump is
    #    issued and non-zero when anything before it failed
    t0=$(date +%s)
    meta t_reboot "$t0"
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ BOARD_HOST=\$BOARD_HOST sh driver/tools/kexec-zephyr/kexec_run.sh $IMG > <rundir>/kexec.log"
        echo "DRY host\$ wait for the end line ($END_RE) or a result yk-record: line, <= $RUN_BOUND s; then ssh, <= $RETURN_BOUND s more"
        exit 0
    fi
    if ! BOARD_HOST=$BOARD_HOST sh "$REPO/driver/tools/kexec-zephyr/kexec_run.sh" "$IMG" > "$RD/kexec.log" 2>&1; then
        stop_cap
        meta jump failed
        console_from_ts
        finish
    fi
    meta jump ok
    # 7. the end (or the RTI feeder's lifetime over: no summary can follow), then Linux
    while [ $(($(date +%s) - t0)) -lt "$RUN_BOUND" ]; do
        console_from_ts
        if end_seen "$RD/console.txt"; then break; fi
        sleep 10
    done
    until timeout 6 ssh -o ConnectTimeout=4 -o BatchMode=yes "$BOARD_HOST" true 2>/dev/null; do
        sleep 5
        if [ $(($(date +%s) - t0)) -gt $((RUN_BOUND + RETURN_BOUND)) ]; then rcc=5; break; fi
    done
    meta t_back "$(date +%s)"
    if [ -n "$rcc" ]; then meta rc_cycle "$rcc"; else meta back 1; fi
    stop_cap
    console_from_ts
    finish
}

# imx8mp_kexec_ready STATE...: the readiness words of an i.MX8MP crash-route
# jump, "<systemd> <uptime> <remoteproc0> <kexec_zephyr> <clients> <crash
# window> <kexec_crash_loaded>"; prints nothing and returns 0 when the board is
# ours to use, else prints the reason and returns 1. Each condition is the thing
# a wait used to stand for:
#   - systemd settled: is-system-running answered "running" or "degraded" (the
#     boot finished; "degraded" is a failed unit, recorded, not a refusal).
#     This replaces "uptime >= 180 s": the 2026-10-02 handoff's "uptime >= 3
#     min" named no mechanism, and the U-Boot route already waits on
#     is-system-running and nothing else.
#   - the M7 offline (remoteproc0), as before.
#   - no jump in progress: no kexec_zephyr process and kexec_crash_loaded 0.
#     This replaces "the staged ~/kexec/zephyr.bin is older than 200 s", which
#     also held back our own next run; the board lock (board_lock.sh) already
#     serialises board.sh's runs.
#   - nobody's GL client running, as before.
#   - the crash window present and large enough for the image (kexec_run.sh's
#     refusal).
# The uptime is recorded, not judged.
imx8mp_kexec_ready() {
    local sd=${1:-?} rp=${3:-?} nk=${4:-?} ng=${5:-?} cw=${6:-?} cl=${7:-?}
    case "$sd" in running | degraded) ;; *) echo "systemd is '$sd', not settled (is-system-running)"; return 1 ;; esac
    [ "$rp" = offline ] || { echo "remoteproc0 is $rp"; return 1; }
    { [ "$nk" = 0 ] && [ "$cl" = 0 ]; } || { echo "a jump is in progress (kexec_zephyr $nk, kexec_crash_loaded $cl)"; return 1; }
    [ "$ng" = 0 ] || { echo "something is running (clients $ng)"; return 1; }
    { [[ $cw =~ ^[0-9]+$ ]] && [ "$footprint" -le "$cw" ]; } ||
        { echo "the image needs $footprint bytes and the crash window is '$cw' (kexec_run.sh's refusal)"; return 1; }
    return 0
}

kexec_imx8mp() {
    local st why rsha t0 t lost="" pb="" sb="" ip rec
    # 2. the board is ours to use (imx8mp_kexec_ready); is-system-running
    #    --wait returns when the boot has finished, bounded at 300 s
    bget st 'timeout 300 systemctl is-system-running --wait 2>/dev/null || true; cut -d" " -f1 /proc/uptime; cat /sys/class/remoteproc/remoteproc0/state; pgrep -c -x kexec_zephyr || true; n=0; for q in /proc/[0-9]*; do [ "${q#/proc/}" = "$$" ] && continue; c=$(tr "\0" " " < $q/cmdline 2>/dev/null) || continue; case "$c" in *"for q in /proc"*) ;; *gl_test*|*deqp*|*labrun*) n=$((n+1));; esac; done; echo $n; cat /sys/kernel/kexec_crash_size; cat /sys/kernel/kexec_crash_loaded' \
        "running 999.0 offline 0 0 $((LOAD_END - YAKO_ADDR)) 0" || die "board unreachable"
    # shellcheck disable=SC2086  # seven words
    set -- $st
    meta board_state "systemd ${1:-?} uptime ${2%.*}s remoteproc0=${3:-?} kexec=${4:-?} clients=${5:-?} crash_window=${6:-?} crash_loaded=${7:-?}"
    # shellcheck disable=SC2086  # seven words
    why=$(imx8mp_kexec_ready $st) || die "$why"
    # 3. the record, 4. stage and read back
    snvs_clear
    act scp -q -o BatchMode=yes "$IMG" "$BOARD_HOST:kexec/zephyr.bin" || die "scp failed"
    bget rsha 'sync; sha256sum ~/kexec/zephyr.bin' "$IMG_SHA  zephyr.bin"
    [ "${rsha%% *}" = "$IMG_SHA" ] || die "the staged image reads back as '${rsha%% *}'"
    meta staged "~/kexec/zephyr.bin, read back equal"
    # 6. jump through the board's own sudo wrapper, bounded
    t0=$(date +%s)
    meta t_reboot "$t0"
    if [ "$DRY" = 1 ]; then
        b 'sync; sync; sudo -n /usr/local/sbin/kexec-zephyr 0x60000000 --crash   # timeout 120'
        echo "DRY host\$ wait: ping lost, ping back, ssh back (<= 1500 s); then read the SNVS record and the uptime"
        exit 0
    fi
    timeout 120 ssh -o BatchMode=yes -o ConnectTimeout=5 -o ServerAliveInterval=3 -o ServerAliveCountMax=2 "$BOARD_HOST" \
        'sync; sync; sudo -n /usr/local/sbin/kexec-zephyr 0x60000000 --crash' < /dev/null >> "$RD/kexec.log" 2>&1 || true
    meta jump ok
    # 7. Linux: ping lost, ping back, ssh back (cap 1500 s: the image's watchdog cap is 600 s)
    ip=${BOARD_HOST#*@}
    while :; do
        t=$(($(date +%s) - t0))
        if [ "$t" -gt 1500 ]; then break; fi
        if ping -c1 -W1 "$ip" > /dev/null 2>&1; then
            [ -n "$lost" ] && [ -z "$pb" ] && pb=$t
            if [ -n "$pb" ] && timeout 10 ssh -o BatchMode=yes -o ConnectTimeout=5 "$BOARD_HOST" true < /dev/null 2>/dev/null; then
                sb=$t
                break
            fi
        else
            [ -z "$lost" ] && lost=$t
        fi
        sleep 2
    done
    meta lost "$lost"
    meta timeline "jump 0s, ping lost ${lost:-?}s, ping back ${pb:-?}s, ssh back ${sb:-?}s, route --crash"
    if [ -n "$sb" ]; then
        meta back 1
        meta t_back "$(date +%s)"
        rec=$(snvs_read)
        [ -n "$rec" ] && echo "$rec" > "$RD/snvs.txt"
        meta up "$(b "cut -d' ' -f1 /proc/uptime")"
    else
        meta rc_cycle 5
    fi
    finish
}

# ---- run / probe ---------------------------------------------------------------------
cmd_run() {
    local sha8
    PROBE=0
    if [ "$VERB" = probe ]; then
        [ $# -ge 1 ] || usage
        PROBE=1; IMG=; LOGDIR=$1; NAME=probe
    else
        [ $# -ge 2 ] || usage
        IMG=$1; LOGDIR=$2; NAME=${3:-$(basename "$IMG" .bin)}
    fi
    [[ $NAME =~ ^[A-Za-z0-9_-]+$ ]] || die "name '$NAME': letters, digits, _ and - only" 2
    need_board
    if [ "$PROBE" = 1 ]; then
        IMG_SHA=none; IMG_SIZE=0; footprint=none; sha8=probe
        IMG_BUILD="absent (probe: no image)"; IMG_ZEPHYR="absent (probe: no image)"
    else
        check_image "$IMG"
        sha8=${IMG_SHA:0:8}
    fi
    RD="$LOGDIR/$(date +%Y%m%d-%H%M)-$NAME-$sha8"
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ mkdir -p $RD   # meta.txt, console.txt, INDEX.txt"
    else
        [ -e "$RD" ] && die "$RD exists: a record is never overwritten"
        mkdir -p "$RD" || die "cannot create $RD"
    fi
    meta board "$BOARD"
    meta boot "$BOOT"
    meta name "$NAME"
    meta probe "$PROBE"
    meta image "${IMG:+$(basename "$IMG")}"
    meta image_sha256 "$IMG_SHA"
    meta image_size "$IMG_SIZE"
    meta footprint "${footprint:-none}"
    meta image_build "$IMG_BUILD"
    meta image_zephyr "$IMG_ZEPHYR"
    meta started "$(date '+%F %T')"
    note "$NAME: build $(git -C "$REPO" describe --always --dirty 2>/dev/null || echo unknown) boot $BOOT ($ROW) image sha256 ${IMG_SHA:0:16} footprint ${footprint:-none}"
    note "$NAME: image build: $IMG_BUILD; zephyr: $IMG_ZEPHYR"
    if [ "$PROBE" = 1 ] && [ "${BOOT#uboot-}" = "$BOOT" ]; then
        # No image runs and the board is not rebooted: the pre-checks only.
        HOLD_MAX=${HOLD_MAX:-300}
        take_lock
        common_prechecks
        case "$BOOT" in
        tryboot)
            STAGED="yakogl-probe.bin"
            if [ "$DRY" = 1 ]; then BOARD_UART=${BOARD_UART:-/dev/PI4_UART}; fi
            : "${BOARD_UART:?set BOARD_UART (or PI4_UART)=the serial device wired to the Pi}"
            pi4_prechecks
            ;;
        kexec-dev)
            local st
            bget st 'cut -d" " -f1 /proc/uptime; cat /sys/kernel/kexec_loaded'
            ;;
        esac
        note "probe: the pre-checks passed; no image ran, the board was not rebooted"
        exit 0
    fi
    case "$BOOT" in
    tryboot)
        if [ "$DRY" = 1 ]; then BOARD_UART=${BOARD_UART:-/dev/PI4_UART}; fi
        : "${BOARD_UART:?set BOARD_UART (or PI4_UART)=the serial device wired to the Pi}"
        run_tryboot
        ;;
    uboot-tftp | uboot-mmc) run_uboot ;;
    kexec-dev) run_kexec ;;
    esac
}

# ---- status / install / uninstall / disarm -----------------------------------------
cmd_status() {
    need_board
    case "$BOOT" in
    uboot-tftp) nb_status ;;
    uboot-mmc)
        nb_status
        b "ls -l $STAGE_DIR 2>/dev/null; findmnt -n -v -o SOURCE,TARGET,FSROOT -T /var"
        ;;
    tryboot)
        b 'cat /proc/sys/kernel/random/boot_id; who; df -B1 --output=avail /boot/firmware | tail -1; ls -l /boot/firmware/yakogl-*.bin /boot/firmware/pi4run-*.bin 2>/dev/null; cat /boot/firmware/tryboot.txt 2>/dev/null'
        ;;
    kexec-dev)
        if [ "$BOARD" = imx8mp ]; then
            b 'cut -d" " -f1 /proc/uptime; cat /sys/class/remoteproc/remoteproc0/state /sys/kernel/kexec_loaded /sys/kernel/kexec_crash_loaded /sys/kernel/kexec_crash_size; ls -l ~/kexec 2>/dev/null'
        else
            b 'cut -d" " -f1 /proc/uptime; cat /sys/kernel/kexec_loaded; ls -l ~/kexec_zephyr ~/zephyr.bin ~/kexec 2>/dev/null'
        fi
        ;;
    esac
}

cmd_setup() {
    need_board
    case "$BOOT:$VERB" in
    uboot-*:install) nb_install ;;
    uboot-*:uninstall) nb_uninstall ;;
    uboot-tftp:disarm) nb_disarm ;;
    uboot-mmc:disarm)
        b "sudo -n rm -f $STAGE_DIR/once-*.bin; ls $STAGE_DIR 2>/dev/null"
        nb_disarm ;;
    tryboot:install | tryboot:uninstall)
        note "tryboot needs no install: config.txt stays the Linux default; each run writes tryboot.txt and puts it back" ;;
    tryboot:disarm)
        # nothing persists: the one-shot flag is set only by `reboot "0 tryboot"`
        b 'ls -l /boot/firmware/yakogl-*.bin /boot/firmware/pi4run-*.bin 2>/dev/null; cat /boot/firmware/tryboot.txt 2>/dev/null'
        note "tryboot arms nothing that persists (only 'reboot \"0 tryboot\"' sets the one-shot); a staged image listed above is a run that did not clean up: remove it by hand" ;;
    kexec-dev:install | kexec-dev:uninstall)
        note "kexec-dev needs no install: the loader and the image are copied by each run (driver/tools/kexec-zephyr)" ;;
    kexec-dev:disarm)
        b 'cat /sys/kernel/kexec_loaded /sys/kernel/kexec_crash_loaded'
        note "kexec-dev arms nothing that persists across a reboot; a loaded image (1 above) is someone's pending jump: find its owner" ;;
    esac
}

# ---- default / nodefault: the permanent boot (docs/standalone-boot.md) ---------------
uboot_default() {
    local st got w
    if [ "$DRY" = 0 ]; then
        st=$(check_install)
        [ "$st" = ok ] || die "the U-Boot hook is not installed as tools/uboot-netboot-lib.sh writes it ($st): 'install' first"
        st=$(b 'sudo -n fw_printenv -n yako_once 2>/dev/null')
        [ -z "$st" ] || die "a one-shot is pending (yako_once=$st); find its owner, or 'disarm'"
    else
        b 'sudo -n fw_printenv   # the install must match the library; yako_once must be absent'
    fi
    mmc_where
    mmc_stage default.bin
    {
        printf 'yako_default=%s\n' "$UB_DIR/default.bin"
        # the count goes in the same write: never a moment with an unbounded default
        if [ "$BOOTS_N" = forever ]; then printf 'yako_default_left=\n'; else printf 'yako_default_left=%s\n' "$BOOTS_N"; fi
    } > "$TMP/default"
    setenv_script "$TMP/default" || die "fw_setenv failed; the env is as it was (one write)" 5
    w="$UB_DIR/default.bin"
    [ "$BOOTS_N" = forever ] || w+=$'\n'"$BOOTS_N"
    bget got 'sudo -n fw_printenv -n yako_default; sudo -n fw_printenv -n yako_default_left 2>/dev/null' "$w"
    [ "$got" = "$w" ] || die "read-back of yako_default and its count: '$got', not '$w'" 5
    if [ "$BOOTS_N" = forever ]; then
        note "default set: from the next boot on, U-Boot loads $UB_DIR/default.bin from mmc $NB_MMC at every boot. Nothing was rebooted."
    else
        note "default set for $BOOTS_N boots: U-Boot loads $UB_DIR/default.bin at each of the next $BOOTS_N boots, then deletes the default and boots Linux. Nothing was rebooted."
    fi
}

uboot_nodefault() {
    local got
    printf 'yako_default=\nyako_default_left=\n' > "$TMP/nodefault"
    setenv_script "$TMP/nodefault" || die "fw_setenv failed" 5
    bget got 'sudo -n fw_printenv -n yako_default 2>/dev/null' ''
    [ -z "$got" ] || die "yako_default is still '$got'" 5
    b "sudo -n rm -f $STAGE_DIR/default.bin"
    note "no default: U-Boot boots the saved bootcmd (Linux) again"
}

pi4_default() {
    local avail tmpd got cfg
    STAGED=yakogl-default.bin
    bget avail 'df -B1 --output=avail /boot/firmware | tail -1' 999999999999 || die "df on the board failed"
    avail=${avail// /}
    [[ $avail =~ ^[0-9]+$ ]] || die "cannot read the free space of /boot/firmware ('$avail')"
    [ "$avail" -ge $((2 * IMG_SIZE)) ] || die "/boot/firmware has $avail bytes free, less than twice the image ($IMG_SIZE)"
    if [ "$DRY" = 1 ]; then
        b 'test ! -e /boot/firmware/config-linux.txt   # a default already set refuses'
    else
        b 'test ! -e /boot/firmware/config-linux.txt' || die "a default is already set (/boot/firmware/config-linux.txt exists): 'nodefault' first"
    fi
    bget tmpd 'mktemp -d' /tmp/DRY.board
    [[ $tmpd == /tmp/* ]] || die "mktemp -d on the board failed"
    b "cat > $tmpd/img" < "$IMG" || { b "rm -rf $tmpd"; die "copy to the board failed"; }
    b "sudo install -m 0755 $tmpd/img /boot/firmware/$STAGED; r=\$?; rm -rf $tmpd; sync; exit \$r" ||
        { b "sudo rm -f /boot/firmware/$STAGED"; die "install into /boot/firmware failed (removed again)"; }
    bget got "sha256sum /boot/firmware/$STAGED" "$IMG_SHA  /boot/firmware/$STAGED"
    [ "${got%% *}" = "$IMG_SHA" ] || { b "sudo rm -f /boot/firmware/$STAGED"; die "the copy reads back as '${got%% *}', not $IMG_SHA (removed)"; }
    # Linux's config.txt is kept beside it, byte for byte, before config.txt changes
    b 'sudo cp /boot/firmware/config.txt /boot/firmware/config-linux.txt && sync && cmp /boot/firmware/config.txt /boot/firmware/config-linux.txt' ||
        { b "sudo rm -f /boot/firmware/$STAGED /boot/firmware/config-linux.txt"; die "keeping config.txt as config-linux.txt failed (nothing changed)"; }
    cfg="# tools/board.sh: the permanent default (docs/standalone-boot.md); Linux's is config-linux.txt
$(pi4_config "$STAGED")"
    b 'sudo tee /boot/firmware/config.txt >/dev/null && sync' <<<"$cfg" ||
        { pi4_nodefault; die "writing config.txt failed (Linux's put back)"; }
    bget got 'cat /boot/firmware/config.txt' "$cfg"
    [ "$got" = "$cfg" ] || { pi4_nodefault; die "config.txt reads back different (Linux's put back)"; }
    note "default set: from the next boot on, the firmware boots /boot/firmware/$STAGED, every boot. Nothing was rebooted."
}

pi4_nodefault() {
    if [ "$DRY" = 1 ]; then
        b 'test -e /boot/firmware/config-linux.txt   # no default set: nothing to undo'
    else
        b 'test -e /boot/firmware/config-linux.txt' || { note "no default set (no /boot/firmware/config-linux.txt)"; return 0; }
    fi
    b 'sudo mv /boot/firmware/config-linux.txt /boot/firmware/config.txt && sudo rm -f /boot/firmware/yakogl-default.bin && sync' ||
        die "restoring config.txt failed: look at /boot/firmware by hand" 5
    note "no default: config.txt is Linux's again"
}

# default_undo: the exact undo of "default", printed before anything changes
default_undo() {
    note "UNDO, from Linux (before the reboot, or once Linux is back): tools/board.sh $BOARD nodefault"
    case "$BOARD:$BOOTS_N" in
    pi4:*)
        note "UNDO, without Linux (a person): the SD card in a PC; on its FAT partition copy config-linux.txt over config.txt" ;;
    *:forever)
        note "UNDO, without Linux (a person at the U-Boot prompt$([ "$BOARD" = am62p ] && echo ", or the serial bridge: docs/standalone-boot.md")): setenv yako_default; saveenv; boot" ;;
    *)
        note "UNDO, without Linux: none needed: after $BOOTS_N boots that end in the image's watchdog reset, U-Boot deletes the default and boots Linux" ;;
    esac
}

cmd_default() {
    need_board
    RD=$TMP   # meta() writes here; nothing of it is kept
    if [ "$VERB" = default ]; then
        [ $# -eq 2 ] || usage
        IMG=$1 BOOTS_N=$2
        case "$BOARD:$BOOTS_N" in
        pi4:forever | am62p:forever | imx8mp:forever) ;;
        pi4:*) die "pi4: the firmware has no boot count: 'forever' only, with a person (docs/standalone-boot.md)" 2 ;;
        *:[1-9]) ;;
        *) die "boots must be 1-9 or forever, not '$BOOTS_N'" 2 ;;
        esac
        if [ "$BOOTS_N" = forever ] && [ "${BOARD_PERSON_PRESENT:-0}" != 1 ]; then
            die "'forever' makes the image every boot's: Linux comes back only through a person (docs/standalone-boot.md). Set BOARD_PERSON_PRESENT=1 when one is at the board$([ "$BOARD" != pi4 ] && echo ", or give a boot count (1-9): the board then returns to Linux by itself")."
        fi
        check_image "$IMG"
        default_undo
    else
        [ $# -eq 0 ] || usage
    fi
    HOLD_MAX=${HOLD_MAX:-900}
    take_lock
    trap 'rm -rf "$TMP"; [ -n "${BOARD_LOCK_WATCHDOG:-}" ] && kill "$BOARD_LOCK_WATCHDOG" 2>/dev/null' EXIT
    common_prechecks
    case "$BOARD:$VERB" in
    pi4:default) pi4_default ;;
    pi4:nodefault) pi4_nodefault ;;
    *:default) uboot_default ;;
    *:nodefault) uboot_nodefault ;;
    esac
    exit 0
}

# ---- tree ----------------------------------------------------------------------------
# tree_rows TABLE: "name<TAB>image<TAB>selector<TAB>filter" per row, in order
tree_rows() {
    local table=$1
    if [ -n "${TREE_IMAGE:-}" ]; then
        [ "$BOARD" = imx8mp ] || die "TREE_IMAGE (one image, the row picked by the selector) needs a board with a selector: imx8mp" 2
        awk -v img="$TREE_IMAGE" '/^[[:space:]]*(#|$)/ { next } { printf "e%02d\t%s\t%d\t%s\n", n, img, n, $0; n++ }' "$table"
    elif [ -n "${TREE_IMAGE_DIR:-}" ]; then
        awk -v dir="$TREE_IMAGE_DIR" -v tag="${TREE_TAG:-}" '
            /^# image [A-Za-z0-9]+:/ { l = $3; sub(/:$/, "", l); next }
            /^[[:space:]]*(#|$)/ { next }
            { n++; lab = (l != "") ? l : "img" n "-" tag; l = ""
              printf "%s\t%s/%s/zephyr/zephyr.bin\t-\t%s\n", lab, dir, lab, $0 }' "$table"
    else
        awk '/^[[:space:]]*(#|$)/ { next } { printf "%s\t%s\t%s\t-\n", $1, $2, ($3 == "" ? "-" : $3) }' "$table"
    fi
}

in_rows() { # in_rows INDEX SPEC: SPEC like 3,5-9 (empty: every row)
    local i=$1 spec=${2:-} part a b parts=()
    [ -z "$spec" ] && return 0
    IFS=, read -ra parts <<<"$spec"
    for part in "${parts[@]}"; do
        a=${part%-*}; b=${part#*-}
        [ "$i" -ge "$a" ] && [ "$i" -le "$b" ] && return 0
    done
    return 1
}

# finished_run OUTDIR NAME IMAGE: the directory of NAME's finished run of IMAGE
finished_run() {
    local out=$1 name=$2 img=$3 sha8="" d
    [ -f "$img" ] && sha8=$(sha256sum "$img" | cut -c1-8)
    for d in "$out"/*/; do
        is_run_of "$name" "$d" "$sha8" && [ -f "$d/INDEX.txt" ] && { echo "${d%/}"; return 0; }
    done
    return 1
}

# builder_ok TREE_IMAGE_DIR LABEL N: wait for a builder's verdict on the image
# (pi4-deqp-images.sh's <label>.built, am62p-deqp-images.sh's build.out); 0 = built
builder_ok() {
    local dir=$1 lab=$2 n=$3 rc
    if compgen -G "$dir/*.built" > /dev/null; then
        while [ ! -s "$dir/$lab.built" ]; do sleep 10; done
        read -r _ rc _ < "$dir/$lab.built"
        [ "$rc" = rc=0 ] || { note "$lab: not built ($rc), skipped: see $dir/$lab.log"; return 1; }
    elif [ -f "$dir/build.out" ]; then
        until grep -q "^img$n rc=" "$dir/build.out"; do sleep 20; done
        grep -q "^img$n rc=0 " "$dir/build.out" || { note "img$n build failed: skipped"; return 1; }
    fi
    return 0
}

# tree_run_one NAME IMAGE SEL OUTDIR: one run of the chain, with the retry rule
tree_run_one() {
    local name=$1 img=$2 sel=$3 out=$4 r tries=0 d
    local retries=${TREE_RETRIES:-$TREE_RETRIES_DEFAULT} wait=${TREE_RETRY_S:-30}
    if [ "$DRY" = 0 ] && d=$(finished_run "$out" "$name" "$img"); then
        note "$name: finished run $d, skipped"
        return 0
    fi
    while :; do
        echo "=== $name ($(basename "$img")${sel:+, selector $sel}) $(date '+%F %T')"
        if [ "$sel" != - ]; then
            SEL=$sel "$SELF" "${DRYFLAG[@]+"${DRYFLAG[@]}"}" "$BOARD" run "$img" "$out" "$name" < /dev/null
        else
            env -u SEL "$SELF" "${DRYFLAG[@]+"${DRYFLAG[@]}"}" "$BOARD" run "$img" "$out" "$name" < /dev/null
        fi
        r=$?
        echo "=== $name rc=$r $(date '+%F %T')"
        case "$r" in
        3)
            # a dry run shows the whole table; it never waits
            [ "$DRY" = 1 ] && return 0
            tries=$((tries + 1))
            [ "$tries" -le "$retries" ] || { note "$name refused; the chain stops"; exit 3; }
            note "$name refused (try $tries of $retries); again in $wait s"
            sleep "$wait"
            ;;
        5) note "$name: the board did not come back; the chain stops (never keep booting)"; exit 5 ;;
        *) return 0 ;;
        esac
    done
}

# name_pages NAME IMAGE SEL OUTDIR: the selector form's DEQP_FAILS pages for a
# row with a failure, another status or a case without a result (six names
# per boot, pages until 6 * (page + 1) >= the fails seen)
name_pages() {
    local name=$1 img=$2 sel=$3 out=$4 d words t tp tf tn to tb nr page low
    d=$(finished_run "$out" "$name" "$img") || return 0
    words=$(cat "$d/snvs.txt" 2>/dev/null)
    [ -n "$words" ] || return 0
    # shellcheck disable=SC2086  # four words
    t=$(ytools record --line $words | sed -n 's/.* tally=//p')
    { [ -n "$t" ] && [ "$t" != - ]; } || return 0
    IFS=, read -r tp tf tn to tb <<<"$t"
    nr=$((tb - tp - tf - tn - to))
    [ $((tf + to + nr)) -gt 0 ] || return 0
    for page in $(seq 0 39); do
        tree_run_one "$name-p$page" "$img" $((0x8000 | page << 8 | sel)) "$out"
        d=$(finished_run "$out" "$name-p$page" "$img") || break
        words=$(cat "$d/snvs.txt" 2>/dev/null)
        [ -n "$words" ] && [ $(((0x${words%% *} >> 8) & 255)) = 10 ] || break
        low=$((0x${words%% *} & 255))
        [ $((6 * (page + 1))) -ge "$low" ] && break
    done
}

cmd_tree() {
    [ $# -eq 2 ] || usage
    local table=$1 out=$2 i=0 name img sel filt rows
    [ -f "$table" ] || die "no table $table" 2
    rows=$(tree_rows "$table") || exit $?
    [ -n "$rows" ] || die "the table $table has no rows" 2
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ mkdir -p $out; write $out/tree.txt ($(grep -c . <<<"$rows") rows)"
    else
        mkdir -p "$out" || die "cannot create $out"
        printf '%s\n' "$rows" > "$out/tree.txt"
    fi
    note "tree: $(grep -c . <<<"$rows") rows from $table, build $(git -C "$REPO" describe --always --dirty 2>/dev/null || echo unknown)"
    while IFS=$'\t' read -r name img sel filt; do
        if in_rows "$i" "${TREE_ROWS:-}"; then
            if [ -n "${TREE_IMAGE_DIR:-}" ] && [ "$DRY" = 0 ] && ! builder_ok "$TREE_IMAGE_DIR" "$name" $((i + 1)); then
                i=$((i + 1))
                continue
            fi
            tree_run_one "$name" "$img" "$sel" "$out"
            if [ "$sel" != - ] && [ "${NAMES:-1}" != 0 ] && [ "$DRY" = 0 ]; then name_pages "$name" "$img" "$sel" "$out"; fi
        fi
        i=$((i + 1))
    done <<<"$rows"
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ tools/board.sh $BOARD tally $out${TREE_PREV:+; tools/board.sh $BOARD diff $TREE_PREV $out}"
        exit 0
    fi
    "$SELF" "$BOARD" tally "$out"
    [ -n "${TREE_PREV:-}" ] && "$SELF" "$BOARD" diff "$TREE_PREV" "$out"
    echo TREE-EXIT
}

# ---- tally / diff -----------------------------------------------------------------------
# run_dirs DIR: the run directories of a tree, in the tree's order (tree.txt,
# each row followed by its DEQP_FAILS pages), else in name order
run_dirs() {
    local dir=$1 name d
    if [ -f "$dir/tree.txt" ]; then
        while IFS=$'\t' read -r name _; do
            for d in "$dir"/*/; do is_run_of "$name" "$d" && [ -f "$d/meta.txt" ] && echo "${d%/}"; done
            for d in "$dir"/*/; do
                [[ ${d%/} =~ -$name-p[0-9]+-([0-9a-f]{8}|probe)$ ]] && [ -f "$d/meta.txt" ] && echo "${d%/}"
            done
        done < "$dir/tree.txt"
    else
        for d in "$dir"/*/; do [ -f "$d/meta.txt" ] && echo "${d%/}"; done
    fi
}

# py_list A B ...: Python's repr of a list of strings, as the old i.MX8MP tree runner printed them
py_list() { local s="" x; for x in "$@"; do s+="${s:+, }'$x'"; done; echo "[$s]"; }

# results_tsv DIR: the selector form's per-row results, as the old i.MX8MP tree runner
# wrote them (results.tsv, rows.md, the TALLY line): begun passed failed notsup
# other noresult and the failing names; '-' is absent, never 0
results_tsv() {
    local dir=$1 name img sel filt d words t tp tf tn to tb nr page c w p f n inside joined fs=() names=()
    printf '# index\tfilter\tbegun\tpassed\tfailed\tnotsup\tother\tnoresult\tfailing names (%s-separated)\n' "';'" > "$dir/results.tsv"
    while IFS=$'\t' read -r name img sel filt; do
        d=$(finished_run "$dir" "$name" "$img") || d=
        words=
        [ -n "$d" ] && words=$(cat "$d/snvs.txt" 2>/dev/null)
        t=
        # shellcheck disable=SC2086  # four words
        [ -n "$words" ] && t=$(ytools record --line $words | sed -n 's/.* tally=//p')
        if [ -z "$t" ] || [ "$t" = - ]; then
            echo "$name: no usable record ${words:-absent}"
            printf '%s\t%s\t-\t-\t-\t-\t-\t-\t-\n' "$sel" "$filt" >> "$dir/results.tsv"
            continue
        fi
        IFS=, read -r tp tf tn to tb <<<"$t"
        nr=$((tb - tp - tf - tn - to))
        names=()
        IFS=, read -ra fs <<<"$filt"
        for page in $(seq 0 39); do
            p=$(finished_run "$dir" "$name-p$page" "$img") || break
            words=$(cat "$p/snvs.txt" 2>/dev/null)
            if [ -z "$words" ] || [ $(((0x${words%% *} >> 8) & 255)) != 10 ]; then names+=("PAGE$page-NOT-KIND-10"); break; fi
            for w in ${words#* }; do
                for c in $(printf '%04x %04x' $((0x$w & 0xffff)) $((0x$w >> 16))); do
                    [ "$c" = ffff ] && continue
                    # the names inside the row's filters (a filter's last character, its '*', dropped)
                    inside=$(ytools record crc "$c" | while read -r n; do
                        for f in "${fs[@]}"; do [[ $n == "${f:0:${#f}-1}"* ]] && { echo "$n"; break; }; done
                    done | paste -sd, -)
                    names+=("${inside:-crc:$c}")
                done
            done
        done
        joined=$(IFS=';'; echo "${names[*]}")
        printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$sel" "$filt" "$tb" "$tp" "$tf" "$tn" "$to" "$nr" "${joined:--}" >> "$dir/results.tsv"
        echo "$name: begun $tb pass $tp fail $tf ns $tn other $to no-result $nr"
    done < "$dir/tree.txt"
    awk -F'\t' '!/^#/ { n++; for (c = 3; c <= 8; c++) if ($c != "-") s[c] += $c }
        END { printf "TALLY (%d entries): begun %d passed %d failed %d notsup %d other %d noresult %d\n", n, s[3], s[4], s[5], s[6], s[7], s[8] }' \
        "$dir/results.tsv"
    {
        echo '| entry | begun | pass | fail | notsup | other | no result | failing names |'
        echo '|---|---|---|---|---|---|---|---|'
        awk -F'\t' '!/^#/ { printf "| %s |", $1; for (c = 3; c <= 9; c++) printf " %s |", $c; printf "\n" }' "$dir/results.tsv"
    } > "$dir/rows.md"
}

# consoles_with_deqp DIRS...: the console.txt of every run that printed dEQP lines
consoles_with_deqp() {
    local d
    for d in "$@"; do
        [ -f "$d/console.txt" ] && grep -aq 'deqp: ' "$d/console.txt" && echo "$d/console.txt"
    done
}

cmd_tally() {
    [ $# -eq 1 ] || usage
    local dir=$1 d c rv runs=() cons=()
    if [ -f "$dir/meta.txt" ]; then
        verdict_line "$dir"
        echo "$V_LINE"
        if [ -f "$dir/INDEX.txt" ]; then
            if grep -qxF "verdict: $V_LINE" "$dir/INDEX.txt"; then echo "  (INDEX.txt says the same)"
            else echo "  DIFFERS from INDEX.txt: $(sed -n 's/^verdict: //p' "$dir/INDEX.txt")"; fi
        fi
        exit "$V_RC"
    fi
    mapfile -t runs < <(run_dirs "$dir")
    [ ${#runs[@]} -gt 0 ] || die "no runs in $dir (no */meta.txt)" 2
    echo "== per run (the verdict line, recomputed from each run's files)"
    for d in "${runs[@]}"; do verdict_line "$d"; echo "$V_LINE"; done
    if [ -f "$dir/tree.txt" ] && [ "$(cut -f3 "$dir/tree.txt" | head -1)" != - ]; then
        echo "== results per row (the selector form)"
        results_tsv "$dir"
        return 0
    fi
    mapfile -t cons < <(consoles_with_deqp "${runs[@]}")
    [ ${#cons[@]} -gt 0 ] || { echo "== no dEQP lines in these consoles"; return 0; }
    echo "== per record (deqp-uart-tally.py)"
    for c in "${cons[@]}"; do python3 "$HERE/cts/deqp-uart-tally.py" "$c" | head -1; done
    echo "== recoveries per record (the OSAL's diag line; GPU reset lines)"
    for c in "${cons[@]}"; do
        rv=$(grep -ao 'recoveries: [0-9]*' "$c" | tail -1)
        printf '%-28s %s; GPU reset lines: %s\n' "$(basename "$(dirname "$c")")" "${rv:-no recoveries line}" \
            "$(grep -ac 'GPU reset (recovery' "$c")"
    done
    python3 "$HERE/cts/deqp-uart-tally.py" --mustpass "${BOARD_MUSTPASS:-$HERE/cts/gles2-mustpass.txt}" --table "${cons[@]}"
}

cmd_diff() {
    [ $# -eq 2 ] || usage
    local a=$1 b=$2 base ra=() rb=() ca=() cb=()
    if [ -f "$a/results.tsv" ] && [ -f "$b/results.tsv" ]; then
        echo "DIFF against $a:"
        awk -F'\t' -v q="'" '
            function lst(f, from, to,   s, c) { s = ""; for (c = from; c <= to; c++) s = s (s == "" ? "" : ", ") q f[c] q; return "[" s "]" }
            FNR == NR { if (!/^#/) old[$1] = $0; next }
            /^#/ { next }
            { n++; split($0, nw, FS); now = lst(nw, 3, 8); nown = lst(nw, 9, NF)
              if ($1 in old) { split(old[$1], o, FS); was = lst(o, 3, 8); wasn = lst(o, 9, length(o)) }
              if (($1 in old) && was == now && wasn == nown) same++
              else printf "  e%02d now %s (names %s) was %s (names %s)\n", $1, now, nown,
                   ($1 in old) ? was : "absent", ($1 in old) ? wasn : "absent" }
            END { printf "  %d of %d entries identical\n", same, n }' "$a/results.tsv" "$b/results.tsv"
        return 0
    fi
    mapfile -t ra < <(run_dirs "$a")
    mapfile -t rb < <(run_dirs "$b")
    mapfile -t ca < <(consoles_with_deqp "${ra[@]}")
    mapfile -t cb < <(consoles_with_deqp "${rb[@]}")
    { [ ${#ca[@]} -gt 0 ] && [ ${#cb[@]} -gt 0 ]; } || die "diff needs dEQP consoles (or results.tsv) in both $a and $b" 2
    base=$TMP/statuses-a.txt
    python3 "$HERE/cts/deqp-uart-tally.py" --mustpass "${BOARD_MUSTPASS:-$HERE/cts/gles2-mustpass.txt}" --statuses "${ca[@]}" > "$base" ||
        die "cannot read the statuses of $a"
    python3 "$HERE/cts/deqp-uart-tally.py" --mustpass "${BOARD_MUSTPASS:-$HERE/cts/gles2-mustpass.txt}" --baseline "$base" "${cb[@]}" |
        sed "s|against $base|of $b against $a|"
}

case "$VERB" in
status) cmd_status ;;
install | uninstall | disarm) cmd_setup ;;
probe | run) cmd_run "$@" ;;
default | nodefault) cmd_default "$@" ;;
tree) cmd_tree "$@" ;;
tally) cmd_tally "$@" ;;
diff) cmd_diff "$@" ;;
*) usage ;;
esac

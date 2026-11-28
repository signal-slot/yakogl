#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The i.MX8MP's (Verdin iMX8M Plus, Vivante GC7000UL) glmark2 scene pictures by
# eye: one frame-dump image per scene (CONFIG_GLMARK2_FRAME_DUMP=y, the scene
# alone in CONFIG_GLMARK2_ARGS; driver/docs/glmark2.md, "The frame dump"), run
# on the board with tools/board.sh and decoded to a PNG with
# tools/cts/glmark2-fb.py. The scenes are the imx8mp column of
# tools/cts/glmark2-pictures.txt (33). The counterpart of
# tools/cts/pi4-glmark2-dumps.sh.
#
#   tools/cts/imx8mp-glmark2-dumps.sh scenes
#   tools/cts/imx8mp-glmark2-dumps.sh [--dry-run] build <img-dir> [SCENE...]
#   tools/cts/imx8mp-glmark2-dumps.sh [--dry-run] run <img-dir> <records-dir> [SCENE...]
#   tools/cts/imx8mp-glmark2-dumps.sh take <img-dir> <record> [SCENE...]
#   tools/cts/imx8mp-glmark2-dumps.sh --self-test
#
# SCENE      a scene's number (1-33, as `scenes` lists them), its label
#            (gm01-build, ...) or its glmark2 -b argument without the "-b "
#            (texture:texture-filter=mipmap), an exact match that must name
#            one scene: all scenes when none.
# scenes     "<label> <scene info string> -b <glmark2 argument>" per scene.
# build      one west build directory per scene, <img-dir>/<label>/, board
#            verdin_imx8mp/mimx8ml8/a53, from this checkout as it is (build it
#            from an origin/main commit): the glmark2 sample with
#            -DCONFIG_GLMARK2_FRAME_DUMP=y and -DCONFIG_GLMARK2_ARGS="-b <scene>"
#            (a `;` of an effect2d kernel written `\;`: unescaped, CMake drops
#            it and the image runs another kernel), the watchdog arm of
#            tools/cts/common-images.sh (CONFIG_GC_BOARD_WDOG_SELF_ARM=y; never
#            an image without it) and CONFIG_GC_BOARD_WDOG_SECONDS=$WDOG_S.
#            Console: the sample's Verdin conf draws the console on the panel
#            (UART_CONSOLE=n), so a dump over the UART needs the board's UART3
#            console: IMX8MP_DUMP_CONSOLE=uart (default) adds
#            -DEXTRA_CONF_FILE=tools/cts/imx8mp-console.conf (CONFIG_SERIAL,
#            CONSOLE, UART_CONSOLE, PRINTK, STDOUT_CONSOLE: UART_CONSOLE
#            depends on SERIAL and CONSOLE, which the sample's prj.conf does
#            not set, so a bare -DCONFIG_UART_CONSOLE=y may be dropped by
#            Kconfig with a warning) and -DCONFIG_GC_COMMON_PANEL_CONSOLE=n;
#            IMX8MP_DUMP_CONSOLE=panel leaves the sample's conf (a camera photograph of the panel, no UART value). A
#            directory that exists is refused, never reused. Environment as
#            tools/cts/common-images.sh and imx8mp-env.sh: ZEPHYR_WS,
#            GLMARK2_SRC, WEST_VENV (or ZEPHYR_VENV), ZEPHYR_SDK_INSTALL_DIR,
#            RUSTUP_TOOLCHAIN.
# run        for each scene, with BOARD_BOOT=uboot-mmc (the board's boot route,
#            tools/cts/board-stage.sh imx8mp) and the RUN_BOUND/RETURN_BOUND
#            below, `tools/board.sh imx8mp run <img> <records-dir> <label>` (a
#            record <records-dir>/<YYYYMMDD-HHMM>-<label>-<sha8>/, the
#            records/imx8mp/ of .gitignore). BOARD_UART (the host:port of the
#            serial bridge of target:imx8mp-console) comes from the
#            environment, never from a file; board.sh then captures
#            console.raw, console.ts and console.txt ("HH:MM:SS line") into the
#            record. Then glmark2-fb.py --scene <the glmark2 scene name> on
#            <record>/console-live.txt: console.txt (kept as it is) up to a
#            "board: record" line if there is one (the Pi 4's replay; this
#            board's image has none), the whole file otherwise. PNG:
#            <record>/<label>.png. Exit 1 when a dump is damaged (rows
#            listed): rerun that scene, never patch pixels; a PNG of a damaged
#            dump is renamed <label>.damaged.png, so no bad PNG keeps the
#            record's name. Hold the target:imx8mp and target:imx8mp-console
#            leases.
# take       the steps of run after board.sh, for the record of a run of this
#            image that run did not take (one scene, or one set): commit= and
#            the decode. Only a record with back=1 or a console return (below);
#            a record that has its commit= line is refused. No board.
# --dry-run  print what build or run would execute and run nothing.
#
# IMX8MP_DUMP_BOARD=verdin|x2 (default verdin, everything above). x2 is the
# Armadillo-X2 (same i.MX8MP and GC7000UL), where the i.MX8MP picture sessions
# run while the Verdin is away (owner, 2026-11-27): build for
# armadillo_x2/mimx8ml8/a53 (its sample conf already has the UART2 console,
# GC_COMMON_PANEL_CONSOLE=n and the watchdog arm, so IMX8MP_DUMP_CONSOLE adds
# nothing: imx8mp-console.conf is the Verdin's UART3 console and is left out;
# the watchdog flags stay) and run `BOARD_BOOT=kexec-dev tools/board.sh x2 run
# <img> <records-dir> <label>` (crash-route kexec out of Armadillo Base OS, a
# development run; BOARD_UART and BOARD_HOST from the environment, never a
# file). RUN_BOUND/RETURN_BOUND are the same 720/600: the image ends itself
# after ~410 s and WDOG1 resets at 660 s at the latest, inside board.sh's
# kexec-dev defaults (1500/600). board.sh itself writes board=x2 and
# board_measured=Armadillo-X2 ... into every x2 record's meta.txt (so every
# record says it was measured on the X2; this script adds only commit=).
# X2_STOP_CONTAINERS (board.sh: 1 stops the running podman containers for the
# run, each UNDO line printed first; 0 refuses a run while one runs) is passed
# through; default 1 for x2 here, since a jump over a running container is
# refused. The record is <records-dir>/<YYYYMMDD-HHMM>-<name>-<sha8>/, name =
# the label, or IMX8MP_DUMP_NAME for a one-scene run (the mipmap dump:
# mipmap-dump, records-dir records/imx8mp). The PNG stays <label>.png.
#
# IMX8MP_DUMP_TREE=<checkout>: the checkout the images are built from and whose
# tools/board.sh runs them (default: the one this script is in). The images of
# a picture session are built only from an origin/main commit; with a clean
# checkout of that commit here (a `git worktree`), this script can carry a fix
# that is not on main yet and the image still is that commit's, not a -dirty
# one. <label>.commit is that checkout's HEAD.
#
# IMX8MP_DUMP_SET=1 (x2 only): the selected scenes (at most 4) in ONE image and
# ONE jump. Why: connected installs a task machine's ssh key in the board's
# tmpfs overlay, so the SoC reset that ends a run removes it and a leased task
# machine gets one jump a turn (docs/notes/x2-glmark2-slow-scenes.md, "Stop,
# 2026-11-27"). build writes one image <img-dir>/<set>/ (set =
# gmset-NN-NN-..., the scene numbers) with every scene's "-b <scene>" in
# CONFIG_GLMARK2_ARGS, in the order given, and <set>.scenes (the labels); the
# image prints each scene's dump in turn. run jumps once (record
# <stamp>-<set>-<sha8>/) and then writes one record per scene,
# <stamp>-<label>-<sha8>/ (the one-record-per-scene layout the session and
# check tools read): meta.txt copied, with commit=, set_record= and
# set_scenes= added, the capture files linked, and that scene's dump decoded
# to <label>.png. Run with the same SCENE list as build. The window of a set:
# 60 + n * 290 + 60 s as below (boot, n scenes with their dumps, the hold),
# WDOG_S = that + 100, at most 1380 (WDOG1 then resets at 1440 s at the
# latest, inside board.sh's 1500 s wait for the return): 4 scenes.
#
# IMX8MP_DUMP_X2_CONSOLE_RETURN=1 (x2 only, off by default): take the record of
# a run whose only failure is that ssh did not return (board.sh: jump=ok,
# rc_cycle=5, no back=1; the lost key above) when the captured console shows
# the image's own end ("board: resetting the SoC") and, after it, Linux's
# "login:" prompt: the image ran to its end and the board is back in Linux,
# which is all a picture needs. meta.txt gets return=console ...; without
# those two console lines the run stays failed. Nothing is retried.
#
# The watchdog numbers. The image feeds WDOG1 only until
# CONFIG_GC_BOARD_WDOG_SECONDS of uptime (gc_wdog_arm.c); WDOG1 then resets the
# SoC 60 s later. A run is: boot and GPU init (~60 s allowed), the scene (10 s
# of glmark2 time plus the first frames and the readback of the slow scenes,
# terrain, shadow and refract at 1 to 4 FPS: ~60 s allowed), the dump (800x600
# RGB888 is 1.44 MB, 1.92 MB as base64 rows with their numbers and crcs:
# about 170 s at 115200 baud; 230 s allowed) and the sample's hold
# (GC_SAMPLE_HOLD_SECONDS, 60) before it resets the SoC itself: about 410 s.
# WDOG_S is 600, a margin of ~190 s that is still a bound (WDOG1 resets at 660 s
# at the latest). RUN_BOUND (board.sh: seconds to the image's end, 600 for
# U-Boot) is WDOG_S + 120 = 720, so a hung image is waited for until its own
# watchdog has fired; RETURN_BOUND stays 600, the U-Boot value. WDOG_S,
# RUN_BOUND and RETURN_BOUND can be set in the environment.
# --self-test is hermetic: it re-runs itself once without any variable this
# script reads, so its expected lines hold whatever the caller has exported
# (TMPDIR is kept). Run and build behaviour are not touched.
if [ "${1:-}" = --self-test ] && [ -z "${IMX8MP_DUMP_SELFTEST_CLEAN:-}" ]; then
    exec env -u GLMARK2_SRC -u WEST_VENV -u ZEPHYR_VENV -u IMX8MP_DUMP_BOARD \
        -u IMX8MP_DUMP_CONSOLE -u ZEPHYR_WS -u ZEPHYR_WORKSPACE -u MODS -u DRY \
        -u WDOG_S -u RUN_BOUND -u RETURN_BOUND -u PICTURES -u BOARD_UART \
        -u IMX8MP_DUMP_TREE -u IMX8MP_DUMP_SET -u IMX8MP_DUMP_X2_CONSOLE_RETURN -u IMX8MP_DUMP_NAME \
        IMX8MP_DUMP_SELFTEST_CLEAN=1 bash "$0" --self-test
fi
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
PICTURES=${PICTURES:-$HERE/glmark2-pictures.txt}
case "${IMX8MP_DUMP_BOARD:-verdin}" in
verdin) ZBOARD=verdin_imx8mp/mimx8ml8/a53; BTARGET=imx8mp; BBOOT=uboot-mmc ;;
x2) ZBOARD=armadillo_x2/mimx8ml8/a53; BTARGET=x2; BBOOT=kexec-dev ;;
*) echo "imx8mp-glmark2-dumps: IMX8MP_DUMP_BOARD must be verdin or x2" >&2; exit 2 ;;
esac
RNAME=${IMX8MP_DUMP_NAME:-}
[ -z "$RNAME" ] || [[ $RNAME =~ ^[A-Za-z0-9._-]+$ ]] || { echo "imx8mp-glmark2-dumps: IMX8MP_DUMP_NAME: letters, digits, . _ - only" >&2; exit 2; }
X2ENV=
[ "$BTARGET" != x2 ] || X2ENV="X2_STOP_CONTAINERS=${X2_STOP_CONTAINERS:-1}"
die() { echo "imx8mp-glmark2-dumps: $*" >&2; exit 2; }
TREE=${IMX8MP_DUMP_TREE:-$REPO}
[ -d "$TREE/driver/samples/zephyr/glmark2" ] || die "IMX8MP_DUMP_TREE: no driver/samples/zephyr/glmark2 in it"
SET=${IMX8MP_DUMP_SET:-0}
case $SET in 0 | 1) ;; *) die "IMX8MP_DUMP_SET must be 0 or 1" ;; esac
[ "$SET" = 0 ] || [ "$BTARGET" = x2 ] || die "IMX8MP_DUMP_SET=1 needs IMX8MP_DUMP_BOARD=x2"
[ "$SET" = 0 ] || [ -z "$RNAME" ] || die "IMX8MP_DUMP_NAME names a one-scene record: not with IMX8MP_DUMP_SET=1"
CONSRET=${IMX8MP_DUMP_X2_CONSOLE_RETURN:-0}
case $CONSRET in 0 | 1) ;; *) die "IMX8MP_DUMP_X2_CONSOLE_RETURN must be 0 or 1" ;; esac
[ "$CONSRET" = 0 ] || [ "$BTARGET" = x2 ] || die "IMX8MP_DUMP_X2_CONSOLE_RETURN=1 needs IMX8MP_DUMP_BOARD=x2"
SET_MAX=4
WDOG_GIVEN=${WDOG_S:-}
WDOG_S=${WDOG_S:-600}
RUN_BOUND_GIVEN=${RUN_BOUND:-}
RUN_BOUND=${RUN_BOUND:-$((WDOG_S + 120))}
RETURN_BOUND=${RETURN_BOUND:-600}

# scene_rows: "<NN>-<label>TAB<info string>TAB-b <argument>" per imx8mp scene
scene_rows() {
    awk -F' [|] ' '$1 == "imx8mp" {
        info = $2; n++
        b = info; sub(/^\[/, "", b); sub(/\].*/, "", b)
        o = info; sub(/^\[[^]]*\] */, "", o)
        arg = (o == "<default>" || o == "") ? b : b ":" o
        printf "%02d-gm%02d-%s\t%s\t-b %s\n", n, n, b, info, arg
    }' "$PICTURES"
}

cmd_scenes() {
    local l info arg
    scene_rows | while IFS=$'\t' read -r l info arg; do echo "${l#*-} $info $arg"; done
}

# selected SCENE...: "<label>TAB<-b argument>" per selected scene
selected() {
    local rows s hit l info arg num
    rows=$(scene_rows)
    [ -n "$rows" ] || die "no imx8mp scenes in $PICTURES"
    if [ $# -eq 0 ]; then
        printf '%s\n' "$rows" | while IFS=$'\t' read -r l info arg; do printf '%s\t%s\n' "${l#*-}" "$arg"; done
        return 0
    fi
    for s in "$@"; do
        num=
        [[ $s =~ ^[0-9]+$ ]] && num=$(printf %02d "$((10#$s))")
        hit=$(printf '%s\n' "$rows" | while IFS=$'\t' read -r l info arg; do
            if [ "${l%%-*}" = "$num" ] || [ "${l#*-}" = "$s" ] || [ "${arg#-b }" = "$s" ]; then printf '%s\t%s\n' "${l#*-}" "$arg"; fi
        done)
        [ -n "$hit" ] || die "no scene '$s' (scenes lists them)"
        [ "$(printf '%s\n' "$hit" | wc -l)" = 1 ] || die "'$s' names several scenes (scenes lists them)"
        printf '%s\n' "$hit"
    done
}

# set_row SELECTED: the one "<set>TAB<arguments>" row of IMX8MP_DUMP_SET=1 for
# the selected scenes: gmset-NN-NN-..., and every scene's "-b <argument>" in
# the order given. Also sets WDOG_S (and RUN_BOUND) for the set when the
# environment gave none.
set_row() {
    local l arg name=gmset args= n=0 seen=" "
    while IFS=$'\t' read -r l arg; do
        case $seen in *" $l "*) die "IMX8MP_DUMP_SET: $l selected twice" ;; esac
        seen="$seen$l "
        name=$name-${l:2:2}
        args="$args${args:+ }$arg"
        n=$((n + 1))
    done <<< "$1"
    [ "$n" -le "$SET_MAX" ] || die "IMX8MP_DUMP_SET: $n scenes, at most $SET_MAX fit one jump (the window is 60 + n * 290 + 60 s and board.sh waits 1500 s for the return)"
    printf '%s\t%s\n' "$name" "$args"
}
# set_window N: WDOG_S of an N-scene set (the header's numbers)
set_window() {
    local w=$((60 + $1 * 290 + 60 + 100))
    [ "$w" -le 1380 ] || w=1380
    echo "$w"
}
# set_apply SELECTED: MEMBERS (the selected rows) and the set's row into SEL;
# WDOG_S and RUN_BOUND for the set unless the environment gave them
set_apply() {
    MEMBERS=$1
    SEL=$(set_row "$1") || exit 2
    [ -n "$WDOG_GIVEN" ] || WDOG_S=$(set_window "$(wc -l <<< "$1")")
    [ -n "$RUN_BOUND_GIVEN" ] || RUN_BOUND=$((WDOG_S + 120))
}
MEMBERS=
SEL=

# build_args LABEL ARG OUT: the west command's words, one per line
build_args() {
    local a=${2//;/\\;} console=()
    case "${IMX8MP_DUMP_CONSOLE:-uart}" in
    uart | panel) ;;
    *) die "IMX8MP_DUMP_CONSOLE must be uart or panel" ;;
    esac
    [ "$BTARGET" = x2 ] || case "${IMX8MP_DUMP_CONSOLE:-uart}" in
    uart) console=("-DEXTRA_CONF_FILE=$HERE/imx8mp-console.conf" -DCONFIG_GC_COMMON_PANEL_CONSOLE=n) ;;
    panel) ;;
    esac
    printf '%s\n' build -p always -b "$ZBOARD" -d "$3/$1" "$TREE/driver/samples/zephyr/glmark2" -- \
        "-DZEPHYR_MODULES=${MODS:-<zephyr-modules>}" "-DYAKOGL_DIR=$TREE" \
        "-DGLMARK2_SRC=${GLMARK2_SRC:-<GLMARK2_SRC>}" \
        -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y "-DCONFIG_GC_BOARD_WDOG_SECONDS=$WDOG_S" "${console[@]}" \
        -DCONFIG_GLMARK2_FRAME_DUMP=y "-DCONFIG_GLMARK2_ARGS=\"$a\""
}

# The west venv: WEST_VENV, else ZEPHYR_VENV (imx8mp-env.sh's name).
# Prints the path on stdout and the variable used on stderr.
venv_pick() {
    if [ -n "${WEST_VENV:-}" ]; then echo "using WEST_VENV" >&2; echo "$WEST_VENV"
    elif [ -n "${ZEPHYR_VENV:-}" ]; then echo "using ZEPHYR_VENV" >&2; echo "$ZEPHYR_VENV"
    else echo "no venv variable set" >&2; fi
}

cmd_build() {
    [ $# -ge 1 ] || die "usage: $0 [--dry-run] build <img-dir> [SCENE...]"
    local out=$1 sel rc=0 l arg commit venv
    shift
    sel=$(selected "$@") || exit 2
    if [ "$SET" = 1 ]; then set_apply "$sel"; sel=$SEL; fi
    if [ "$DRY" = 1 ]; then
        echo "venv: $(venv_pick 2>&1 >/dev/null)"
        while IFS=$'\t' read -r l arg; do
            [ -e "$out/$l" ] && echo "$l: $out/$l exists, a real run refuses it"
            echo "echo <git -C $TREE rev-parse HEAD> > $out/$l.commit"
            [ "$SET" != 1 ] || echo "echo <$(cut -f1 <<< "$MEMBERS" | paste -sd' ' -)> > $out/$l.scenes"
            echo "west $(build_args "$l" "$arg" "$out" | paste -sd' ' -)"
        done <<< "$sel"
        return 0
    fi
    ZEPHYR_WS=${ZEPHYR_WS:-${ZEPHYR_WORKSPACE:-}}
    : "${ZEPHYR_WS:?set ZEPHYR_WS to the west workspace (it holds zephyr/)}"
    [ -d "$ZEPHYR_WS/zephyr" ] || die "$ZEPHYR_WS/zephyr does not exist"
    venv=$(venv_pick)
    [ -n "$venv" ] && PATH=$venv/bin:$PATH
    command -v west > /dev/null || die "west not on PATH (set WEST_VENV or ZEPHYR_VENV)"
    [ -f "${GLMARK2_SRC:-}/src/default-benchmarks.h" ] || die "set GLMARK2_SRC to the glmark2 checkout"
    export ZEPHYR_BASE=$ZEPHYR_WS/zephyr
    MODS=$("$HERE/zephyr-modules.sh" "$ZEPHYR_WS") || die "zephyr-modules.sh failed"
    mkdir -p "$out" || die "cannot create $out"
    out=$(cd "$out" && pwd)
    commit=$(git -C "$TREE" rev-parse HEAD) || die "git rev-parse HEAD failed in $TREE"
    echo "imx8mp-glmark2-dumps: build $commit ($(git -C "$TREE" status --porcelain | wc -l) changed paths)"
    while IFS=$'\t' read -r l arg; do
        if [ -e "$out/$l" ]; then echo "$l: $out/$l exists, not reused" >&2; rc=1; continue; fi
        mapfile -t words < <(build_args "$l" "$arg" "$out")
        if (cd "$ZEPHYR_WS" && west "${words[@]}") > "$out/$l.log" 2>&1 < /dev/null &&
            [ -s "$out/$l/zephyr/zephyr.bin" ]; then
            echo "$commit" > "$out/$l.commit"
            [ "$SET" != 1 ] || { cut -f1 <<< "$MEMBERS" > "$out/$l.scenes"; echo "$WDOG_S" > "$out/$l.wdog"; }
            echo "$l rc=0 sha256=$(sha256sum "$out/$l/zephyr/zephyr.bin" | cut -d' ' -f1) args=$arg"
        else
            echo "$l rc=failed (see $out/$l.log)"; rc=1
        fi
    done <<< "$sel"
    return $rc
}

# live_part CONSOLE: CONSOLE up to (not including) the first "board: record"
# line (the Pi 4 board layer's replay; the i.MX8MP image prints none); the
# whole file when it has none
live_part() {
    awk '/board: record/ { exit } { print }' "$1"
}

# scene_name ARG: the glmark2 scene of a "-b <scene>[:<options>]" argument, as
# the dump's "glmark2-fb: <scene> <w>x<h> begin" line names it
scene_name() {
    local s=${1#-b }
    echo "${s%%:*}"
}

# decode_live RECORD LABEL ARG: RECORD/console-live.txt, then RECORD/LABEL.png
# for that scene; a PNG of a refused dump is renamed LABEL.damaged.png
decode_live() {
    local r=$1 l=$2 rc
    live_part "$r/console.txt" > "$r/console-live.txt"
    python3 "$HERE/glmark2-fb.py" "$r/console-live.txt" "$r/$l.png" --scene "$(scene_name "$3")"
    rc=$?
    if [ $rc -ne 0 ] && [ -e "$r/$l.png" ]; then mv -f "$r/$l.png" "$r/$l.damaged.png"; fi
    return $rc
}

# stored_commit IMG-DIR LABEL: the full commit IMG-DIR/LABEL.commit holds (written
# by build); an error and status 1 when it is missing or not 40 hex digits
stored_commit() {
    local c
    c=$(head -n1 "$1/$2.commit" 2> /dev/null)
    if ! [[ $c =~ ^[0-9a-f]{40}$ ]]; then
        echo "$2: no stored commit ($1/$2.commit), rebuild the image with build; nothing run" >&2
        return 1
    fi
    echo "$c"
}

cmd_run() {
    [ $# -ge 2 ] || die "usage: $0 [--dry-run] run <img-dir> <records-dir> [SCENE...]"
    local img=$1 rec=$2 sel rc=0 l arg r c rn brc before
    shift 2
    sel=$(selected "$@") || exit 2
    if [ "$SET" = 1 ]; then set_apply "$sel"; sel=$SEL; fi
    [ -z "$RNAME" ] || [ "$(wc -l <<< "$sel")" = 1 ] || die "IMX8MP_DUMP_NAME names one record: select one scene"
    [ "$DRY" = 1 ] || : "${BOARD_UART:?set BOARD_UART to the serial bridge of the console (host:port)}"
    # every image needs its stored 40-hex build commit before anything runs
    while IFS=$'\t' read -r l arg; do
        stored_commit "$img" "$l" > /dev/null || exit 2
        # a set's image holds exactly the scenes selected now, in this order
        if [ "$SET" = 1 ] && [ "$(cat "$img/$l.scenes" 2> /dev/null)" != "$(cut -f1 <<< "$MEMBERS")" ]; then
            echo "$l: $img/$l.scenes does not list the selected scenes (build the set with the same SCENE list); nothing run" >&2
            exit 2
        fi
    done <<< "$sel"
    while IFS=$'\t' read -r l arg; do
        c=$(stored_commit "$img" "$l")
        rn=${RNAME:-$l}
        if [ "$DRY" = 1 ]; then
            echo "${X2ENV:+$X2ENV }BOARD_BOOT=$BBOOT RUN_BOUND=$RUN_BOUND RETURN_BOUND=$RETURN_BOUND tools/board.sh $BTARGET run $img/$l/zephyr/zephyr.bin $rec $rn   # BOARD_UART${X2ENV:+ and BOARD_HOST} from the environment"
            [ "$CONSRET" != 1 ] || echo "# IMX8MP_DUMP_X2_CONSOLE_RETURN=1: ssh not back (jump=ok, rc_cycle=5) is taken when the console shows 'board: resetting the SoC' and then 'login:'"
            echo "echo commit=$c >> $rec/<record of $l>/meta.txt"
            [ "$BTARGET" = x2 ] && echo "# board.sh writes board=x2 and board_measured=Armadillo-X2 ... into meta.txt itself"
            if [ "$SET" = 1 ]; then
                while IFS=$'\t' read -r ml marg; do
                    echo "# one record per scene: $rec/<stamp>-$ml-<sha8>/ (meta.txt copied, commit=, set_record=, set_scenes=; the capture linked)"
                    echo "tools/cts/glmark2-fb.py $rec/<stamp>-$ml-<sha8>/console-live.txt $rec/<stamp>-$ml-<sha8>/$ml.png --scene $(scene_name "$marg")"
                done <<< "$MEMBERS"
                continue
            fi
            echo "awk '/board: record/ { exit } { print }' $rec/<record of $l>/console.txt > $rec/<record of $l>/console-live.txt"
            echo "tools/cts/glmark2-fb.py $rec/<record of $l>/console-live.txt $rec/<record of $l>/$l.png --scene $(scene_name "$arg")"
            continue
        fi
        [ -s "$img/$l/zephyr/zephyr.bin" ] || { echo "$l: no image $img/$l/zephyr/zephyr.bin" >&2; rc=1; continue; }
        before=$(ls -d "$rec"/*-"$rn"-* 2> /dev/null)
        brc=0
        env ${X2ENV:+"$X2ENV"} BOARD_BOOT=$BBOOT RUN_BOUND=$RUN_BOUND RETURN_BOUND=$RETURN_BOUND \
            "$TREE/tools/board.sh" "$BTARGET" run "$img/$l/zephyr/zephyr.bin" "$rec" "$rn" || brc=$?
        r=$(ls -d "$rec"/*-"$rn"-* 2> /dev/null | tail -1)
        if [ "$brc" != 0 ]; then
            # only this run's own record, and only the one failure named above
            if [ -n "$r" ] && ! grep -Fxq -- "$r" <<< "$before" && console_return "$r"; then
                echo "$l: board.sh exited $brc (ssh did not return); the console shows the image's reset and Linux's login prompt after it: the record is taken (IMX8MP_DUMP_X2_CONSOLE_RETURN=1)" >&2
            else
                echo "$l: board run failed" >&2; rc=1; continue
            fi
        fi
        after_run "$r" "$l" "$arg" "$c" "$img" || rc=1
    done <<< "$sel"
    return $rc
}

# after_run RECORD LABEL ARG COMMIT IMG-DIR: what run does with the record
# board.sh left: return= (a console return), commit=, the x2 lines checked,
# then the dump decoded, or a set's records written. Status 1 on a problem.
after_run() {
    local r=$1 l=$2 arg=$3 c=$4 img=$5
    [ -s "$r/meta.txt" ] || { echo "$l: no meta.txt in the record" >&2; return 1; }
    if ! grep -q '^back=1$' "$r/meta.txt" && console_return "$r"; then
        echo "return=console (ssh did not return to this machine; the console shows the image's reset, then Linux's login prompt)" >> "$r/meta.txt"
    fi
    echo "commit=$c" >> "$r/meta.txt"
    [ "$SET" != 1 ] || echo "wdog_s=$(head -n1 "$img/$l.wdog" 2> /dev/null)" >> "$r/meta.txt"
    if [ "$BTARGET" = x2 ]; then
        grep -q '^board=x2$' "$r/meta.txt" && grep -q '^board_measured=Armadillo-X2' "$r/meta.txt" ||
            { echo "$l: meta.txt lacks board=x2 / board_measured=Armadillo-X2" >&2; return 1; }
    fi
    [ -s "$r/console.txt" ] || { echo "$l: no console.txt in the record" >&2; return 1; }
    if [ "$SET" = 1 ]; then
        set_records "$r" "$l" "$MEMBERS"
        return $?
    fi
    decode_live "$r" "$l" "$arg" || { echo "$l: damaged or missing dump, rerun it ($r)" >&2; return 1; }
}

# cmd_take: the steps of run after board.sh, on the record of a run of this
# image that run itself did not take (the script was stopped, or run without
# IMX8MP_DUMP_X2_CONSOLE_RETURN=1). Nothing is run on the board.
cmd_take() {
    [ $# -ge 2 ] || die "usage: $0 take <img-dir> <record> [SCENE...]"
    local img=$1 r=${2%/} sel l arg c rn sha
    shift 2
    sel=$(selected "$@") || exit 2
    if [ "$SET" = 1 ]; then set_apply "$sel"; sel=$SEL; fi
    [ "$(wc -l <<< "$sel")" = 1 ] || die "take: one record, so one scene (or one set with IMX8MP_DUMP_SET=1)"
    IFS=$'\t' read -r l arg <<< "$sel"
    c=$(stored_commit "$img" "$l") || exit 2
    if [ "$SET" = 1 ] && [ "$(cat "$img/$l.scenes" 2> /dev/null)" != "$(cut -f1 <<< "$MEMBERS")" ]; then
        die "take: $img/$l.scenes does not list the selected scenes"
    fi
    rn=${RNAME:-$l}
    [ -s "$img/$l/zephyr/zephyr.bin" ] || die "take: no image $img/$l/zephyr/zephyr.bin"
    sha=$(sha256sum "$img/$l/zephyr/zephyr.bin" | cut -c1-8)
    [[ $(basename "$r") =~ ^[0-9]{8}-[0-9]{4}-"$rn"-"$sha"$ ]] || die "take: the record is not <YYYYMMDD-HHMM>-$rn-$sha, a run of this image"
    [ -s "$r/meta.txt" ] || die "take: no meta.txt in the record"
    ! grep -q '^commit=' "$r/meta.txt" || die "take: the record has its commit= line: it was taken already"
    grep -q '^back=1$' "$r/meta.txt" || console_return "$r" ||
        die "take: neither back=1 nor a console return (IMX8MP_DUMP_X2_CONSOLE_RETURN=1: jump=ok, rc_cycle=5, the image's reset and then Linux's login prompt on the console): the run failed, rerun it"
    after_run "$r" "$l" "$arg" "$c" "$img"
}

# console_return RECORD: IMX8MP_DUMP_X2_CONSOLE_RETURN=1 and RECORD is an x2
# run that jumped and whose ssh did not return (jump=ok, rc_cycle=5, no
# back=1), with the image's own end line in the captured console and Linux's
# login prompt after it. The prompt ends no line, so it is in console.raw
# (the bridge's bytes) and not in console.txt (completed lines only):
# console.raw is read when the record has one, console.txt otherwise.
console_return() {
    local m=$1/meta.txt f=$1/console.raw
    [ "$CONSRET" = 1 ] && [ "$BTARGET" = x2 ] || return 1
    grep -q '^jump=ok$' "$m" 2> /dev/null && grep -q '^rc_cycle=5$' "$m" && ! grep -q '^back=1$' "$m" || return 1
    [ -s "$f" ] || f=$1/console.txt
    [ -s "$f" ] || return 1
    LC_ALL=C tr -d '\000' < "$f" |
        LC_ALL=C awk '/board: resetting the SoC/ { end = 1; next } end && /login:/ { ok = 1; exit } END { exit !ok }'
}

# set_records RECORD SET MEMBERS: one record per scene of a set's run beside
# RECORD (<stamp>-<set>-<sha8>): <stamp>-<label>-<sha8>/ with meta.txt copied
# (set_record=, set_scenes= added), every other file of RECORD linked (copied
# where a link cannot be made), and the scene's dump decoded to <label>.png.
# Status 1 when a scene's record exists already or its dump is damaged or
# missing (rerun the set, never patch).
set_records() {
    local r=${1%/} set=$2 base stamp sha d f l arg rc=0 labels
    base=$(basename "$r")
    stamp=${base%%-"$set"-*}
    sha=${base##*-}
    labels=$(cut -f1 <<< "$3" | paste -sd' ' -)
    while IFS=$'\t' read -r l arg; do
        d=$(dirname "$r")/$stamp-$l-$sha
        if [ -e "$d" ]; then echo "$l: $d exists, not reused" >&2; rc=1; continue; fi
        mkdir "$d" || { rc=1; continue; }
        for f in "$r"/*; do
            [ -f "$f" ] || continue
            case $(basename "$f") in
            meta.txt) cp "$f" "$d/" ;;
            *.png | console-live.txt) ;;
            *) ln "$f" "$d/" 2> /dev/null || cp "$f" "$d/" ;;
            esac
        done
        { echo "set_record=$base"; echo "set_scenes=$labels"; } >> "$d/meta.txt"
        decode_live "$d" "$l" "$arg" || { echo "$l: damaged or missing dump, rerun the set ($d)" >&2; rc=1; }
    done <<< "$3"
    return $rc
}

# dump_test: synthetic console.txt files ("HH:MM:SS line", as board.sh writes
# them): a clean dump, a damaged one, one followed by a replay (live part only
# is read), and another scene's dump
dump_test() {
    local d rc=0
    d=$(mktemp -d "${TMPDIR:-/tmp}/imx8mp-dumps.XXXXXX") || return 1
    python3 - "$HERE/glmark2-fb.py" "$d" <<'PY' || return 1
import base64, binascii, importlib.util, os, sys
sp = importlib.util.spec_from_file_location("fb", sys.argv[1])
fb = importlib.util.module_from_spec(sp)
sp.loader.exec_module(fb)
d, w, h = sys.argv[2], 8, 6
rows = [bytes((x * 5 + y * 9 + c) & 255 for x in range(w) for c in range(3)) for y in range(h)]
probes, c, cq, mean = fb.picture_value(w, h, rows)
pic = ("%dx%d; probes c %s00, tl %s00, tr %s00, bl %s00, br %s00; crc16 %04x, crc16q %04x; "
       "mean %d %d %d; black no" % ((w, h) + tuple(p.hex() for p in probes) + (c, cq) + tuple(mean)))
head = ["gc_wdog_arm: armed", "glmark2-fb: [texture] texture-filter=mipmap %dx%d begin" % (w, h)]
R = ["glmark2-fb: %d %04x %s" % (y, fb.crc16(r), base64.b64encode(r).decode()) for y, r in enumerate(rows)]
tail = ["glmark2-fb: end crc32 %08x" % (binascii.crc32(b"".join(rows)) & 0xFFFFFFFF),
        "glmark2-picture: [texture] texture-filter=mipmap: " + pic]
replay = ["board: record, 99 s after boot", R[-2], R[-1][:30]]
bad = list(R)
bad[2] = bad[2][:30] + "A" + bad[2][31:]

def write(name, ls):
    os.mkdir(os.path.join(d, name))
    with open(os.path.join(d, name, "console.txt"), "w") as f:
        f.write("".join("12:00:%02d %s\n" % (i % 60, l) for i, l in enumerate(ls)))

write("ok", head + R + tail)
write("bad", head + bad + tail)
write("replay", head + R + tail + replay)
# a set's run: two scenes' dumps in one console, the second one's rows other
# pixels; and the same with a damaged row in the second dump
rows2 = [bytes((x * 3 + y * 7 + c * 11) & 255 for x in range(w) for c in range(3)) for y in range(h)]
probes, c, cq, mean = fb.picture_value(w, h, rows2)
pic2 = ("%dx%d; probes c %s00, tl %s00, tr %s00, bl %s00, br %s00; crc16 %04x, crc16q %04x; "
        "mean %d %d %d; black no" % ((w, h) + tuple(p.hex() for p in probes) + (c, cq) + tuple(mean)))
R2 = ["glmark2-fb: %d %04x %s" % (y, fb.crc16(r), base64.b64encode(r).decode()) for y, r in enumerate(rows2)]
two = (head + R + tail + ["glmark2-fb: [terrain] <default> %dx%d begin" % (w, h)] + R2
       + ["glmark2-fb: end crc32 %08x" % (binascii.crc32(b"".join(rows2)) & 0xFFFFFFFF),
          "glmark2-picture: [terrain] <default>: " + pic2])
end = ["board: resetting the SoC", "U-Boot 2020.04", "armadillo login: "]
for name, ls in (("20261128-1200-gmset-05-23-0123abcd", two + end),
                 ("20261128-1300-gmset-05-23-0123abcd", two[:-4] + [two[-4][:30] + "A" + two[-4][31:]] + two[-3:] + end)):
    write(name, ls)
    with open(os.path.join(d, name, "meta.txt"), "w") as f:
        f.write("board=x2\njump=ok\nrc_cycle=5\n")
    with open(os.path.join(d, name, "console.ts"), "w") as f:
        f.write("1 x\n")
# the bridge's raw bytes: the login prompt ends no line, so only console.raw has it
name = "20261128-1400-gmset-05-23-0123abcd"
write(name, two + end[:2])
with open(os.path.join(d, name, "meta.txt"), "w") as f:
    f.write("board=x2\nboard_measured=Armadillo-X2 (i.MX 8M Plus, GC7000UL), stands in for the Verdin iMX8MP\njump=ok\nrc_cycle=5\n")
with open(os.path.join(d, name, "console.raw"), "wb") as f:
    f.write(("\r\n".join(two + end)).encode().replace(b"U-Boot", b"\0\0U-Boot"))
PY
    cp "$d/ok/console.txt" "$d/ok.orig"
    decode_live "$d/ok" gm-t "-b texture:texture-filter=mipmap" > /dev/null 2>&1 || { echo "self-test: clean dump refused" >&2; rc=1; }
    [ -s "$d/ok/gm-t.png" ] || { echo "self-test: no PNG for the clean dump" >&2; rc=1; }
    cmp -s "$d/ok.orig" "$d/ok/console.txt" || { echo "self-test: console.txt changed" >&2; rc=1; }
    decode_live "$d/ok" gm-s "-b shading" > /dev/null 2>&1 && { echo "self-test: another scene's dump accepted" >&2; rc=1; }
    [ ! -e "$d/ok/gm-s.png" ] || { echo "self-test: a PNG for another scene's dump" >&2; rc=1; }
    decode_live "$d/bad" gm-t "-b texture:texture-filter=mipmap" > /dev/null 2>&1
    [ $? -eq 1 ] || { echo "self-test: damaged dump not exit 1" >&2; rc=1; }
    [ ! -e "$d/bad/gm-t.png" ] || { echo "self-test: damaged PNG kept under the record's name" >&2; rc=1; }
    [ -s "$d/bad/gm-t.damaged.png" ] || { echo "self-test: no .damaged.png" >&2; rc=1; }
    decode_live "$d/replay" gm-t "-b texture:texture-filter=mipmap" > /dev/null 2>&1 || { echo "self-test: live dump before a replay refused" >&2; rc=1; }
    python3 "$HERE/glmark2-fb.py" "$d/replay/console.txt" "$d/whole.png" > /dev/null 2>&1 && { echo "self-test: the replay does not damage the whole log" >&2; rc=1; }
    # a set's run: one record per scene, each scene's own dump decoded
    local s1=$d/20261128-1200-gmset-05-23-0123abcd s2=$d/20261128-1300-gmset-05-23-0123abcd
    local mem=$'gm05-texture\t-b texture:texture-filter=mipmap\ngm23-terrain\t-b terrain' a b
    set_records "$s1" gmset-05-23 "$mem" > /dev/null 2>&1 || { echo "self-test: a clean set refused" >&2; rc=1; }
    a=$d/20261128-1200-gm05-texture-0123abcd; b=$d/20261128-1200-gm23-terrain-0123abcd
    [ -s "$a/gm05-texture.png" ] && [ -s "$b/gm23-terrain.png" ] || { echo "self-test: a set's scene records lack their PNGs" >&2; rc=1; }
    cmp -s "$a/gm05-texture.png" "$b/gm23-terrain.png" && { echo "self-test: a set's two scenes decode to one picture" >&2; rc=1; }
    cmp -s "$a/gm05-texture.png" "$d/ok/gm-t.png" || { echo "self-test: a set's first scene is not its own dump" >&2; rc=1; }
    grep -qx 'set_record=20261128-1200-gmset-05-23-0123abcd' "$b/meta.txt" && grep -qx 'set_scenes=gm05-texture gm23-terrain' "$b/meta.txt" &&
        grep -qx 'board=x2' "$b/meta.txt" || { echo "self-test: a set's scene meta.txt" >&2; rc=1; }
    grep -q set_record "$s1/meta.txt" && { echo "self-test: the set's own meta.txt was changed" >&2; rc=1; }
    cmp -s "$s1/console.txt" "$b/console.txt" && [ -e "$b/console.ts" ] || { echo "self-test: a set's scene record lacks the capture" >&2; rc=1; }
    set_records "$s1" gmset-05-23 "$mem" > /dev/null 2>&1 && { echo "self-test: existing scene records reused" >&2; rc=1; }
    set_records "$s2" gmset-05-23 "$mem" > /dev/null 2>&1 && { echo "self-test: a set with a damaged dump accepted" >&2; rc=1; }
    [ -s "$d/20261128-1300-gm05-texture-0123abcd/gm05-texture.png" ] || { echo "self-test: the whole scene of a damaged set has no PNG" >&2; rc=1; }
    [ ! -e "$d/20261128-1300-gm23-terrain-0123abcd/gm23-terrain.png" ] && [ -s "$d/20261128-1300-gm23-terrain-0123abcd/gm23-terrain.damaged.png" ] ||
        { echo "self-test: the damaged scene of a set keeps a PNG under its name" >&2; rc=1; }
    # console_return: only x2, only the opt-in, only jump=ok + rc_cycle=5 without back=1, only reset then login
    (BTARGET=x2; CONSRET=1; console_return "$s1") || { echo "self-test: console_return refuses the reset-then-login record" >&2; rc=1; }
    (BTARGET=x2; CONSRET=0; console_return "$s1") && { echo "self-test: console_return without the opt-in" >&2; rc=1; }
    (BTARGET=imx8mp; CONSRET=1; console_return "$s1") && { echo "self-test: console_return on the verdin" >&2; rc=1; }
    (BTARGET=x2; CONSRET=1; console_return "$d/ok") && { echo "self-test: console_return without a meta.txt" >&2; rc=1; }
    printf 'board=x2\njump=ok\nrc_cycle=5\n' > "$d/ok/meta.txt"
    (BTARGET=x2; CONSRET=1; console_return "$d/ok") && { echo "self-test: console_return without the reset and login lines" >&2; rc=1; }
    printf '12:00:00 armadillo login: \n12:00:01 board: resetting the SoC\n' >> "$d/ok/console.txt"
    (BTARGET=x2; CONSRET=1; console_return "$d/ok") && { echo "self-test: console_return with the login before the reset" >&2; rc=1; }
    echo "back=1" >> "$s1/meta.txt"
    (BTARGET=x2; CONSRET=1; console_return "$s1") && { echo "self-test: console_return on a run whose ssh returned" >&2; rc=1; }
    printf 'board=x2\njump=ok\n' > "$s1/meta.txt"
    (BTARGET=x2; CONSRET=1; console_return "$s1") && { echo "self-test: console_return without rc_cycle=5" >&2; rc=1; }
    # the prompt ends no line: console.raw has it, console.txt does not; and take
    local s3=$d/20261128-1400-gmset-05-23-0123abcd ti=$d/img
    (BTARGET=x2; CONSRET=1; console_return "$s3") || { echo "self-test: console_return does not read console.raw" >&2; rc=1; }
    mkdir -p "$ti/gmset-05-23/zephyr"
    echo image > "$ti/gmset-05-23/zephyr/zephyr.bin"
    echo 0123456789abcdef0123456789abcdef01234567 > "$ti/gmset-05-23.commit"
    printf 'gm05-texture\ngm23-terrain\n' > "$ti/gmset-05-23.scenes"
    echo 800 > "$ti/gmset-05-23.wdog"
    local sha t3
    sha=$(sha256sum "$ti/gmset-05-23/zephyr/zephyr.bin" | cut -c1-8)
    t3=$d/20261128-1400-gmset-05-23-$sha
    (IMX8MP_DUMP_BOARD=x2 IMX8MP_DUMP_SET=1 IMX8MP_DUMP_X2_CONSOLE_RETURN=1 bash "$0" take "$ti" "$s3" 5 23 > /dev/null 2>&1); [ $? -eq 2 ] ||
        { echo "self-test: take of another image's record accepted" >&2; rc=1; }
    mv "$s3" "$t3"
    (IMX8MP_DUMP_BOARD=x2 IMX8MP_DUMP_SET=1 bash "$0" take "$ti" "$t3" 5 23 > /dev/null 2>&1); [ $? -eq 2 ] ||
        { echo "self-test: take without the console-return opt-in accepted" >&2; rc=1; }
    (IMX8MP_DUMP_BOARD=x2 IMX8MP_DUMP_SET=1 IMX8MP_DUMP_X2_CONSOLE_RETURN=1 bash "$0" take "$ti" "$t3" 5 23 > /dev/null 2>&1) ||
        { echo "self-test: take of a console-return set record refused" >&2; rc=1; }
    grep -q '^return=console ' "$t3/meta.txt" && grep -qx 'commit=0123456789abcdef0123456789abcdef01234567' "$t3/meta.txt" &&
        grep -qx 'wdog_s=800' "$t3/meta.txt" || { echo "self-test: take's meta.txt lines" >&2; rc=1; }
    [ -s "$d/20261128-1400-gm23-terrain-$sha/gm23-terrain.png" ] && grep -qx 'commit=0123456789abcdef0123456789abcdef01234567' "$d/20261128-1400-gm23-terrain-$sha/meta.txt" ||
        { echo "self-test: take wrote no scene record with its commit" >&2; rc=1; }
    (IMX8MP_DUMP_BOARD=x2 IMX8MP_DUMP_SET=1 IMX8MP_DUMP_X2_CONSOLE_RETURN=1 bash "$0" take "$ti" "$t3" 5 23 > /dev/null 2>&1); [ $? -eq 2 ] ||
        { echo "self-test: a record taken twice" >&2; rc=1; }
    rm -rf "$d"
    return $rc
}

self_test() {
    local n out all
    n=$(scene_rows | wc -l)
    all=$(cmd_scenes)
    [ "$n" = 33 ] || { echo "self-test: $n imx8mp scenes, expected 33" >&2; return 1; }
    [ "$(wc -l <<< "$all")" = 33 ] || { echo "self-test: scenes does not print 33 lines" >&2; return 1; }
    grep -q ' -b terrain$' <<< "$all" || { echo "self-test: terrain's argument missing" >&2; return 1; }
    grep -q ' -b texture:texture-filter=mipmap$' <<< "$all" || { echo "self-test: mipmap's argument missing" >&2; return 1; }
    (selected build > /dev/null 2>&1) && { echo "self-test: a name that matches several scenes accepted" >&2; return 1; }
    [ "$(selected texture:texture-filter=mipmap 2> /dev/null)" = "$(selected gm05-texture 2> /dev/null)" ] || { echo "self-test: -b argument selection" >&2; return 1; }
    [ "$(selected 5 2> /dev/null)" = "$(selected gm05-texture 2> /dev/null)" ] || { echo "self-test: selection by number" >&2; return 1; }
    [ "$(selected texture:texture-filter=mipmap 2> /dev/null | wc -l)" = 1 ] || { echo "self-test: -b argument names several rows" >&2; return 1; }
    (selected nosuch:scene > /dev/null 2>&1) && { echo "self-test: unknown SCENE accepted" >&2; return 1; }
    [ "$(selected 5 gm33-loop 2> /dev/null | wc -l)" = 2 ] || { echo "self-test: SCENE selection" >&2; return 1; }
    out=$(DRY=1 cmd_build /nonexistent-dir 5)
    case $out in
    *"verdin_imx8mp/mimx8ml8/a53"*"-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y -DCONFIG_GC_BOARD_WDOG_SECONDS=$WDOG_S"*"-DCONFIG_GLMARK2_FRAME_DUMP=y"*'-DCONFIG_GLMARK2_ARGS="-b texture:texture-filter=mipmap"'*) ;;
    *) echo "self-test: build line: $out" >&2; return 1 ;;
    esac
    out=$(unset WEST_VENV; ZEPHYR_VENV=/z DRY=1 cmd_build /nonexistent-dir 5)
    case $out in *"venv: using ZEPHYR_VENV"*) ;; *) echo "self-test: ZEPHYR_VENV not used: $out" >&2; return 1 ;; esac
    out=$(WEST_VENV= ZEPHYR_VENV=/z DRY=1 cmd_build /nonexistent-dir 5)
    case $out in *"venv: using ZEPHYR_VENV"*) ;; *) echo "self-test: empty WEST_VENV does not fall back: $out" >&2; return 1 ;; esac
    out=$(WEST_VENV=/w ZEPHYR_VENV=/z DRY=1 cmd_build /nonexistent-dir 5)
    case $out in *"venv: using WEST_VENV"*) ;; *) echo "self-test: WEST_VENV does not win: $out" >&2; return 1 ;; esac
    [ "$(WEST_VENV=/w ZEPHYR_VENV=/z venv_pick 2> /dev/null)" = /w ] || { echo "self-test: venv_pick path (WEST_VENV)" >&2; return 1; }
    [ "$(unset WEST_VENV; ZEPHYR_VENV=/z venv_pick 2> /dev/null)" = /z ] || { echo "self-test: venv_pick path (ZEPHYR_VENV)" >&2; return 1; }
    out=$(IMX8MP_DUMP_BOARD=x2 bash "$0" --dry-run build /nonexistent-dir 5)
    case $out in
    *"-b armadillo_x2/mimx8ml8/a53"*"-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y -DCONFIG_GC_BOARD_WDOG_SECONDS=$WDOG_S"*"-DCONFIG_GLMARK2_FRAME_DUMP=y"*'-DCONFIG_GLMARK2_ARGS="-b texture:texture-filter=mipmap"'*) ;;
    *) echo "self-test: x2 build line: $out" >&2; return 1 ;;
    esac
    case $out in *verdin* | *EXTRA_CONF_FILE* | *PANEL_CONSOLE*) echo "self-test: x2 build line has verdin or console flags: $out" >&2; return 1 ;; esac
    out=$(IMX8MP_DUMP_BOARD=x2 bash "$0" --dry-run build /nonexistent-dir "$(grep -m1 'effect2d' <<< "$all" | cut -d' ' -f1)")
    case $out in *'kernel=0,1,0\;1,-4,1\;0,1,0\;'*) ;; *) echo "self-test: x2 effect2d kernel not escaped: $out" >&2; return 1 ;; esac
    local k14='kernel=1,1,1,1,1\;1,1,1,1,1\;1,1,1,1,1\;'
    out=$(IMX8MP_DUMP_BOARD=x2 bash "$0" --dry-run build /nonexistent-dir 14)
    case $out in *"$k14"*) ;; *) echo "self-test: x2 scene 14 kernel not escaped in full: $out" >&2; return 1 ;; esac
    out=$(DRY=1 cmd_build /nonexistent-dir 14)
    case $out in *"$k14"*) ;; *) echo "self-test: scene 14 kernel not escaped in full: $out" >&2; return 1 ;; esac
    echo "self-test: scene 14 effect2d kernel ok (default and x2)"
    (IMX8MP_DUMP_BOARD=nosuch bash "$0" scenes > /dev/null 2>&1); [ $? -eq 2 ] || { echo "self-test: bad IMX8MP_DUMP_BOARD accepted" >&2; return 1; }
    out=$(DRY=1 cmd_build /nonexistent-dir terrain)
    case $out in *'-DCONFIG_GLMARK2_ARGS="-b terrain"'*) ;; *) echo "self-test: terrain build line: $out" >&2; return 1 ;; esac
    case $out in *"-DEXTRA_CONF_FILE=$HERE/imx8mp-console.conf"*"-DCONFIG_GC_COMMON_PANEL_CONSOLE=n"*) ;; *) echo "self-test: uart console flags: $out" >&2; return 1 ;; esac
    out=$(IMX8MP_DUMP_CONSOLE=panel DRY=1 cmd_build /nonexistent-dir terrain)
    case $out in *EXTRA_CONF_FILE*) echo "self-test: panel mode adds the uart console: $out" >&2; return 1 ;; esac
    out=$(DRY=1 cmd_build /nonexistent-dir "$(grep -m1 'effect2d' <<< "$all" | cut -d' ' -f1)")
    case $out in *'kernel=0,1,0\;1,-4,1\;0,1,0\;'*) ;; *) echo "self-test: effect2d kernel not escaped: $out" >&2; return 1 ;; esac
    local ci=${TMPDIR:-/tmp}/imx8mp-dumps-img.$$ cm=0123456789abcdef0123456789abcdef01234567
    mkdir -p "$ci"
    echo "$cm" > "$ci/gm05-texture.commit"
    out=$(DRY=1 cmd_run "$ci" /rec 5)
    case $out in *"echo commit=$cm >> /rec/<record of gm05-texture>/meta.txt"*) ;; *) rm -rf "$ci"; echo "self-test: run line has no commit line: $out" >&2; return 1 ;; esac
    (DRY=1 cmd_run "$ci" /rec gm33-loop > /dev/null 2>&1); [ $? -eq 2 ] || { rm -rf "$ci"; echo "self-test: image without commit not refused with 2" >&2; return 1; }
    echo "not-a-commit" > "$ci/gm33-loop.commit"
    (DRY=1 cmd_run "$ci" /rec gm33-loop > /dev/null 2>&1); [ $? -eq 2 ] || { rm -rf "$ci"; echo "self-test: malformed commit not refused with 2" >&2; return 1; }
    (DRY=1 cmd_run "$ci" /rec 5 gm33-loop > /dev/null 2>&1); [ $? -eq 2 ] || { rm -rf "$ci"; echo "self-test: one missing commit does not stop the whole run" >&2; return 1; }
    out=$(DRY=1 cmd_run "$ci" /rec 5)
    out=${out//"$ci"//img}
    local x2out
    x2out=$(IMX8MP_DUMP_BOARD=x2 bash "$0" --dry-run run "$ci" /rec 5)
    x2out=${x2out//"$ci"//img}
    rm -rf "$ci"
    case $x2out in
    "X2_STOP_CONTAINERS=1 BOARD_BOOT=kexec-dev RUN_BOUND=$RUN_BOUND RETURN_BOUND=$RETURN_BOUND tools/board.sh x2 run /img/gm05-texture/zephyr/zephyr.bin /rec gm05-texture"*"echo commit=$cm >>"*"board.sh writes board=x2 and board_measured=Armadillo-X2"*) ;;
    *) echo "self-test: x2 run line: $x2out" >&2; return 1 ;;
    esac
    case $x2out in *uboot-mmc* | *"board.sh imx8mp"*) echo "self-test: x2 run line names the verdin route: $x2out" >&2; return 1 ;; esac
    if grep -qE '[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+|/dev/tty' <<< "$x2out"; then echo "self-test: an address in the x2 run line" >&2; return 1; fi
    case $out in
    "BOARD_BOOT=uboot-mmc RUN_BOUND=$RUN_BOUND RETURN_BOUND=$RETURN_BOUND tools/board.sh imx8mp run /img/gm05-texture/zephyr/zephyr.bin /rec gm05-texture"*) ;;
    *) echo "self-test: run line: $out" >&2; return 1 ;;
    esac
    if grep -qE '[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+|/dev/tty' <<< "$out"; then echo "self-test: an address in the run line" >&2; return 1; fi
    mkdir -p "${TMPDIR:-/tmp}/imx8mp-dumps-exists.$$/gm05-texture"
    out=$(DRY=1 cmd_build "${TMPDIR:-/tmp}/imx8mp-dumps-exists.$$" 5)
    rm -rf "${TMPDIR:-/tmp}/imx8mp-dumps-exists.$$"
    case $out in *"exists, a real run refuses it"*) ;; *) echo "self-test: an existing build directory not flagged" >&2; return 1 ;; esac
    # IMX8MP_DUMP_SET: one image for the scenes, x2 only, at most SET_MAX, its window
    out=$(IMX8MP_DUMP_BOARD=x2 IMX8MP_DUMP_SET=1 bash "$0" --dry-run build /nonexistent-dir 1 23 24 25)
    case $out in
    *"-d /nonexistent-dir/gmset-01-23-24-25 "*"-DCONFIG_GC_BOARD_WDOG_SECONDS=1380 "*'-DCONFIG_GLMARK2_ARGS="-b build:use-vbo=false -b terrain -b shadow -b refract"'*) ;;
    *) echo "self-test: set build line: $out" >&2; return 1 ;;
    esac
    case $out in *"<gm01-build gm23-terrain gm24-shadow gm25-refract> > /nonexistent-dir/gmset-01-23-24-25.scenes"*) ;; *) echo "self-test: set build has no .scenes line: $out" >&2; return 1 ;; esac
    [ "$(grep -c '^west ' <<< "$out")" = 1 ] || { echo "self-test: a set builds more than one image" >&2; return 1; }
    out=$(IMX8MP_DUMP_BOARD=x2 IMX8MP_DUMP_SET=1 WDOG_S=900 bash "$0" --dry-run build /nonexistent-dir 13 14)
    case $out in *"-DCONFIG_GC_BOARD_WDOG_SECONDS=900 "*'kernel=0,1,0\;1,-4,1\;0,1,0\; -b effect2d:kernel=1,1,1,1,1\;'*) ;; *) echo "self-test: set with WDOG_S and effect2d kernels: $out" >&2; return 1 ;; esac
    [ "$(set_window 1)" = 510 ] && [ "$(set_window 2)" = 800 ] && [ "$(set_window 4)" = 1380 ] || { echo "self-test: set_window" >&2; return 1; }
    (IMX8MP_DUMP_SET=1 bash "$0" --dry-run build /nonexistent-dir 1 23 > /dev/null 2>&1); [ $? -eq 2 ] || { echo "self-test: a set on the verdin accepted" >&2; return 1; }
    (IMX8MP_DUMP_BOARD=x2 IMX8MP_DUMP_SET=1 bash "$0" --dry-run build /nonexistent-dir 1 2 3 4 5 > /dev/null 2>&1); [ $? -eq 2 ] || { echo "self-test: a set of 5 scenes accepted" >&2; return 1; }
    (IMX8MP_DUMP_BOARD=x2 IMX8MP_DUMP_SET=1 bash "$0" --dry-run build /nonexistent-dir 1 gm01-build > /dev/null 2>&1); [ $? -eq 2 ] || { echo "self-test: a set with one scene twice accepted" >&2; return 1; }
    (IMX8MP_DUMP_X2_CONSOLE_RETURN=1 bash "$0" scenes > /dev/null 2>&1); [ $? -eq 2 ] || { echo "self-test: IMX8MP_DUMP_X2_CONSOLE_RETURN on the verdin accepted" >&2; return 1; }
    ci=${TMPDIR:-/tmp}/imx8mp-dumps-set.$$
    mkdir -p "$ci/tree/driver/samples/zephyr/glmark2"
    echo "$cm" > "$ci/gmset-01-23.commit"
    printf 'gm01-build\ngm23-terrain\n' > "$ci/gmset-01-23.scenes"
    out=$(IMX8MP_DUMP_BOARD=x2 IMX8MP_DUMP_SET=1 IMX8MP_DUMP_X2_CONSOLE_RETURN=1 bash "$0" --dry-run run "$ci" /rec 1 23)
    out=${out//"$ci"//img}
    case $out in
    "X2_STOP_CONTAINERS=1 BOARD_BOOT=kexec-dev RUN_BOUND=920 RETURN_BOUND=$RETURN_BOUND tools/board.sh x2 run /img/gmset-01-23/zephyr/zephyr.bin /rec gmset-01-23"*"IMX8MP_DUMP_X2_CONSOLE_RETURN=1"*"/rec/<stamp>-gm01-build-<sha8>/gm01-build.png --scene build"*"/rec/<stamp>-gm23-terrain-<sha8>/gm23-terrain.png --scene terrain"*) ;;
    *) rm -rf "$ci"; echo "self-test: set run lines: $out" >&2; return 1 ;;
    esac
    [ "$(grep -c 'board.sh x2 run' <<< "$out")" = 1 ] || { rm -rf "$ci"; echo "self-test: a set jumps more than once" >&2; return 1; }
    (IMX8MP_DUMP_BOARD=x2 IMX8MP_DUMP_SET=1 bash "$0" --dry-run run "$ci" /rec 23 1 > /dev/null 2>&1); [ $? -eq 2 ] || { rm -rf "$ci"; echo "self-test: a set run with another scene order accepted" >&2; return 1; }
    # IMX8MP_DUMP_TREE: the image's source, its commit and its board.sh are that checkout's
    out=$(IMX8MP_DUMP_BOARD=x2 IMX8MP_DUMP_TREE="$ci/tree" bash "$0" --dry-run build /nonexistent-dir 5)
    case $out in *"git -C $ci/tree rev-parse HEAD"*" $ci/tree/driver/samples/zephyr/glmark2 -- "*"-DYAKOGL_DIR=$ci/tree "*) ;; *) rm -rf "$ci"; echo "self-test: IMX8MP_DUMP_TREE build line: $out" >&2; return 1 ;; esac
    (IMX8MP_DUMP_TREE="$ci" bash "$0" scenes > /dev/null 2>&1); [ $? -eq 2 ] || { rm -rf "$ci"; echo "self-test: an IMX8MP_DUMP_TREE without the sample accepted" >&2; return 1; }
    rm -rf "$ci"
    python3 "$HERE/glmark2-fb.py" --self-test > /dev/null || return 1
    dump_test || return 1
    echo "imx8mp-glmark2-dumps self-test: ok"
}

DRY=0
while [ $# -gt 0 ]; do
    case "$1" in
    --dry-run) DRY=1; shift ;;
    --self-test) self_test; exit $? ;;
    *) break ;;
    esac
done
[ $# -ge 1 ] || die "usage: $0 scenes | [--dry-run] build|run ... | take ... | --self-test"
verb=$1
shift
case "$verb" in
scenes) cmd_scenes ;;
build) cmd_build "$@" ;;
run) cmd_run "$@" ;;
take) cmd_take "$@" ;;
*) die "unknown command '$verb'" ;;
esac

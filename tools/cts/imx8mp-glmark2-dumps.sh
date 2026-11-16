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
# --dry-run  print what build or run would execute and run nothing.
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
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
PICTURES=${PICTURES:-$HERE/glmark2-pictures.txt}
ZBOARD=verdin_imx8mp/mimx8ml8/a53
WDOG_S=${WDOG_S:-600}
RUN_BOUND=${RUN_BOUND:-$((WDOG_S + 120))}
RETURN_BOUND=${RETURN_BOUND:-600}
die() { echo "imx8mp-glmark2-dumps: $*" >&2; exit 2; }

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

# build_args LABEL ARG OUT: the west command's words, one per line
build_args() {
    local a=${2//;/\\;} console=()
    case "${IMX8MP_DUMP_CONSOLE:-uart}" in
    uart) console=("-DEXTRA_CONF_FILE=$HERE/imx8mp-console.conf" -DCONFIG_GC_COMMON_PANEL_CONSOLE=n) ;;
    panel) ;;
    *) die "IMX8MP_DUMP_CONSOLE must be uart or panel" ;;
    esac
    printf '%s\n' build -p always -b "$ZBOARD" -d "$3/$1" "$REPO/driver/samples/zephyr/glmark2" -- \
        "-DZEPHYR_MODULES=${MODS:-<zephyr-modules>}" "-DYAKOGL_DIR=$REPO" \
        "-DGLMARK2_SRC=${GLMARK2_SRC:-<GLMARK2_SRC>}" \
        -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y "-DCONFIG_GC_BOARD_WDOG_SECONDS=$WDOG_S" "${console[@]}" \
        -DCONFIG_GLMARK2_FRAME_DUMP=y "-DCONFIG_GLMARK2_ARGS=\"$a\""
}

cmd_build() {
    [ $# -ge 1 ] || die "usage: $0 [--dry-run] build <img-dir> [SCENE...]"
    local out=$1 sel rc=0 l arg
    shift
    sel=$(selected "$@") || exit 2
    if [ "$DRY" = 1 ]; then
        while IFS=$'\t' read -r l arg; do
            [ -e "$out/$l" ] && echo "$l: $out/$l exists, a real run refuses it"
            echo "west $(build_args "$l" "$arg" "$out" | paste -sd' ' -)"
        done <<< "$sel"
        return 0
    fi
    ZEPHYR_WS=${ZEPHYR_WS:-${ZEPHYR_WORKSPACE:-}}
    : "${ZEPHYR_WS:?set ZEPHYR_WS to the west workspace (it holds zephyr/)}"
    [ -d "$ZEPHYR_WS/zephyr" ] || die "$ZEPHYR_WS/zephyr does not exist"
    [ -n "${WEST_VENV:-}" ] || WEST_VENV=${ZEPHYR_VENV:-}
    [ -n "$WEST_VENV" ] && PATH=$WEST_VENV/bin:$PATH
    command -v west > /dev/null || die "west not on PATH (set WEST_VENV or ZEPHYR_VENV)"
    [ -f "${GLMARK2_SRC:-}/src/default-benchmarks.h" ] || die "set GLMARK2_SRC to the glmark2 checkout"
    export ZEPHYR_BASE=$ZEPHYR_WS/zephyr
    MODS=$("$HERE/zephyr-modules.sh" "$ZEPHYR_WS") || die "zephyr-modules.sh failed"
    mkdir -p "$out" || die "cannot create $out"
    out=$(cd "$out" && pwd)
    echo "imx8mp-glmark2-dumps: build $(git -C "$REPO" rev-parse HEAD) ($(git -C "$REPO" status --porcelain | wc -l) changed paths)"
    while IFS=$'\t' read -r l arg; do
        if [ -e "$out/$l" ]; then echo "$l: $out/$l exists, not reused" >&2; rc=1; continue; fi
        mapfile -t words < <(build_args "$l" "$arg" "$out")
        if (cd "$ZEPHYR_WS" && west "${words[@]}") > "$out/$l.log" 2>&1 < /dev/null &&
            [ -s "$out/$l/zephyr/zephyr.bin" ]; then
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

cmd_run() {
    [ $# -ge 2 ] || die "usage: $0 [--dry-run] run <img-dir> <records-dir> [SCENE...]"
    local img=$1 rec=$2 sel rc=0 l arg r
    shift 2
    sel=$(selected "$@") || exit 2
    [ "$DRY" = 1 ] || : "${BOARD_UART:?set BOARD_UART to the bridge of target:imx8mp-console (host:port)}"
    while IFS=$'\t' read -r l arg; do
        if [ "$DRY" = 1 ]; then
            echo "BOARD_BOOT=uboot-mmc RUN_BOUND=$RUN_BOUND RETURN_BOUND=$RETURN_BOUND tools/board.sh imx8mp run $img/$l/zephyr/zephyr.bin $rec $l   # BOARD_UART from the environment"
            echo "awk '/board: record/ { exit } { print }' $rec/<record of $l>/console.txt > $rec/<record of $l>/console-live.txt"
            echo "tools/cts/glmark2-fb.py $rec/<record of $l>/console-live.txt $rec/<record of $l>/$l.png --scene $(scene_name "$arg")"
            continue
        fi
        [ -s "$img/$l/zephyr/zephyr.bin" ] || { echo "$l: no image $img/$l/zephyr/zephyr.bin" >&2; rc=1; continue; }
        BOARD_BOOT=uboot-mmc RUN_BOUND=$RUN_BOUND RETURN_BOUND=$RETURN_BOUND \
            "$REPO/tools/board.sh" imx8mp run "$img/$l/zephyr/zephyr.bin" "$rec" "$l" || { echo "$l: board run failed" >&2; rc=1; continue; }
        r=$(ls -d "$rec"/*-"$l"-* 2> /dev/null | tail -1)
        [ -s "$r/console.txt" ] || { echo "$l: no console.txt in the record" >&2; rc=1; continue; }
        decode_live "$r" "$l" "$arg" || { echo "$l: damaged or missing dump, rerun it ($r)" >&2; rc=1; }
    done <<< "$sel"
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
    out=$(DRY=1 cmd_build /nonexistent-dir terrain)
    case $out in *'-DCONFIG_GLMARK2_ARGS="-b terrain"'*) ;; *) echo "self-test: terrain build line: $out" >&2; return 1 ;; esac
    case $out in *"-DEXTRA_CONF_FILE=$HERE/imx8mp-console.conf"*"-DCONFIG_GC_COMMON_PANEL_CONSOLE=n"*) ;; *) echo "self-test: uart console flags: $out" >&2; return 1 ;; esac
    out=$(IMX8MP_DUMP_CONSOLE=panel DRY=1 cmd_build /nonexistent-dir terrain)
    case $out in *EXTRA_CONF_FILE*) echo "self-test: panel mode adds the uart console: $out" >&2; return 1 ;; esac
    out=$(DRY=1 cmd_build /nonexistent-dir "$(grep -m1 'effect2d' <<< "$all" | cut -d' ' -f1)")
    case $out in *'kernel=0,1,0\;1,-4,1\;0,1,0\;'*) ;; *) echo "self-test: effect2d kernel not escaped: $out" >&2; return 1 ;; esac
    out=$(DRY=1 cmd_run /img /rec 5)
    case $out in
    "BOARD_BOOT=uboot-mmc RUN_BOUND=$RUN_BOUND RETURN_BOUND=$RETURN_BOUND tools/board.sh imx8mp run /img/gm05-texture/zephyr/zephyr.bin /rec gm05-texture"*) ;;
    *) echo "self-test: run line: $out" >&2; return 1 ;;
    esac
    if grep -qE '[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+|/dev/tty' <<< "$out"; then echo "self-test: an address in the run line" >&2; return 1; fi
    mkdir -p "${TMPDIR:-/tmp}/imx8mp-dumps-exists.$$/gm05-texture"
    out=$(DRY=1 cmd_build "${TMPDIR:-/tmp}/imx8mp-dumps-exists.$$" 5)
    rm -rf "${TMPDIR:-/tmp}/imx8mp-dumps-exists.$$"
    case $out in *"exists, a real run refuses it"*) ;; *) echo "self-test: an existing build directory not flagged" >&2; return 1 ;; esac
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
[ $# -ge 1 ] || die "usage: $0 scenes | [--dry-run] build|run ... | --self-test"
verb=$1
shift
case "$verb" in
scenes) cmd_scenes ;;
build) cmd_build "$@" ;;
run) cmd_run "$@" ;;
*) die "unknown command '$verb'" ;;
esac

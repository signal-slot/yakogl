#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/x2-glmark2-scenes-selftest.sh
#
# Host-only checks of tools/cts/x2-glmark2-scenes.sh with a stub board.sh,
# west, zephyr-modules.sh and yakogl-tools, in a scratch git repository. No
# board, no network, nothing outside the scratch directory is written.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
SCRIPT=$HERE/x2-glmark2-scenes.sh
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
check()  { case "$3" in *"$2"*) ok "$1" ;; *) bad "$1: wanted '$2' in: $3" ;; esac; }
absent() { case "$3" in *"$2"*) bad "$1: found '$2' in: $3" ;; *) ok "$1" ;; esac; }
rc_is()  { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1: rc $3, wanted $2"; fi; }

T=$(mktemp -d "${TMPDIR:-/tmp}/x2gm-selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT

# ---- the stubs -------------------------------------------------------------------
S=$T/stub
mkdir -p "$S"
cat > "$S/board.sh" <<'EOF'
#!/bin/bash
# stub board.sh: logs its arguments and the variables that must not leak,
# prints a verdict line shaped as the real one's
echo "args=$* SEL=${SEL-unset} NNW=${NETBOOT_NO_WATCHDOG-unset} UART=${BOARD_UART-unset}" >> "$STUB_LOG"
# the record directory as board.sh names it: <logdir>/<YYYYMMDD-HHMM>-<name>-<sha8 of the image>
a=("$@"); [ "${a[0]}" = --dry-run ] && a=("${a[@]:1}")
echo "rd=${a[3]}/$(date +%Y%m%d-%H%M)-${a[4]-x}-$(sha256sum "${a[2]}" | cut -c1-8)" >> "$STUB_LOG"
[ "$1" = --dry-run ] && { echo "DRY host\$ would run"; exit 0; }
name=${5:-x}
rc=0 verdict=done rec=absent
case "${SEL-}" in
0x1000) rec=594b0c00,09c40064,00000000 ;;
0x1001) rec=594b0c01,00020001,00040003 ;;
0x1003) rec=594b0c03,00020001,00040003 ;;
0x1004) rec=594b0c04,00020001,00040003 ;;
"") rec=53450012,0000002a,20000000 ;;
esac
if [ "${STUB_FAIL_SEL-}" = "${SEL-}" ] && [ -n "${STUB_FAIL_SEL+x}" ]; then rc=5 verdict=no_record rec=absent; fi
echo "board.sh x2 $name rc=$rc verdict=$verdict boot=kexec-dev DEV boot_verdict=- result=none resets=0 record=$rec back_after=60s"
exit $rc
EOF
cat > "$S/west" <<'EOF'
#!/bin/bash
echo "west $*" >> "$STUB_LOG"
d=
while [ $# -gt 0 ]; do [ "$1" = -d ] && d=$2; shift; done
mkdir -p "$d/zephyr"
if [ -n "${STUB_NO_MARKER-}" ]; then echo "image" > "$d/zephyr/zephyr.bin"; else echo "image gc_wdog_arm: armed" > "$d/zephyr/zephyr.bin"; fi
EOF
cat > "$S/modules.sh" <<'EOF'
#!/bin/bash
echo stub-modules
EOF
cat > "$S/ytools" <<'EOF'
#!/bin/bash
# record --words 3 w0 w1 w2
[ "$1" = record ] || exit 2
shift 3
w0=$((16#$1)) w1=$((16#$2)) w2=$((16#$3))
if [ $((w0 >> 8)) = $((0x594b0c)) ]; then
    echo "experiment 0 page $((w0 & 7)): $((w1 & 0xffff)) $((w1 >> 16)) $((w2 & 0xffff)) $((w2 >> 16)) n/a n/a"
else
    echo "GLMARK2 record $1 $2 $3"
fi
EOF
chmod +x "$S"/*

# ---- a scratch repository: main, and a branch off it ----------------------------------
R=$T/repo
git init -q "$R"
git -C "$R" config user.email t@example.com
git -C "$R" config user.name t
mkdir -p "$R/driver/samples/zephyr/glmark2/src"
printf '#define GM_PERF_PAGES 4u\n' > "$R/driver/samples/zephyr/glmark2/src/gm_perf_page.h"
git -C "$R" add -A
git -C "$R" commit -q -m one
git -C "$R" branch -M main
MAIN1=$(git -C "$R" rev-parse HEAD)
git -C "$R" checkout -q -b topic
echo x > "$R/x"
git -C "$R" add x
git -C "$R" commit -q -m topic-only
git -C "$R" checkout -q main

export STUB_LOG=$T/calls.log
export X2SC_REPO=$R X2SC_MAIN_REF=main X2SC_BOARD_SH=$S/board.sh X2SC_WEST=$S/west
export X2SC_MODULES_SH=$S/modules.sh X2SC_YAKOGL_TOOLS=$S/ytools
export ZEPHYR_WS=$T/ws GLMARK2_SRC=$T/gm
mkdir -p "$ZEPHYR_WS" "$GLMARK2_SRC"
# a caller that has these set must not get them through
export NETBOOT_NO_WATCHDOG=1 BOARD_UART=host:1234

run() { : > "$STUB_LOG"; out=$(bash "$SCRIPT" "$@" 2>&1); rc=$?; }

# ---- the table of a good run -----------------------------------------------------------
run "$T/o1" terrain
rc_is "good run: exit 0" 0 "$rc"
check "table header" "scene	commit	selector	verdict line	record words	decoded	ms/frame" "$out"
check "table: no-selector row" "terrain	${MAIN1:0:8}	none	board.sh x2 terrain-nosel rc=0 verdict=done" "$out"
check "table: page 0 row, decoded v0..v3" "page 0: v0=100 v1=2500 v2=0 v3=0" "$out"
check "table: page 0 ms a frame (2500 us)" "2.500" "$out"
check "table: page 1 row, record words" "594b0c01 00020001 00040003" "$out"
check "table: page 1 decoded" "page 1: v0=1 v1=2 v2=3 v3=4" "$out"
absent "no page 4 row when the sample has 4 pages" "0x1004" "$out"
n=$(grep -c 'x2 run' "$STUB_LOG")
[ "$n" = 4 ] && ok "four runs: no selector, 0x1000, 0x1001, 0x1003" || bad "runs: $n"
check "west: -b scene in the arguments" 'CONFIG_GLMARK2_ARGS="-b terrain"' "$(cat "$STUB_LOG")"
check "west: the X2 board" "armadillo_x2/mimx8ml8/a53" "$(cat "$STUB_LOG")"
log=$(cat "$STUB_LOG")
absent "NETBOOT_NO_WATCHDOG never passed on" "NNW=1" "$log"
absent "BOARD_UART never passed on" "UART=host" "$log"
check "SEL set for a page run" "SEL=0x1003" "$log"
check "SEL unset for the plain run" "args=x2 run" "$(grep 'SEL=unset' "$STUB_LOG" | head -1)"
[ -s "$T/o1/table.tsv" ] && ok "table.tsv written" || bad "table.tsv missing"

# ---- page 4 when the sample has it ---------------------------------------------------
printf '#define GM_PERF_PAGES 5u\n' > "$R/driver/samples/zephyr/glmark2/src/gm_perf_page.h"
git -C "$R" commit -q -am five-pages
MAIN2=$(git -C "$R" rev-parse HEAD)
run "$T/o2" shadow refract
rc_is "page 4: exit 0" 0 "$rc"
check "page 4: 0x1004 row" "0x1004" "$out"
n=$(grep -c 'x2 run' "$STUB_LOG")
[ "$n" = 10 ] && ok "two scenes, five runs each" || bad "runs: $n"
check "second scene row" "refract	${MAIN2:0:8}	0x1004" "$out"
# --commit of the older main commit
run --commit "$MAIN1" "$T/o3" terrain
rc_is "--commit on main: exit 0" 0 "$rc"
absent "--commit: the old sample has no page 4" "0x1004" "$out"
check "--commit: the commit shown" "${MAIN1:0:8}" "$out"

# ---- refusals ----------------------------------------------------------------------
git -C "$R" checkout -q topic
run "$T/o4" terrain
rc_is "HEAD not on origin/main: refused (exit 3)" 3 "$rc"
check "not on main: the message" "not an ancestor of main" "$out"
[ ! -s "$STUB_LOG" ] && ok "not on main: no build, no board call" || bad "not on main: stub was called: $(cat "$STUB_LOG")"
run --commit "$(git -C "$R" rev-parse topic)" "$T/o4b" terrain
rc_is "--commit not on main: refused" 3 "$rc"
git -C "$R" checkout -q main

echo dirty >> "$R/driver/samples/zephyr/glmark2/src/gm_perf_page.h"
run "$T/o5" terrain
rc_is "tracked changes: refused" 3 "$rc"
git -C "$R" checkout -q -- .

STUB_NO_MARKER=1 run "$T/o6" terrain
rc_is "no watchdog marker: refused (exit 3)" 3 "$rc"
check "no marker: the message" "gc_wdog_arm: armed" "$out"
absent "no marker: board.sh never called" "x2 run" "$(cat "$STUB_LOG")"

run "$T/o7"
rc_is "no scene: usage (exit 2)" 2 "$rc"
run "$T/o7" 'bad scene;rm'
rc_is "odd scene name: refused" 2 "$rc"

# ---- dry-run ------------------------------------------------------------------------
run --dry-run "$T/o8" terrain
rc_is "dry-run: exit 0" 0 "$rc"
check "dry-run: the image is built" "west build" "$(cat "$STUB_LOG")"
n=$(grep -c 'x2 run' "$STUB_LOG")
nd=$(grep -c 'args=--dry-run x2 run ' "$STUB_LOG")
[ "$n" = 5 ] && [ "$nd" = 5 ] && ok "dry-run: every board.sh call carries --dry-run" || bad "dry-run: $nd of $n calls"
check "dry-run: table still printed" "terrain	${MAIN2:0:8}	0x1000" "$out"

# ---- --conf: an allowlist, a tag in the commit column and the directory ------------------
run --dry-run --conf CONFIG_GLMARK2_GC_TILED_RT=y "$T/o10" terrain
rc_is "--conf accepted key: exit 0" 0 "$rc"
check "--conf: -D passed to west build" "-DCONFIG_GLMARK2_GC_TILED_RT=y" "$(grep '^west' "$STUB_LOG")"
check "--conf: commit column carries +tiled_rt" "terrain	${MAIN2:0:8}+tiled_rt	0x1000" "$out"
[ -d "$T/o10/terrain-tiled_rt" ] && ok "--conf: record dir suffixed" || bad "--conf: no $T/o10/terrain-tiled_rt"
run --dry-run "$T/o10" terrain
absent "no --conf: no -D of the knob" "GC_TILED_RT" "$(grep '^west' "$STUB_LOG")"
absent "no --conf: plain commit column" "+tiled_rt" "$out"
[ -d "$T/o10/terrain" ] && ok "off and on runs share one outdir" || bad "no plain record dir beside the tagged one"
run --dry-run --conf CONFIG_GLMARK2_GC_TILED_RT=y --conf CONFIG_GLMARK2_GC_TILED_RT=n "$T/o11" terrain
check "--conf repeated: both passed" "-DCONFIG_GLMARK2_GC_TILED_RT=n" "$(grep '^west' "$STUB_LOG")"
run --dry-run --conf CONFIG_GLMARK2_FRAME_DUMP=y "$T/o12" terrain
rc_is "--conf key outside the allowlist: refused (exit 2)" 2 "$rc"
check "--conf refusal: the message" "not in the allowlist" "$out"
[ ! -s "$STUB_LOG" ] && ok "--conf refused: no build, no board call" || bad "--conf refused: stub was called"
run --dry-run --conf 'CONFIG_GLMARK2_GC_TILED_RT=y;x' "$T/o12" terrain
rc_is "--conf odd value: refused" 2 "$rc"
run --dry-run --conf "$T/o12" terrain
rc_is "--conf without KEY=VAL: refused" 2 "$rc"

# ---- the board does not come back: stop, no retry -----------------------------------------
STUB_FAIL_SEL=0x1001 run "$T/o9" terrain refract
rc_is "board not back: exit 5" 5 "$rc"
check "board not back: the message" "needs a person" "$out"
n=$(grep -c 'x2 run' "$STUB_LOG")
[ "$n" = 3 ] && ok "stopped at the failing run: 3 board calls (no retry, no further selector, no next scene)" || bad "board calls: $n"
check "the failed row is in the table" "terrain	${MAIN2:0:8}	0x1001" "$out"
absent "next scene never built" "refract" "$(grep '^west' "$STUB_LOG")"

# ---- session A's run sheet (docs/notes/gc-tiled-wire.md, "The invocations") ---------------------
# Every line of the sheet, SHA the main tip, <outdir> outside the tree. rc 0 is the script
# accepting the arguments; the logged west line is the image's build options; the stub
# board.sh's rd= line is the record directory name the real one derives (board.sh, RD=).
SHA=$MAIN2
SH8=${MAIN2:0:8}
SA=$T/sa
sheet() { # <label> <args...>: one line of the sheet is accepted
    local label=$1
    shift
    run "$@"
    rc_is "sheet: $label: accepted" 0 "$rc"
}
sheet 'dry-run, off, terrain shadow refract' --dry-run --commit "$SHA" "$SA/dry" terrain shadow refract
w=$(grep '^west' "$STUB_LOG")
[ "$(grep -c '^west' "$STUB_LOG")" = 3 ] && ok "sheet: dry off: three images" || bad "sheet: dry off: west calls"
absent "sheet: dry off: no knob in any image" "GC_TILED_RT" "$w"
check "sheet: dry off: commit column" "shadow	$SH8	0x1000" "$out"
absent "sheet: dry off: no +tiled_rt" "+tiled_rt" "$out"
sheet 'dry-run, on, terrain shadow refract' --dry-run --commit "$SHA" --conf CONFIG_GLMARK2_GC_TILED_RT=y "$SA/dry" terrain shadow refract
[ "$(grep -c -- '-DCONFIG_GLMARK2_GC_TILED_RT=y' "$STUB_LOG")" = 3 ] && ok "sheet: dry on: knob in all three images" || bad "sheet: dry on: knob"
for sc in terrain shadow refract; do
    check "sheet: dry on: $sc commit column" "$sc	$SH8+tiled_rt	0x1000" "$out"
    if [ -d "$SA/dry/$sc-tiled_rt" ] && [ -d "$SA/dry/$sc" ]; then ok "sheet: $sc off and on directories share the outdir"; else bad "sheet: $sc directories"; fi
done
for sc in terrain shadow refract; do
    for arm in off on; do
        if [ $arm = on ]; then sheet "$sc on" --commit "$SHA" --conf CONFIG_GLMARK2_GC_TILED_RT=y "$SA/run" "$sc"
        else sheet "$sc off" --commit "$SHA" "$SA/run" "$sc"; fi
        log=$(cat "$STUB_LOG")
        d=$sc
        [ $arm = on ] && d=$sc-tiled_rt
        check "sheet: $sc $arm: scene argument of the image" "CONFIG_GLMARK2_ARGS=\"-b $sc\"" "$log"
        check "sheet: $sc $arm: X2 board" "armadillo_x2/mimx8ml8/a53" "$log"
        if [ $arm = on ]; then
            check "sheet: $sc on: image built with the knob" "-DCONFIG_GLMARK2_GC_TILED_RT=y" "$(grep '^west' "$STUB_LOG")"
            check "sheet: $sc on: commit column" "$SH8+tiled_rt" "$out"
        else
            absent "sheet: $sc off: image without the knob" "GC_TILED_RT" "$(grep '^west' "$STUB_LOG")"
            absent "sheet: $sc off: commit column" "+tiled_rt" "$out"
        fi
        [ "$(grep -c '^rd=' "$STUB_LOG")" = 5 ] && ok "sheet: $sc $arm: five runs (none, 0x1000, 0x1001, 0x1003, 0x1004)" || bad "sheet: $sc $arm: runs"
        if grep -Eq "^rd=$SA/run/$d/[0-9]{8}-[0-9]{4}-$sc-nosel-[0-9a-f]{8}\$" "$STUB_LOG"; then
            ok "sheet: $sc $arm: record dir <logdir>/<YYYYMMDD-HHMM>-<name>-<sha8>"
        else bad "sheet: $sc $arm: record dir: $(grep '^rd=' "$STUB_LOG" | head -1)"; fi
        absent "sheet: $sc $arm: NETBOOT_NO_WATCHDOG never set" "NNW=1" "$log"
    done
done
# the sheet reads <sha8> as the commit's; board.sh's is the image's sha256 (a mismatch of the note)
absent "sheet: the record dir's sha8 is the image's, not the commit's" "-nosel-$SH8" "$(grep '^rd=' "$STUB_LOG")"
# the gl_test boots, as the sheet writes them: the two unsets, SEL, the image, logdir and name
echo "image gc_wdog_arm: armed" > "$T/gt.bin"
for c in 3:gt-case3-off 3:gt-case3-on 44:gt-case44-off 44:gt-case44-on 25:gt-case169-off 25:gt-case169-on; do
    : > "$STUB_LOG"
    env -u NETBOOT_NO_WATCHDOG -u BOARD_UART SEL="${c%%:*}" "$S/board.sh" x2 run "$T/gt.bin" "$SA/gt" "${c#*:}" > /dev/null 2>&1
    check "sheet: SEL=${c%%:*} ${c#*:}: selector, name, no leaks" "args=x2 run $T/gt.bin $SA/gt ${c#*:} SEL=${c%%:*} NNW=unset UART=unset" "$(cat "$STUB_LOG")"
    if [[ ${c#*:} =~ ^[A-Za-z0-9_-]+$ ]]; then ok "sheet: name ${c#*:} passes board.sh's name rule"; else bad "name ${c#*:}"; fi
done
# the gl_test images, DRYRUN=1 (needs no variable): first case 0 for cases 3 and 44, 144 for 169
GB=$HERE/imx8mp-gltest-build.sh
for img in gt0:0 gt144:144; do
    for arm in off on; do
        args=()
        [ $arm = on ] && args=(-DCONFIG_GL_TEST_GC_TILED_RT=y)
        out=$(IMX8MP_BOARD=x2 DRYRUN=1 bash "$GB" "${img%%:*}-$arm" "${img#*:}" ${args[@]+"${args[@]}"} 2>&1)
        rc=$?
        rc_is "sheet: gl_test ${img%%:*} $arm: accepted" 0 "$rc"
        check "sheet: gl_test ${img%%:*} $arm: X2 board" "armadillo_x2/mimx8ml8/a53" "$out"
        check "sheet: gl_test ${img%%:*} $arm: first case" "CONFIG_GC_GL_TEST_SWEEP_FIRST=${img#*:}" "$out"
        check "sheet: gl_test ${img%%:*} $arm: watchdog option" "CONFIG_GC_BOARD_WDOG_SELF_ARM=y" "$out"
        if [ $arm = on ]; then check "sheet: gl_test ${img%%:*} on: knob" "CONFIG_GL_TEST_GC_TILED_RT=y" "$out"
        else absent "sheet: gl_test ${img%%:*} off: no knob" "GC_TILED_RT" "$out"; fi
    done
done

echo
if [ "$fail" = 0 ]; then echo "x2-glmark2-scenes-selftest: all ok"; else echo "x2-glmark2-scenes-selftest: $fail failed"; exit 1; fi

#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/x2-glmark2-pictures-selftest.sh
#
# Host-only checks of tools/cts/x2-glmark2-pictures.sh with a stub west,
# zephyr-modules.sh, board.sh and screenshot command, in a scratch git
# repository under $TMPDIR. The stub board.sh writes a run directory whose
# console.txt is the fixture tools/cts/testdata/x2-glmark2-pictures/console.txt
# with the scene's line of pictures.txt and the 'board: holding' line; the
# records it leads to are judged by tools/cts/imx8mp-glmark2-record-check.py.
# No board, no network, no real build; nothing outside the scratch directory
# is written.
#
# Row to add to tools/check.sh later (not added here: another plan owns it):
#   tools/cts/x2-glmark2-pictures-selftest.sh, with the other
#   tools/cts/*-selftest.sh host-script rows.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
REAL=$(cd "$HERE/../.." && pwd)
SCRIPT=$HERE/x2-glmark2-pictures.sh
FIX=$HERE/testdata/x2-glmark2-pictures
CHECK=$HERE/imx8mp-glmark2-record-check.py
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
check()  { case "$3" in *"$2"*) ok "$1" ;; *) bad "$1: wanted '$2' in: $3" ;; esac; }
absent() { case "$3" in *"$2"*) bad "$1: found '$2' in: $3" ;; *) ok "$1" ;; esac; }
rc_is()  { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1: rc $3, wanted $2"; fi; }
is() { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1: '$3', wanted '$2'"; fi; }

T=$(mktemp -d "${TMPDIR:-/tmp}/x2gp-selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
# nothing of the operator's environment reaches the script
unset BOARD_HOST BOARD_UART BOARD_BOOT SEL NETBOOT_NO_WATCHDOG X2_SHOT_CMD X2_SHOT_WAIT \
    X2_STOP_CONTAINERS X2_STOP_APPS ZEPHYR_WORKSPACE WEST_VENV ZEPHYR_VENV ZEPHYR_SDK_INSTALL_DIR
export STUB_LOG=$T/stub.log FIX
: > "$STUB_LOG"

# ---- the stubs ------------------------------------------------------------------
S=$T/stub
mkdir -p "$S"
cat > "$S/west" <<'EOF'
#!/bin/bash
# stub west: one argument a line in the log; the image and .config a build
# would leave. CMake's -D: an unescaped ';' splits the value (only the first
# part reaches Kconfig), '\;' is a ';'. A build path with ':' fails as cargo
# does (LD_LIBRARY_PATH).
{ echo "west-argv-begin"; printf '%s\n' "$@"; echo "west-argv-end"; } >> "$STUB_LOG"
d= args=
for a in "$@"; do
    case $prev in -d) d=$a ;; esac
    case $a in -DCONFIG_GLMARK2_ARGS=*) args=${a#-DCONFIG_GLMARK2_ARGS=} ;; esac
    prev=$a
done
case $d in *:*) echo "error: failed to join paths from \$LD_LIBRARY_PATH: path segment contains separator" >&2; exit 1 ;; esac
v=$(printf '%s' "$args" | sed 's/\\;/\x01/g; s/;.*//; s/\x01/;/g')
case $v in \"*\") ;; *) v="\"${v#\"}\"" ;; esac
mkdir -p "$d/zephyr" "$d/cargo-drivers"
echo "CONFIG_GLMARK2_ARGS=$v" > "$d/zephyr/.config"
if [ -n "${STUB_NO_MARKER-}" ]; then printf 'image %s' "$args" > "$d/zephyr/zephyr.bin"
else printf 'image gc_wdog_arm: armed %s' "$args" > "$d/zephyr/zephyr.bin"; fi
EOF
cat > "$S/modules.sh" <<'EOF'
#!/bin/bash
echo stub-modules
EOF
cat > "$S/deps-env.sh" <<'EOF'
#!/bin/bash
exit 0
EOF
cat > "$S/board.sh" <<'EOF'
#!/bin/bash
# stub board.sh x2 run: logs its arguments and environment; writes the run
# directory board.sh names <logdir>/<YYYYMMDD-HHMM>-<name>-<image sha8> with
# meta.txt and a console.txt made from the fixture
echo "board args=$* STOP=${X2_STOP_CONTAINERS-unset} BOOT=${BOARD_BOOT-unset} SEL=${SEL-unset} NNW=${NETBOOT_NO_WATCHDOG-unset} UART=${BOARD_UART+set}" >> "$STUB_LOG"
if [ "$1" = --dry-run ]; then echo "DRY host\$ board.sh x2 run (stub)"; exit 0; fi
img=$3 logdir=$4 name=$5
nn=$((10#${name:2:2}))
rd=$logdir/${STUB_STAMP:-$(date +%Y%m%d-%H%M)}-$name-$(sha256sum "$img" | cut -c1-8)
# as board.sh: a record is never overwritten
[ -e "$rd" ] && { echo "board.sh: $rd exists: a record is never overwritten" >&2; exit 3; }
mkdir -p "$rd"
printf '%s\n' board=x2 "board_measured=Armadillo-X2 (i.MX 8M Plus, GC7000UL), stands in for the Verdin iMX8MP" boot=kexec-dev > "$rd/meta.txt"
pic=$(grep -v '^#' "$FIX/pictures.txt" | sed -n "${nn}p")
grep -v '^#' "$FIX/console.txt" | while IFS= read -r line; do
    [ "$line" = "@PICTURE@" ] && line=$pic
    case $line in "board: holding"*) [ "${STUB_NO_HOLD-}" = "$nn" ] && continue ;; esac
    case $line in "glmark2-picture:"*) [ "${STUB_NO_PICTURE-}" = "$nn" ] && continue ;; esac
    printf '%(%H:%M:%S)T %s\n' -1 "$line" >> "$rd/console.txt"
done
# the hold: wait for the screenshot when the test takes one
if [ -n "${STUB_WAIT_PHOTO-}" ] && [ "${STUB_NO_HOLD-}" != "$nn" ]; then
    i=0; while [ $i -lt 50 ] && ! ls "$logdir"/../photos/"$(printf %02d "$nn")"-*.png > /dev/null 2>&1; do sleep 0.1; i=$((i + 1)); done
else
    sleep 0.3
fi
rc=0
[ "${STUB_RC_SCENE-}" = "$nn" ] && rc=$STUB_RC
echo "board.sh x2 $name rc=$rc verdict=done boot=kexec-dev DEV"
exit $rc
EOF
python3 -I - "$T/stub.png" <<'EOF'
import struct, sys, zlib
def chunk(t, b):
    return struct.pack(">I", len(b)) + t + b + struct.pack(">I", zlib.crc32(t + b) & 0xffffffff)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)) \
    + chunk(b"IDAT", zlib.compress(b"\0\0\0\0")) + chunk(b"IEND", b"")
open(sys.argv[1], "wb").write(png)
EOF
cat > "$S/shot" <<EOF
#!/bin/bash
echo "shot \$1" >> "\$STUB_LOG"
cp "$T/stub.png" "\$1"
EOF
chmod +x "$S"/*

# ---- a scratch repository: main, and a commit off it -------------------------------
R=$T/repo
# the host's git configuration (commit.gpgsign, hooks, templates) stays out of it
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
git init -q -b main "$R"
git -C "$R" config user.email selftest
git -C "$R" config user.name selftest
git -C "$R" config commit.gpgsign false
mkdir -p "$R/driver/samples/zephyr/glmark2"
echo sample > "$R/driver/samples/zephyr/glmark2/CMakeLists.txt"
git -C "$R" add -A && git -C "$R" commit -q -m main
MAIN=$(git -C "$R" rev-parse HEAD)
git -C "$R" checkout -q -b side
echo side >> "$R/driver/samples/zephyr/glmark2/CMakeLists.txt"
git -C "$R" commit -q -am side
SIDE=$(git -C "$R" rev-parse HEAD)
git -C "$R" checkout -q main

mkdir -p "$T/ws/zephyr" "$T/glmark2/src"
touch "$T/glmark2/src/default-benchmarks.h"
export X2P_REPO=$R X2P_MAIN_REF=main X2P_PICTURES=$REAL/tools/cts/glmark2-pictures.txt \
    X2P_WEST=$S/west X2P_MODULES_SH=$S/modules.sh X2P_DEPS_ENV=$S/deps-env.sh X2P_BOARD_SH=$S/board.sh \
    X2P_POLL=0.1
x2p() { bash "$SCRIPT" "$@"; }

echo "x2-glmark2-pictures selftest"

# ---- 1. list --------------------------------------------------------------------
out=$(x2p list); rc=$?
rc_is "list: exit 0" 0 $rc
is "list: 33 lines" 33 "$(grep -c . <<< "$out")"
labels=$(cut -d' ' -f1 <<< "$out")
is "list: every label is gmNN-<slug>, slug [a-z0-9-] only" 33 "$(grep -cE '^gm[0-9]{2}-[a-z0-9-]+$' <<< "$labels")"
is "list: labels unique" 33 "$(sort -u <<< "$labels" | grep -c .)"
is "list: NN runs 01..33 in order" "$(seq -f 'gm%02g' 1 33 | paste -sd' ')" "$(cut -c1-4 <<< "$labels" | paste -sd' ')"
is "list: the scenes are the host column's, in its order" \
    "$(awk -F' [|] ' '$1 == "host" { print $2 }' "$X2P_PICTURES")" "$(cut -d' ' -f2- <<< "$out")"
check "list: gm13 is the first effect2d kernel" "gm13-effect2d-kernel-0-1-0-1-4-1-0-1-0 [effect2d] kernel=0,1,0;1,-4,1;0,1,0;" "$out"
check "list: gm22 is jellyfish" "gm22-jellyfish-default [jellyfish] <default>" "$out"
# the photo names the record check expects: its slug() and this label's agree
want=$(python3 -I - "$CHECK" <<'EOF'
import importlib.util, sys
spec = importlib.util.spec_from_file_location("rc", sys.argv[1]); m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
for i, s in enumerate(m.SCENES, 1): print("gm%02d-%s" % (i, m.slug(s)))
EOF
)
is "list: labels are gmNN-<record check slug()>" "$want" "$labels"

# ---- 2. dry-run build ---------------------------------------------------------------
out=$(ZEPHYR_WS=$T/ws GLMARK2_SRC=$T/glmark2 x2p --dry-run build "$T/img" 2>&1); rc=$?
rc_is "dry-run build: exit 0" 0 $rc
is "dry-run build: 33 west lines" 33 "$(grep -c '^west build ' <<< "$out")"
is "dry-run build: nothing built or created" "" "$(ls "$T" | grep -x img)"
is "dry-run build: west not called" 0 "$(grep -c west-argv-begin "$STUB_LOG")"
check "dry-run build: board and label build dir" "-b armadillo_x2/mimx8ml8/a53 -d $T/img/build-gm01-build-use-vbo-false " "$out"
absent "dry-run build: no build dir named after a scene" "build/[" "$out"
# every printed line, read back by a shell, gives west the intended words
argcheck() { # argcheck NN WANT: line NN's CONFIG_GLMARK2_ARGS word, as a shell reads it
    local line w
    line=$(grep '^west build ' <<< "$out" | sed -n "$1p")
    line=${line%%   #*}
    eval "set -- ${line#west }"
    for w in "$@"; do case $w in -DCONFIG_GLMARK2_ARGS=*) printf '%s' "$w" ;; esac; done
}
is "dry-run build: ':' and '=' (gm01)" '-DCONFIG_GLMARK2_ARGS="-b build:use-vbo=false"' "$(argcheck 1)"
is "dry-run build: ',' and ';' escaped for CMake (gm13)" '-DCONFIG_GLMARK2_ARGS="-b effect2d:kernel=0,1,0\;1,-4,1\;0,1,0\;"' "$(argcheck 13)"
is "dry-run build: gm14 kernel" '-DCONFIG_GLMARK2_ARGS="-b effect2d:kernel=1,1,1,1,1\;1,1,1,1,1\;1,1,1,1,1\;"' "$(argcheck 14)"
is "dry-run build: <default> is the bare scene (gm22)" '-DCONFIG_GLMARK2_ARGS="-b jellyfish"' "$(argcheck 22)"
is "dry-run build: many ':' (gm18)" \
    '-DCONFIG_GLMARK2_ARGS="-b buffer:columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map"' "$(argcheck 18)"
check "dry-run build: unset GLMARK2_SRC shown as a placeholder" "'-DGLMARK2_SRC=<GLMARK2_SRC>'" "$(x2p --dry-run build "$T/img" 1 2>&1)"
out=$(x2p --dry-run build "$T/img" 3 13 2>&1)
is "dry-run build: a selection builds only those" "2" "$(grep -c '^west build ' <<< "$out")"
check "dry-run build: --commit off HEAD prints the worktree" "worktree add --detach $T/img/tree-${MAIN:0:8} ${MAIN:0:8}" \
    "$(git -C "$R" checkout -q side; x2p --dry-run --commit main build "$T/img" 1 2>&1; git -C "$R" checkout -q main)"

# ---- 3. refusals -----------------------------------------------------------------------
out=$(x2p --dry-run --commit side build "$T/img" 2>&1); rc=$?
rc_is "build: a commit off main is refused (exit 3)" 3 $rc
check "build: the refusal says why" "not an ancestor of main" "$out"
out=$(x2p --dry-run build "$T/a:b" 2>&1); rc=$?
rc_is "build: an out-dir with ':' is refused" 2 $rc
out=$(x2p --dry-run build "$T/img" 34 2>&1); rc=$?
rc_is "build: scene 34 is refused" 2 $rc
out=$(x2p --dry-run build "$T/img" 'texture:texture-filter=mipmap' 2>&1); rc=$?
rc_is "build: a bare glmark2 scene is not a selector" 2 $rc
echo dirty >> "$R/driver/samples/zephyr/glmark2/CMakeLists.txt"
out=$(x2p --dry-run build "$T/img" 2>&1); rc=$?
rc_is "build: HEAD with tracked changes is refused" 3 $rc
git -C "$R" checkout -q -- .

# ---- 4. build with the stub west ------------------------------------------------------
: > "$STUB_LOG"
out=$(ZEPHYR_WS=$T/ws GLMARK2_SRC=$T/glmark2 x2p build "$T/img" 2>&1); rc=$?
rc_is "build: exit 0" 0 $rc
is "build: 33 sha256 lines" 33 "$(grep -cE '^gm[0-9]{2}-[a-z0-9-]+ sha256=[0-9a-f]{64} commit='"${MAIN:0:8}"'$' <<< "$out")"
is "build: only commit and the 33 label dirs remain" "34" "$(ls "$T/img" | wc -l)"
is "build: each label dir holds zephyr.bin and zephyr.bin.sha256 only" "zephyr.bin zephyr.bin.sha256" \
    "$(ls "$T/img"/*/ | grep -v '^$' | grep -v ':$' | sort -u | paste -sd' ')"
is "build: commit file" "$MAIN" "$(cat "$T/img/commit")"
is "build: sha256 files verify" 33 "$(for d in "$T"/img/gm*/; do (cd "$d" && sha256sum -c --status zephyr.bin.sha256 && echo ok); done | grep -c ok)"
# the words west got for gm13: the escaped kernel reaches .config whole
w13=$(awk '/^west-argv-begin$/ { n++; next } n == 13 && /^-DCONFIG_GLMARK2_ARGS=/' "$STUB_LOG")
is "build: west argv for gm13" '-DCONFIG_GLMARK2_ARGS="-b effect2d:kernel=0,1,0\;1,-4,1\;0,1,0\;"' "$w13"
out=$(ZEPHYR_WS=$T/ws GLMARK2_SRC=$T/glmark2 x2p build "$T/img" 5 2>&1)
check "build: a built image is kept, not rebuilt" "(kept)" "$out"
out=$(ZEPHYR_WS=$T/ws GLMARK2_SRC=$T/glmark2 STUB_NO_MARKER=1 x2p build "$T/img2" 1 2>&1); rc=$?
rc_is "build: an image without the watchdog marker fails (exit 4)" 4 $rc
is "build: nothing kept of it" "no" "$([ -e "$T/img2/gm01-build-use-vbo-false" ] || [ -e "$T/img2/build-gm01-build-use-vbo-false" ] && echo yes || echo no)"
# the .config check catches a ';' CMake would have split: a west that loses the escape
cat > "$S/west-noesc" <<'EOF'
#!/bin/bash
# a west whose CMake drops what follows the first ';', escaped or not
d= args=
for a in "$@"; do case $prev in -d) d=$a ;; esac; case $a in -DCONFIG_GLMARK2_ARGS=*) args=${a#*=} ;; esac; prev=$a; done
v=$(printf '%s' "$args" | sed 's/\\;.*//; s/;.*//'); case $v in *\") ;; *) v="$v\"" ;; esac
mkdir -p "$d/zephyr"; echo "CONFIG_GLMARK2_ARGS=$v" > "$d/zephyr/.config"; echo "gc_wdog_arm: armed" > "$d/zephyr/zephyr.bin"
EOF
chmod +x "$S/west-noesc"
out=$(ZEPHYR_WS=$T/ws GLMARK2_SRC=$T/glmark2 X2P_WEST=$S/west-noesc x2p build "$T/img3" 13 2>&1); rc=$?
rc_is "build: a .config other than the scene fails (exit 4)" 4 $rc
check "build: and says so" ".config has" "$out"
out=$(ZEPHYR_WS=$T/ws GLMARK2_SRC=$T/glmark2 x2p build "$T/img3" 1 2>&1)
git -C "$R" checkout -q side
out=$(ZEPHYR_WS=$T/ws GLMARK2_SRC=$T/glmark2 x2p --commit main build "$T/img4" 1 2>&1); rc=$?
git -C "$R" checkout -q main
rc_is "build: --commit main from a side HEAD builds" 0 $rc
is "build: the worktree is removed afterwards" "no" "$([ -e "$T/img4/tree-${MAIN:0:8}" ] && echo yes || echo no)"
check "build: the worktree's sample was built" "$T/img4/tree-${MAIN:0:8}/driver/samples/zephyr/glmark2" "$(cat "$STUB_LOG")"

# ---- 5. dry-run run ----------------------------------------------------------------
: > "$STUB_LOG"
mkdir -p "$T/rec"
out=$(X2_SHOT_CMD="$S/shot" x2p --dry-run run "$T/img" "$T/rec" 2>&1); rc=$?
rc_is "dry-run run: exit 0" 0 $rc
is "dry-run run: 33 board.sh lines" 33 "$(grep -c '^X2_STOP_CONTAINERS=1 BOARD_BOOT=kexec-dev tools/board.sh --dry-run x2 run ' <<< "$out")"
check "dry-run run: the board.sh line of gm05" "tools/board.sh --dry-run x2 run $T/img/gm05-texture-texture-filter-mipmap/zephyr.bin $T/rec/" "$out"
is "dry-run run: board.sh called 33 times, each with --dry-run" 33 "$(grep -c '^board args=--dry-run x2 run ' "$STUB_LOG")"
is "dry-run run: board.sh env: X2_STOP_CONTAINERS=1 BOARD_BOOT=kexec-dev, no SEL, no NETBOOT_NO_WATCHDOG" 33 \
    "$(grep -c 'STOP=1 BOOT=kexec-dev SEL=unset NNW=unset' "$STUB_LOG")"
is "dry-run run: 33 shot hook calls" 33 "$(grep -c "^on 'board: holding': $S/shot $T/rec/.*-eye-[0-9a-f]\{8\}/photos/[0-9][0-9]-[a-z0-9-]*\.png$" <<< "$out")"
check "dry-run run: shot hook of gm13" "/photos/13-effect2d-kernel-0-1-0-1-4-1-0-1-0.png" "$out"
is "dry-run run: no record created" "" "$(ls "$T/rec")"
out=$(x2p --dry-run run "$T/img" "$T/rec" 7 2>&1)
check "dry-run run: without X2_SHOT_CMD, shot-now and the wait" "shot-now 07 shading-shading-blinn-phong-inf, then a wait of 15 s" "$out"
echo "$SIDE" > "$T/img4/commit"
out=$(x2p --dry-run run "$T/img4" "$T/rec" 1 2>&1); rc=$?
rc_is "run: images of a commit off main are refused (exit 3)" 3 $rc
cp "$T/img/gm01-build-use-vbo-false/zephyr.bin" "$T/img3/gm01-build-use-vbo-false/zephyr.bin"
printf x >> "$T/img3/gm01-build-use-vbo-false/zephyr.bin"
out=$(x2p --dry-run run "$T/img3" "$T/rec" 1 2>&1); rc=$?
rc_is "run: an image that does not match its sha256 is refused (exit 3)" 3 $rc
out=$(x2p run "$T/img" "$T/rec" 1 2>&1); rc=$?
rc_is "run: without BOARD_UART refused (exit 2)" 2 $rc

# ---- 6. a simulated session: the stub board.sh's console, the shot hook ----------
: > "$STUB_LOG"
out=$(BOARD_UART=stub X2_SHOT_CMD="$S/shot" STUB_WAIT_PHOTO=1 x2p run "$T/img" "$T/rec" 2>&1); rc=$?
rc_is "run: exit 0" 0 $rc
REC=$(ls -d "$T"/rec/*-eye-* 2> /dev/null | head -1)
check "run: the record is <YYYYMMDD-HHMM>-eye-<sha8>" "-eye-" "$(basename "${REC:-none}")"
is "run: board.sh run 33 times, without --dry-run" 33 "$(grep -c '^board args=x2 run .* STOP=1 BOOT=kexec-dev SEL=unset NNW=unset UART=set$' "$STUB_LOG")"
is "run: 33 shots taken on the hold" 33 "$(grep -c '^shot ' "$STUB_LOG")"
is "run: 33 scene lines, all held, shot and pictured" 33 "$(grep -c ' rc=0 holding=yes shot=ok picture=yes$' "$REC/scenes.txt")"
is "run: console.txt holds the 33 picture lines" "$(grep '^glmark2-picture: ' "$FIX/pictures.txt")" "$(cat "$REC/console.txt")"
is "run: meta.txt board, board_measured, commit" "board=x2|board_measured=Armadillo-X2 (i.MX 8M Plus, GC7000UL), stands in for the Verdin iMX8MP|commit=$MAIN" \
    "$(grep -E '^(board|board_measured|commit)=' "$REC/meta.txt" | paste -sd'|')"
is "run: 33 per-label image_sha256 lines" 33 "$(grep -cE '^image_sha256_gm[0-9]{2}-[a-z0-9-]+=[0-9a-f]{64}$' "$REC/meta.txt")"
agg=$(sha256sum < "$REC/images.sha256" | cut -d' ' -f1)
is "run: image_sha256 is the manifest's, the last line" "image_sha256=$agg" "$(tail -1 "$REC/meta.txt")"
is "run: the name's sha8 is the manifest's" "${agg:0:8}" "${REC: -8}"
absent "run: no scratch path in the kept board.sh output" "$T" "$(cat "$REC"/runs/*.out)"
out=$(python3 -I "$CHECK" "$REC" 2>&1); rc=$?
rc_is "record check: complete (exit 0)" 0 $rc
check "record check: verdict" "complete (0 problems)" "$out"

# a resumed session: one scene again into the same record, its line replaced
out=$(BOARD_UART=stub X2_SHOT_CMD="$S/shot" STUB_WAIT_PHOTO=1 STUB_STAMP=20000101-0000 x2p run "$T/img" "$REC" 5 2>&1); rc=$?
rc_is "run: resuming the record for scene 5" 0 $rc
is "run: still 33 picture lines" 33 "$(grep -c '^glmark2-picture: ' "$REC/console.txt")"
is "run: scene 5's line once" 1 "$(grep -c 'texture-filter=mipmap: ' "$REC/console.txt")"
out=$(x2p --dry-run run "$T/img3" "$REC" 1 2>&1); rc=$?
rc_is "run: a record of other images is refused (exit 3)" 3 $rc

# no shot command: shot-now lines, photographs missing; a scene without the
# hold and a scene without a picture line
rm -rf "$T/rec2"; mkdir -p "$T/rec2"
out=$(BOARD_UART=stub X2_SHOT_WAIT=0 STUB_NO_HOLD=3 STUB_NO_PICTURE=4 x2p run "$T/img" "$T/rec2" 2>&1); rc=$?
rc_is "run without shots: a scene without a picture line is exit 4" 4 $rc
is "run without shots: 32 shot-now lines (not for the scene without the hold)" 32 "$(grep -c '^shot-now [0-9][0-9] [a-z0-9-]*$' <<< "$out")"
check "run without shots: shot-now of gm13" "shot-now 13 effect2d-kernel-0-1-0-1-4-1-0-1-0" "$out"
REC2=$(ls -d "$T"/rec2/*-eye-* | head -1)
check "run without shots: scene 3 not held" "03 gm03-texture-texture-filter-nearest rc=0 holding=no shot=none picture=yes" "$(cat "$REC2/scenes.txt")"
check "run without shots: scene 4 no picture" "04 gm04-texture-texture-filter-linear rc=0 holding=yes shot=missing picture=no" "$(cat "$REC2/scenes.txt")"
out=$(python3 -I "$CHECK" "$REC2" 2>&1); rc=$?
rc_is "record check without shots: incomplete" 1 $rc
is "record check without shots: 33 photograph problems and scene 4's console line, nothing else" \
    "34" "$(grep -c . <<< "$out" | awk '{ print $1 - 1 }')"
is "record check without shots: the problems are photographs and the one console line" 34 \
    "$(grep -cE '^(photograph: missing [0-9]{2}-|console: no glmark2-picture line for scene 04 )' <<< "$out")"

# a board that does not come back stops the session
rm -rf "$T/rec3"; mkdir -p "$T/rec3"
out=$(BOARD_UART=stub X2_SHOT_WAIT=0 STUB_RC_SCENE=2 STUB_RC=5 x2p run "$T/img" "$T/rec3" 2>&1); rc=$?
rc_is "run: board.sh rc 5 stops the session (exit 5)" 5 $rc
is "run: no scene after it" 2 "$(grep -c . "$(ls -d "$T"/rec3/*-eye-*)/scenes.txt")"

# ---- 7. the script itself -----------------------------------------------------------
bash -n "$SCRIPT" && ok "bash -n" || bad "bash -n"
if command -v shellcheck > /dev/null; then
    shellcheck -S warning "$SCRIPT" > "$T/sc.txt" 2>&1 && ok "shellcheck -S warning" || bad "shellcheck: $(head -5 "$T/sc.txt")"
fi
absent "the script names no address" "192.168." "$(cat "$SCRIPT")"

if [ "$fail" = 0 ]; then echo "x2-glmark2-pictures selftest: all passed"; exit 0; fi
echo "x2-glmark2-pictures selftest: $fail failed"
exit 1

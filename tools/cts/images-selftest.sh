#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/images-selftest.sh
#
# Host-only checks of tools/cts/am62p-deqp-images.sh and imx8mp-gltest-build.sh,
# the defects the round-1 board sessions found in them, and of
# tools/cts/pi4-deqp-images.sh (section 7: the gles2 path as it was, the
# gles3 path's arguments, the stage 1 check). No board, no Zephyr, no network:
# the scripts run from a scratch copy of the repository layout with a fake
# `west` that writes a zephyr.bin and a .config (it fails for a filter that
# contains "FAILME", and writes no image for one that contains "NOIMAGE"), and
# a fake `cmake` / `ninja` pair for the Pi 4 builder that records the cmake
# arguments it was given.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
# check NAME WANT GOT: GOT must contain WANT
check() { case "$3" in *"$2"*) ok "$1" ;; *) bad "$1: wanted '$2' in: $3" ;; esac; }
# absent NAME NOT GOT: GOT must not contain NOT
absent() { case "$3" in *"$2"*) bad "$1: found '$2' in: $3" ;; *) ok "$1" ;; esac; }

T=$(mktemp -d "${TMPDIR:-/tmp}/images-selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
R=$T/repo
mkdir -p "$R/tools/cts" "$R/driver/src/fw" "$R/driver/samples/zephyr/deqp" \
    "$R/driver/samples/zephyr/gl_test" "$T/bin" "$T/ws/zephyr" \
    "$T/ws/modules/fs/fatfs" "$T/work/cts" "$T/work/build/modules/gles2" "$T/venv/bin"
cp "$HERE/am62p-deqp-images.sh" "$HERE/imx8mp-gltest-build.sh" "$HERE/imx8mp-env.sh" \
    "$HERE/zephyr-modules.sh" "$HERE/pi4-deqp-images.sh" "$R/tools/cts/"
# the fake conformance check passes, and appends its arguments to $CHECK_LOG
# when that is set (section 7 reads which flags the Pi 4 builder gave it)
printf 'import os, sys\nwith open(os.environ.get("CHECK_LOG", os.devnull), "a") as f:\n    f.write(" ".join(sys.argv[1:]) + "\\n")\nsys.exit(0)\n' \
    > "$R/tools/cts/check-deqp-images.py"
: > "$R/driver/src/fw/pvr_fw_blob.h"
: > "$T/work/build/modules/gles2/libdeqp-gles2-package.a"
cat > "$T/bin/west" <<'EOF'
#!/bin/bash
# fake west: "list" prints nothing; "build" honours -d DIR and the filter in CONFIG_DEQP_CASE
[ "$1" = list ] && exit 0
args="$*"
while [ $# -gt 0 ]; do [ "$1" = -d ] && { d=$2; break; }; shift; done
mkdir -p "$d/zephyr"
case "$args" in *FAILME*) echo "fake west: failing" >&2; exit 1 ;; esac
case "$args" in *NOIMAGE*) exit 0 ;; esac
echo "$args" > "$d/zephyr/zephyr.bin"
echo "${ZEPHYR_BASE:-}" > "$d/zephyr/zephyr_base"
: > "$d/zephyr/.config"
EOF
chmod +x "$T/bin/west"
cp "$T/bin/west" "$T/west.fake"   # section 6 removes it; section 7 puts it back
printf 'export PATH=%s:$PATH\n' "$T/bin" > "$T/venv/bin/activate"
S=$R/tools/cts/am62p-deqp-images.sh
# the caller's environment must not reach the scripts under test (the node sets WEST_VENV, a
# TREE_ROWS in the shell would select rows, ...)
unset WEST_VENV TREE_ROWS DEQP_MODULE ZEPHYR_BASE CLUSTER_DIR DEQP_GL_CONFIG_NAME BOARD
export ZEPHYR_WS=$T/ws DEQP_WORK=$T/work PATH="$T/bin:$PATH"
git init -q "$R" 2>/dev/null

# a table of 4 unlabelled images, the last two failing and writing no image
printf 'dEQP-GLES2.a.*\ndEQP-GLES2.FAILME.*\n# comment\n\ndEQP-GLES2.c.*\n' > "$T/plain.txt"

echo
echo "== 1. --dry-run honours TREE_ROWS (rows count from 0)"
out=$(TREE_ROWS=1-2 "$S" --dry-run "$T/plain.txt" "$T/o" t 2>&1)
check "row 1 listed" "2 img2-t dEQP-GLES2.FAILME.*" "$out"
check "row 2 listed" "3 img3-t dEQP-GLES2.c.*" "$out"
absent "row 0 not listed" "img1-t" "$out"
out=$(TREE_ROWS=3- "$S" --dry-run "$T/plain.txt" "$T/o" t 2>&1); rc=$?
[ "$rc" = 2 ] && ok "open range 3- refused (rc 2)" || bad "TREE_ROWS=3-: rc=$rc: $out"
out=$(TREE_ROWS=x "$S" --dry-run "$T/plain.txt" "$T/o" t 2>&1); rc=$?
[ "$rc" = 2 ] && ok "bad TREE_ROWS refused (rc 2)" || bad "bad TREE_ROWS: rc=$rc: $out"

out=$(TREE_ROWS=0,x "$S" "$T/plain.txt" "$T/o0" t 2>&1); rc=$?
[ "$rc" = 2 ] && [ ! -e "$T/o0/img1-t" ] && ok "bad spec refused before image 1 is built" || bad "TREE_ROWS=0,x: rc=$rc: $out"

echo
echo "== 2. a failed build does not end the sweep; the script reaches BUILD-EXIT with rc 0"
# defect: bin=$(sha256sum missing | cut) under set -e -o pipefail killed the loop on the first
# failed build; and an unlabelled last row left the loop with the status of a false test
out=$("$S" "$T/plain.txt" "$T/o1" t 2>/dev/null); rc=$?
[ "$rc" = 0 ] && ok "rc 0" || bad "rc=$rc: $out"
check "image 1 built" "img1 rc=0 bin=" "$out"
check "image 2 failed, still reported" "img2 rc=1 bin=none" "$out"
check "image 3 built after the failure" "img3 rc=0 bin=" "$out"
check "BUILD-EXIT printed" "BUILD-EXIT" "$out"

echo
echo "== 3. TREE_ROWS builds only the named rows (resume)"
out=$(TREE_ROWS=2 "$S" "$T/plain.txt" "$T/o2" t 2>/dev/null); rc=$?
[ "$rc" = 0 ] && ok "rc 0" || bad "rc=$rc"
check "row 2 built" "img3 rc=0" "$out"
absent "row 0 not built" "img1" "$out"
[ ! -e "$T/o2/img1-t" ] && [ -d "$T/o2/img3-t" ] && ok "only img3-t exists" || bad "directories: $(ls "$T/o2")"

echo
echo "== 4. a labelled table writes <LABEL>.built, also for a failed image, and still ends well"
printf '# image A1: x\ndEQP-GLES2.a.*\n# image B2: y (with DEQP_GL_CONFIG_NAME=rgba8888d24s8ms4)\ndEQP-GLES2.FAILME.*\n' > "$T/lab.txt"
out=$("$S" "$T/lab.txt" "$T/o3" t 2>/dev/null); rc=$?
[ "$rc" = 0 ] && ok "rc 0" || bad "rc=$rc: $out"
check "A1.built" "A1 rc=0 sha256=" "$(cat "$T/o3/A1.built" 2>&1)"
check "B2.built" "B2 rc=1 sha256=none" "$(cat "$T/o3/B2.built" 2>&1)"

echo
echo "== 5. missing tools are named, not a cryptic failure mid-sweep"
mkdir "$T/nopy"
for t in awk cut sha256sum git dirname mkdir cat sed grep tr env bash west; do
    p=$(command -v "$t") && ln -s "$p" "$T/nopy/$t"
done
out=$(PATH="$T/nopy" "$S" "$T/plain.txt" "$T/o4" t 2>&1); rc=$?
[ "$rc" = 2 ] && ok "no python3: rc 2" || bad "no python3: rc=$rc: $out"
check "names python3" "python3 not on PATH" "$out"
mkdir "$T/nowest"
for t in awk cut sha256sum git dirname mkdir cat sed grep tr env bash python3; do
    p=$(command -v "$t") && ln -s "$p" "$T/nowest/$t"
done
out=$(PATH="$T/nowest" "$S" "$T/plain.txt" "$T/o5" t 2>&1); rc=$?
[ "$rc" = 2 ] && ok "no west: rc 2" || bad "no west: rc=$rc: $out"
check "names west" "west not on PATH" "$out"

echo
echo "== 6. imx8mp-gltest-build.sh"
G=$R/tools/cts/imx8mp-gltest-build.sh
export ZEPHYR_WORKSPACE=$T/ws ZEPHYR_VENV=$T/venv ZEPHYR_SDK_INSTALL_DIR=$T/sdk IMX8MP_OUT=$T/imx
out=$("$G" sweep1 96 2>&1); rc=$?
[ "$rc" = 0 ] && ok "build rc 0" || bad "rc=$rc: $out"
check "rc line" "rc=0" "$out"
check "first case passed on" "CONFIG_GC_GL_TEST_SWEEP_FIRST=96" "$(cat "$T/imx/sweep1/zephyr/zephyr.bin" 2>&1)"
# defect: without the self-arm flag board.sh refuses the image (rc=3), imx8mp-plan1.md Step 3
check "WDOG self-arm flag passed on" "-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y" "$(cat "$T/imx/sweep1/zephyr/zephyr.bin" 2>&1)"
out=$("$G" ../x 96 2>&1); rc=$?
[ "$rc" = 2 ] && ok "path in <name> refused" || bad "path in name: rc=$rc: $out"
out=$("$G" n1 9x 2>&1); rc=$?
[ "$rc" = 2 ] && ok "non-numeric <first case> refused" || bad "first case: rc=$rc: $out"
out=$("$G" n2 96 -DFAILME 2>&1); rc=$?
[ "$rc" = 1 ] && ok "failed build: rc 1" || bad "failed build: rc=$rc: $out"
check "failed build reported" "rc=1" "$out"
out=$("$G" n3 96 -DNOIMAGE 2>&1); rc=$?
[ "$rc" = 1 ] && ok "build without image: rc 1" || bad "no image: rc=$rc: $out"
check "no_image reported" "rc=no_image" "$out"
# a ZEPHYR_BASE preset for another workspace is replaced by the selected workspace's zephyr/, with one line
mkdir -p "$T/other/zephyr"
out=$(ZEPHYR_BASE=$T/other/zephyr "$G" zb1 96 2>&1); rc=$?
[ "$rc" = 0 ] && ok "bogus ZEPHYR_BASE: build rc 0" || bad "bogus ZEPHYR_BASE: rc=$rc: $out"
check "bogus ZEPHYR_BASE: west got the workspace's zephyr" "$(cd "$T/ws/zephyr" && pwd -P)" "$(cat "$T/imx/zb1/zephyr/zephyr_base" 2>&1)"
check "bogus ZEPHYR_BASE: the override is said" "preset ZEPHYR_BASE=$T/other/zephyr is not in ZEPHYR_WORKSPACE" "$out"
out=$(ZEPHYR_BASE=$T/nonexistent "$G" zb2 96 2>&1)
check "missing ZEPHYR_BASE dir: the override is said" "preset ZEPHYR_BASE=$T/nonexistent" "$out"
out=$(ZEPHYR_BASE=$T/ws/zephyr "$G" zb3 96 2>&1)
absent "matching ZEPHYR_BASE: silent" "preset ZEPHYR_BASE" "$out"
out=$(ZEPHYR_WORKSPACE=$T/nows "$G" zb4 96 2>&1); rc=$?
[ "$rc" = 2 ] && ok "workspace without zephyr/: rc 2" || bad "no zephyr dir: rc=$rc: $out"
rm "$T/bin/west"
out=$(PATH="/usr/bin:/bin" "$G" n4 96 2>&1); rc=$?
[ "$rc" = 2 ] && ok "no west: rc 2" || bad "no west: rc=$rc: $out"
check "names west" "west not on PATH" "$out"

echo
echo "== 7. pi4-deqp-images.sh: the gles2 path as it was, the gles3 path's arguments, the stage 1 check"
# The Pi 4 builder runs cmake -GNinja and ninja itself (no west build). The
# fake cmake records its arguments in <B>/cmake.args and writes a build.ninja
# carrying V3D_BUILD (the .built line's build= field reads it); the fake ninja
# writes zephyr/zephyr.bin and zephyr/.config. The fake west (section 6 removed
# it) serves zephyr-modules.sh's `west list`.
P=$R/tools/cts/pi4-deqp-images.sh
cp "$T/west.fake" "$T/bin/west"; chmod +x "$T/bin/west"
cat > "$T/bin/cmake" <<'EOF'
#!/bin/bash
# fake cmake: honours -B DIR, records every argument, writes a build.ninja
while [ $# -gt 0 ]; do [ "$1" = -B ] && { d=$2; break; }; shift; done
mkdir -p "$d"
printf '%s\n' "$@" > "$d/cmake.args"
printf 'build zephyr/zephyr.elf: link\n  DEFINES = -DV3D_BUILD=\\"fake-1\\"\n' > "$d/build.ninja"
case "$*" in *FAILME*) echo "fake cmake: failing" >&2; exit 1 ;; esac
EOF
cat > "$T/bin/ninja" <<'EOF'
#!/bin/bash
# fake ninja: honours -C DIR, writes the image and its resolved .config
while [ $# -gt 0 ]; do [ "$1" = -C ] && { d=$2; break; }; shift; done
mkdir -p "$d/zephyr"
cp "$d/cmake.args" "$d/zephyr/zephyr.bin"
: > "$d/zephyr/.config"
EOF
chmod +x "$T/bin/cmake" "$T/bin/ninja"
# a gles3 stage 1 beside the gles2 one ($T/work): each module its own WORK
mkdir -p "$T/work3/cts" "$T/work3/build/modules/gles3"
: > "$T/work3/build/modules/gles3/libdeqp-gles3-package.a"
GLES3_ARGS='-DDEQP_MODULE=gles3 -DCONFIG_DEQP_REPORT_ES2=n -DCONFIG_YAKOGL_REPORT_ES2=n'
printf '# image A1: x\ndEQP-GLES2.a.*\n# comment\n\n# image T5: y (with texture-2048.conf)\ndEQP-GLES2.t.*\n' > "$T/pi4-es2.txt"
printf '# image A1: x\ndEQP-GLES3.a.*,dEQP-GLES3.b.*\n# image S01: y\ndEQP-GLES3.s.*\n' > "$T/pi4-es3.txt"

# 7a. the dry runs need none of the environment, and the gles2 one adds nothing
out=$(env -u ZEPHYR_WS -u DEQP_WORK "$P" --dry-run "$T/pi4-es2.txt" "$T/p0" 2>&1); rc=$?
want=$(printf 'A1 dEQP-GLES2.a.*\nT5 dEQP-GLES2.t.* [-DEXTRA_CONF_FILE=texture-2048.conf]')
[ "$rc" = 0 ] && [ "$out" = "$want" ] && ok "gles2 dry run: rc 0, exactly the two lines, texture-2048.conf's argument on T5 only" \
    || bad "gles2 dry run: rc=$rc: $out"
out=$(env -u ZEPHYR_WS -u DEQP_WORK DEQP_MODULE=gles3 "$P" --dry-run "$T/pi4-es3.txt" "$T/p0" 2>&1); rc=$?
want=$(printf 'A1 dEQP-GLES3.a.*,dEQP-GLES3.b.* [%s]\nS01 dEQP-GLES3.s.* [%s]' "$GLES3_ARGS" "$GLES3_ARGS")
[ "$rc" = 0 ] && [ "$out" = "$want" ] && ok "gles3 dry run: rc 0, every line carries [$GLES3_ARGS]" \
    || bad "gles3 dry run: rc=$rc: $out"
out=$(DEQP_MODULE=gles3 "$P" --dry-run "$T/pi4-es3.txt" "$T/p0" S01 2>&1)
[ "$out" = "S01 dEQP-GLES3.s.* [$GLES3_ARGS]" ] && ok "gles3 dry run with a label lists that image only" || bad "gles3 dry run S01: $out"
out=$(DEQP_MODULE=gles4 "$P" --dry-run "$T/pi4-es3.txt" "$T/p0" 2>&1); rc=$?
[ "$rc" = 2 ] && ok "DEQP_MODULE=gles4 refused (rc 2)" || bad "DEQP_MODULE=gles4: rc=$rc: $out"
check "the refusal names the two modules" "DEQP_MODULE must be gles2 or gles3" "$out"
[ ! -e "$T/p0" ] && ok "the dry runs wrote nothing" || bad "a dry run wrote $T/p0"

# 7b. the repository's own tables: gles2 adds nothing, gles3 adds its three arguments to every image
out=$("$P" --dry-run "$HERE/pi4-full-tree.txt" "$T/p0" 2>&1); rc=$?
n=$(printf '%s\n' "$out" | grep -c .); m=$(grep -c '^# image ' "$HERE/pi4-full-tree.txt")
[ "$rc" = 0 ] && [ "$n" = "$m" ] && ok "pi4-full-tree.txt: one line per image ($n)" || bad "pi4-full-tree.txt: rc=$rc, $n lines for $m images"
absent "pi4-full-tree.txt: no DEQP_MODULE argument" "DEQP_MODULE" "$out"
absent "pi4-full-tree.txt: no REPORT_ES2 argument" "REPORT_ES2" "$out"
[ "$(printf '%s\n' "$out" | grep -c '\[-DEXTRA_CONF_FILE=texture-2048.conf\]$')" = 1 ] \
    && ok "pi4-full-tree.txt: texture-2048.conf's argument on one image" || bad "pi4-full-tree.txt: texture-2048.conf lines: $out"
out=$(DEQP_MODULE=gles3 "$P" --dry-run "$HERE/pi4-gles3-tree.txt" "$T/p0" 2>&1); rc=$?
n=$(printf '%s\n' "$out" | grep -c .); m=$(grep -c '^# image ' "$HERE/pi4-gles3-tree.txt")
k=$(printf '%s\n' "$out" | grep -c " dEQP-GLES3\..* \[$GLES3_ARGS\]$")
[ "$rc" = 0 ] && [ "$n" = "$m" ] && [ "$k" = "$m" ] && ok "pi4-gles3-tree.txt: one line per image ($n), each a GLES3 filter with [$GLES3_ARGS]" \
    || bad "pi4-gles3-tree.txt: rc=$rc, $n lines, $k with the gles3 arguments, for $m images"

# 7c. the gles2 build: BOARD rpi_4b, the filter, no module argument, the ES 2.0 conformance check
out=$(CHECK_LOG=$T/check2.log "$P" "$T/pi4-es2.txt" "$T/p2" 2>/dev/null); rc=$?
[ "$rc" = 0 ] && ok "gles2 build: rc 0" || bad "gles2 build: rc=$rc: $out"
absent "gles2 build: no DEQP_MODULE header line" "DEQP_MODULE" "$out"
check "A1.built" "A1 rc=0 sha256=" "$(cat "$T/p2/A1.built" 2>&1)"
check "A1.built carries the image's V3D_BUILD" "build=fake-1" "$(cat "$T/p2/A1.built" 2>&1)"
a=$(cat "$T/p2/A1/cmake.args" 2>&1)
check "gles2 cmake: -DBOARD=rpi_4b" "-DBOARD=rpi_4b" "$a"
check "gles2 cmake: the filter" '-DCONFIG_DEQP_CASE="dEQP-GLES2.a.*"' "$a"
absent "gles2 cmake: no DEQP_MODULE" "DEQP_MODULE" "$a"
absent "gles2 cmake: no REPORT_ES2" "REPORT_ES2" "$a"
check "gles2 cmake, T5: texture-2048.conf" "-DEXTRA_CONF_FILE=texture-2048.conf" "$(cat "$T/p2/T5/cmake.args" 2>&1)"
absent "gles2 cmake, A1: no EXTRA_CONF_FILE" "EXTRA_CONF_FILE" "$a"
c=$(cat "$T/check2.log" 2>&1)
check "gles2: the conformance check ran on the resolved .config" "--config $T/p2/A1/zephyr/.config" "$c"
absent "gles2: the conformance check without --gles3" "--gles3" "$c"

# 7d. the gles3 build: the three arguments, the --gles3 check, the header line
out=$(CHECK_LOG=$T/check3.log DEQP_MODULE=gles3 DEQP_WORK=$T/work3 "$P" "$T/pi4-es3.txt" "$T/p3" A1 2>/dev/null); rc=$?
[ "$rc" = 0 ] && ok "gles3 build: rc 0" || bad "gles3 build: rc=$rc: $out"
check "gles3 build: the header names the module and its arguments" "pi4-deqp-images: DEQP_MODULE=gles3 ($GLES3_ARGS)" "$out"
check "A1.built" "A1 rc=0 sha256=" "$(cat "$T/p3/A1.built" 2>&1)"
[ ! -e "$T/p3/S01.built" ] && ok "gles3 build, A1 named: S01 not built" || bad "S01 built: $(cat "$T/p3/S01.built")"
a=$(cat "$T/p3/A1/cmake.args" 2>&1)
for arg in -DBOARD=rpi_4b $GLES3_ARGS "-DDEQP_SRC=$T/work3/cts" "-DDEQP_BUILD=$T/work3/build"; do
    check "gles3 cmake: $arg" "$arg" "$a"
done
check "gles3 cmake: the filter" '-DCONFIG_DEQP_CASE="dEQP-GLES3.a.*,dEQP-GLES3.b.*"' "$a"
check "gles3: the conformance check ran with --gles3" "--config $T/p3/A1/zephyr/.config --gles3" "$(cat "$T/check3.log" 2>&1)"

# 7e. the stage 1 check and the no-reuse rule
out=$(DEQP_MODULE=gles3 "$P" "$T/pi4-es3.txt" "$T/p4" 2>&1); rc=$?
[ "$rc" = 2 ] && [ ! -e "$T/p4" ] && ok "gles3 against the gles2 stage 1: rc 2, nothing built" || bad "gles3 on gles2 stage 1: rc=$rc: $out"
check "the refusal names the stage 1 command" "cts-build-zephyr.sh MODULE=gles3 BOARD=rpi_4b" "$out"
out=$(DEQP_WORK=$T/work3 "$P" "$T/pi4-es2.txt" "$T/p5" 2>&1); rc=$?
[ "$rc" = 2 ] && ok "gles2 against the gles3 stage 1: rc 2" || bad "gles2 on gles3 stage 1: rc=$rc: $out"
out=$("$P" "$T/pi4-es2.txt" "$T/p2" A1 2>/dev/null); rc=$?
[ "$rc" = 0 ] && ok "an existing image directory: rc 0" || bad "existing directory: rc=$rc: $out"
check "an existing image directory is reported, not reused" "A1 rc=exists sha256=none" "$(cat "$T/p2/A1.built" 2>&1)"

echo
echo "== 8. a rerun resumes: common-images.sh reuses a complete build, rebuilds an interrupted one; board-stage.sh clears a stale once-*.bin"
cp "$HERE/common-images.sh" "$R/tools/cts/"
mkdir -p "$T/bin8"
g8() { git -C "$R" -c user.name=t -c user.email=t@t commit -q --allow-empty -m "$1" 2>/dev/null; }
g8 one
# a west that writes an image with the i.MX8MP's arm line and logs each build
cat > "$T/bin8/west" <<'EOS'
#!/bin/bash
[ "$1" = list ] && exit 0
while [ $# -gt 0 ]; do [ "$1" = -d ] && { d=$2; break; }; shift; done
mkdir -p "$d/zephyr"
echo "gc_wdog_arm: armed" > "$d/zephyr/zephyr.bin"
echo "$d" >> "$WEST_LOG"
EOS
chmod +x "$T/bin8/west"
printf '# image GT: gl_test\ngl_test\n# image GU: gl_test\ngl_test\n' > "$T/two.txt"
CI=$R/tools/cts/common-images.sh
export WEST_LOG=$T/west8.log; : > "$WEST_LOG"
out=$(PATH="$T/bin8:$PATH" "$CI" imx8mp "$T/two.txt" "$T/c8" 2>&1); rc=$?
[ "$rc" = 0 ] && [ "$(wc -l < "$WEST_LOG")" = 2 ] && ok "first build: rc 0, two images built" || bad "first build: rc=$rc: $out"
[ -s "$T/c8/GT/.complete" ] && ok "the completion marker is written" || bad "no $T/c8/GT/.complete"
sha=$(sha256sum "$T/c8/GT/zephyr/zephyr.bin" | cut -d' ' -f1)
# complete directories are reused: no build, the same .built line, rc 0
: > "$WEST_LOG"
out=$(PATH="$T/bin8:$PATH" "$CI" imx8mp "$T/two.txt" "$T/c8" 2>&1); rc=$?
[ "$rc" = 0 ] && ok "rerun over a complete set: rc 0" || bad "rerun: rc=$rc: $out"
[ ! -s "$WEST_LOG" ] && ok "a complete directory is reused, west not run" || bad "west ran on a complete set: $(cat "$WEST_LOG")"
check "reuse is said" "GT: $T/c8/GT is complete at this commit with these arguments, reused" "$out"
check "the reused row reports its sha256" "GT rc=0 sha256=$sha" "$(cat "$T/c8/GT.built")"
# an interrupted build (no marker) is rebuilt
rm "$T/c8/GU/.complete"; echo junk > "$T/c8/GU/zephyr/stale"
: > "$WEST_LOG"
out=$(PATH="$T/bin8:$PATH" "$CI" imx8mp "$T/two.txt" "$T/c8" 2>&1); rc=$?
[ "$rc" = 0 ] && [ "$(cat "$WEST_LOG")" = "$T/c8/GU" ] && ok "an incomplete directory is rebuilt, the complete one is not" || bad "incomplete: rc=$rc log=$(cat "$WEST_LOG"): $out"
[ ! -e "$T/c8/GU/zephyr/stale" ] && ok "the rebuild starts from an empty directory" || bad "stale file survived the rebuild"
check "the rebuild is said" "GU: $T/c8/GU is an interrupted build" "$out"
# an image that changed since the marker is not trusted
echo changed > "$T/c8/GT/zephyr/zephyr.bin"
out=$(PATH="$T/bin8:$PATH" "$CI" imx8mp "$T/two.txt" "$T/c8" GT 2>&1); rc=$?
[ "$rc" = 1 ] && check "an image that no longer matches its marker is refused" "GT rc=exists" "$(cat "$T/c8/GT.built")" || bad "changed image: rc=$rc: $out"
# a complete directory from another commit is refused
g8 two
: > "$WEST_LOG"
out=$(PATH="$T/bin8:$PATH" "$CI" imx8mp "$T/two.txt" "$T/c8" GU 2>&1); rc=$?
[ "$rc" = 1 ] && [ ! -s "$WEST_LOG" ] && ok "a directory from another commit: rc 1, not built" || bad "other commit: rc=$rc: $out"
check "another commit's directory is rc=exists" "GU rc=exists sha256=none" "$(cat "$T/c8/GU.built")"

# board-stage.sh: the stale once-*.bin is removed, in one line; a clean directory says nothing
cat > "$T/bin8/ssh" <<'EOS'
#!/bin/bash
# fake ssh: the board's stage directory is $STAGE_FAKE; the remote command is the last argument
cmd=${*: -1}
cmd=${cmd//\/var\/yakogl/$STAGE_FAKE}
eval "${cmd#sudo -n }"
EOS
chmod +x "$T/bin8/ssh"
export STAGE_FAKE=$T/stage; mkdir -p "$STAGE_FAKE"
touch "$STAGE_FAKE/once-a.bin" "$STAGE_FAKE/once-b.bin" "$STAGE_FAKE/keep.txt"
stage() { PATH="$T/bin8:$PATH" BOARD_STAGE_SOURCE_ONLY=1 BOARD_HOST=u@h bash -c ". '$HERE/board-stage.sh'; clear_stale_once" 2>&1; }
out=$(stage)
check "stale one-shot images are named in one line" "removed stale one-shot image(s)" "$out"
check "both names are in the line" "once-a.bin once-b.bin" "$out"
[ -z "$(ls "$STAGE_FAKE" | grep '^once-')" ] && [ -e "$STAGE_FAKE/keep.txt" ] && ok "once-*.bin gone, other files kept" || bad "stage dir: $(ls "$STAGE_FAKE")"
out=$(stage)
[ -z "$out" ] && ok "a clean stage directory says nothing" || bad "clean stage dir printed: $out"

# board-stage.sh: only the entries the board and the set read are required.
# The real deps.toml against a scratch layout: every marker present except
# the ones a case removes.
REPO_ROOT=$(cd "$HERE/../.." && pwd)
L=$T/deps-layout; mkdir -p "$L"
CONNECTED_DEPS_DIR=$L CCFARM_DEPS_DIR=$L "$REPO_ROOT/tools/cts/deps-env.sh" --check am62p > /dev/null 2>&1
for lay in am62p imx8mp rpi_4b; do
    # touch every marker the layout names under $L (a command entry is outside it)
    while read -r mark; do
        case "$mark" in "$L"/*) mkdir -p "$(dirname "$mark")"; [ -e "$mark" ] || : > "$mark" ;; esac
    done < <(CONNECTED_DEPS_DIR=$L CCFARM_DEPS_DIR=$L "$REPO_ROOT/tools/cts/deps-env.sh" --check "$lay" 2> /dev/null | awk '$1 == "missing" { print $3 }')
done
deps_check() { # deps_check LAYOUT WHAT: output and rc of board-stage.sh's check_deps
    (cd "$REPO_ROOT" && CONNECTED_DEPS_DIR=$L CCFARM_DEPS_DIR=$L BOARD_STAGE_SOURCE_ONLY=1 bash -c ". tools/cts/board-stage.sh; check_deps $1 $2" 2>&1)
}
for t in "am62p common" "imx8mp common" "rpi_4b common"; do
    # the trees only the gate reads are absent: the check passes
    rm -rf "$L/mesa-pan" "$L/PCO_SHC" "$L/glsl2usc-cache"
    out=$(deps_check $t); rc=$?
    [ "$rc" = 0 ] && [ -z "$out" ] && ok "$t: passes without MESA_PAN_TREE and PCO_SHC" || bad "$t without MESA_PAN_TREE: rc=$rc: $out"
done
rm -f "$L/zephyr-ws/modules/hal/nxp/zephyr/module.yml"
out=$(deps_check imx8mp common); rc=$?
[ "$rc" = 0 ] && ok "imx8mp common passes without HAL_NXP_DIR (the gate's register check only)" || bad "imx8mp without HAL_NXP_DIR: rc=$rc: $out"
# a tree the board does read, missing: the same stop, naming it
mv "$L/pvr_fw_blob.h" "$L/pvr_fw_blob.h.off"
out=$(deps_check am62p common); rc=$?
[ "$rc" = 1 ] && check "am62p common without PVR_FW_BLOB fails and names it" "PVR_FW_BLOB" "$out" || bad "am62p without PVR_FW_BLOB: rc=$rc: $out"
out=$(deps_check imx8mp common); rc=$?
[ "$rc" = 0 ] && ok "imx8mp common does not read PVR_FW_BLOB" || bad "imx8mp without PVR_FW_BLOB: rc=$rc: $out"
mv "$L/pvr_fw_blob.h.off" "$L/pvr_fw_blob.h"
mv "$L/glmark2-src/src/default-benchmarks.h" "$L/gm.off"
out=$(deps_check rpi_4b common); rc=$?
[ "$rc" = 1 ] && check "pi4 common without GLMARK2_SRC fails and names it" "GLMARK2_SRC" "$out" || bad "pi4 without GLMARK2_SRC: rc=$rc: $out"
out=$(deps_check rpi_4b deqp); rc=$?
[ "$rc" = 0 ] && ok "pi4 deqp does not read GLMARK2_SRC" || bad "pi4 deqp without GLMARK2_SRC: rc=$rc: $out"
mv "$L/gm.off" "$L/glmark2-src/src/default-benchmarks.h"
rm -rf "$L/deqp-stage1-rpi_4b/build"
out=$(deps_check rpi_4b deqp); rc=$?
[ "$rc" = 1 ] && check "pi4 deqp without its stage 1 fails and names DEQP_WORK" "DEQP_WORK" "$out" || bad "pi4 deqp without stage 1: rc=$rc: $out"

# needed_deps against the builders: a board stage runs the image builders, and
# board-stage.sh stops only for what needed_deps lists. So every deps.toml
# entry a builder reads (${NAME or $NAME, comment lines left out) must be in
# some row of the table, or board-stage.sh would not stop for a missing one
# and the build would fail later.
# unlisted_deps <dir> <file...>: the deps.toml names those builders read that
# no board and set lists, as "NAME file" lines.
unlisted_deps() {
    local dir=$1 b name all_needed hits=
    shift
    for b in "$@"; do
        [ -e "$dir/$b" ] || hits="$hits builder-missing $b"$'\n'
    done
    all_needed=$(for lay in am62p imx8mp rpi_4b; do for what in common deqp ms4; do
        BOARD_STAGE_SOURCE_ONLY=1 bash -c ". '$HERE/board-stage.sh'; needed_deps $lay $what"
    done; done | sort -u)
    for name in $(sed -n 's/^name = "\(.*\)"/\1/p' "$HERE/deps.toml"); do
        for b in "$@"; do
            [ -e "$dir/$b" ] || continue
            if grep -v '^[[:space:]]*#' "$dir/$b" | grep -qE "[$]\{?$name\b" &&
                ! grep -qx "$name" <<< "$all_needed"; then
                hits="$hits$name $b"$'\n'
            fi
        done
    done
    printf '%s' "$hits"
}
builders="cts-common.sh common-images.sh zephyr-modules.sh am62p-deqp-images.sh pi4-deqp-images.sh cts-build-zephyr.sh imx8mp-deqp-build.sh imx8mp-deqp-stage1.sh imx8mp-env.sh imx8mp-gltest-build.sh imx8mp-gles3-board.sh imx8mp-gles3-artefacts.sh"
out=$(unlisted_deps "$HERE" $builders)
[ -z "$out" ] && ok "every deps.toml entry a builder reads is in a needed_deps row" \
    || bad "read by a builder, in no needed_deps row: $(paste -sd';' <<< "$out")"
# the guard itself, not only its input: a scratch builder that reads
# SLINT_NOSTD_DIR (a deps.toml entry in no row; the Slint demo image is not a
# board stage) must be named, as the real builders would be.
cp "$HERE/common-images.sh" "$R/tools/cts/broken-images.sh"
echo ': "${SLINT_NOSTD_DIR:-}"' >> "$R/tools/cts/broken-images.sh"
out=$(unlisted_deps "$R/tools/cts" broken-images.sh)
check "a builder reading an entry in no row is named" "SLINT_NOSTD_DIR broken-images.sh" "$out"

echo
if [ "$fail" = 0 ]; then echo "images-selftest: all checks passed"; else echo "images-selftest: $fail check(s) FAILED"; exit 1; fi

#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/images-selftest.sh
#
# Host-only checks of tools/cts/am62p-deqp-images.sh and imx8mp-gltest-build.sh,
# the defects the round-1 board sessions found in them. No board, no Zephyr, no
# network: the scripts run from a scratch copy of the repository layout with a
# fake `west` that writes a zephyr.bin and a .config (it fails for a filter that
# contains "FAILME", and writes no image for one that contains "NOIMAGE").
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
    "$R/driver/samples/zephyr/gc_gl_test" "$T/bin" "$T/ws/zephyr" \
    "$T/ws/modules/fs/fatfs" "$T/work/cts" "$T/work/build/modules/gles2" "$T/venv/bin"
cp "$HERE/am62p-deqp-images.sh" "$HERE/imx8mp-gltest-build.sh" "$HERE/imx8mp-env.sh" \
    "$HERE/zephyr-modules.sh" "$R/tools/cts/"
printf 'import sys\nsys.exit(0)\n' > "$R/tools/cts/check-deqp-images.py"
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
: > "$d/zephyr/.config"
EOF
chmod +x "$T/bin/west"
printf 'export PATH=%s:$PATH\n' "$T/bin" > "$T/venv/bin/activate"
S=$R/tools/cts/am62p-deqp-images.sh
# the caller's environment must not reach the scripts under test (the node sets WEST_VENV, a
# TREE_ROWS in the shell would select rows, ...)
unset WEST_VENV TREE_ROWS DEQP_MODULE ZEPHYR_BASE SAFEUI_DIR DEQP_GL_CONFIG_NAME BOARD
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
rm "$T/bin/west"
out=$(PATH="/usr/bin:/bin" "$G" n4 96 2>&1); rc=$?
[ "$rc" = 2 ] && ok "no west: rc 2" || bad "no west: rc=$rc: $out"
check "names west" "west not on PATH" "$out"

echo
if [ "$fail" = 0 ]; then echo "images-selftest: all checks passed"; else echo "images-selftest: $fail check(s) FAILED"; exit 1; fi

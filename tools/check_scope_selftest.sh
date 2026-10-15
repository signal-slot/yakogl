#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Pins tools/check.sh --changed's map from a changed path to the rows it
# selects (scope_of_path), and that a diff of nothing runs nothing. A path
# that lands in the wrong scope skips the rows that would have caught its
# break, so the map is pinned here and read in the gate (scope tools).
set -u
cd "$(dirname "$0")/.." || exit 2
# The plans below pin the default CONFIGS. The gate runs this selftest with
# its own environment, so a narrowed YAKOGL_CHECK_CONFIGS would otherwise
# change every plan here (the narrowed plan below sets it itself).
unset YAKOGL_CHECK_CONFIGS
# The same for MESA_V3D_TREE: with a built Mesa tree, check.sh plans three V3D
# Mesa rows instead of four (no "build v3d mesa reference"), so every row
# count below would be one less on a host that sets it.
unset MESA_V3D_TREE
fails=0
checks=0
want() {
  local path=$1 scope=$2 got
  checks=$((checks + 1))
  got=$(tools/check.sh --scope-of "$path" | cut -d' ' -f2-)
  if [ "$got" != "$scope" ]; then
    echo "FAIL $path: scope '$got', expected '$scope'"
    fails=$((fails + 1))
  fi
}
# shared layers and the unknown select every row
want src/gl/draw.rs all
want src/lib.rs all
want src/backend.h all
want src/osal/zephyr.c all
want src/shaderc/online/sema.rs all
want src/shaderc/online/mod.rs all
want capi/assemble.cmake all
want Cargo.toml all
want tools/check.sh all
want tools/land.sh all
want tests/es3_tf_api.rs all
want third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml all
want some/new/place.rs all
# one GPU
want src/pvr/draw.rs pvr
want src/shaderc/online/powervr/sched.rs pvr
want driver/src/pvr_vdm_ppp_gen.c pvr
want driver/src/fw/pvr_fw_blob.h pvr
want tests/reference/blobs.rs pvr
want src/vivante/backend.rs vivante
want src/shaderc/online/vivante/isa.rs vivante
want tools/hal-nxp/gen_regs.py vivante
want src/v3d/job.rs v3d
want src/shaderc/online/v3d/prepare.rs v3d
want src/shaderc/online/v3d_abi.rs v3d
want tools/v3d/job_check.c v3d
want tests/v3d_cube_noseamless.rs v3d
# the rest
want driver/tools/pvrlab/gl_test.c gltest
want tools/precompile/prove.sh precompile
want tools/shaders/precompiled.list precompile
want tests/fixtures/precompile_counts.txt precompile
want tools/yakogl-tools/src/main.rs toolscrate
want tools/osal/linux_heap_check.c osal
want driver/samples/zephyr/gl_test/src/main.c samples
want driver/rust/src/lib.rs drivers
want driver/CMakeLists.txt drivers
want tools/cts/deqp-uart-tally.py tools
want tools/board.sh tools
want tools/hostharness/glmark2/run.sh tools
want tools/testdata/pi4-tryboot-F1rerun2-a27d69c0.console.txt tools
want docs/HANDOFF.md docs
want website/index.html docs
want README.md docs
want tools/cts/log/board-sessions/x.md docs
# the plan for a given change (YAKOGL_CHECK_PATHS stands in for the diff)
# A stand-in west workspace, so the image rows are planned rather than gaps
# (plan mode runs nothing, so the directory only has to exist).
# The same for the glmark2 tree (the corpus row needs GLMARK2_SRC/src).
# MESA_V3D_TREE is emptied: set, it replaces the v3d Mesa reference build row
# with nothing, so the plan's row count would depend on the host.
mkdir -p build/check/plan-ws/zephyr build/check/plan-gm2/src
plan() {
  ZEPHYR_WS=$PWD/build/check/plan-ws GLMARK2_SRC=$PWD/build/check/plan-gm2 MESA_V3D_TREE= \
    YAKOGL_CHECK_PLAN=1 YAKOGL_CHECK_PATHS="$1" tools/check.sh --changed HEAD 2>&1 |
    sed 's/\x1b\[[0-9;]*m//g'
}
expect() {
  local label=$1 pattern=$2 out=$3
  checks=$((checks + 1))
  printf '%s\n' "$out" | grep -qE "$pattern" || { echo "FAIL $label: no line matches '$pattern'"; fails=$((fails + 1)); }
}
out=$(plan docs/HANDOFF.md)
expect "docs: only the shared rows" '^check.sh: 2 ok, 0 failed, 0 gap\(s\).*, 149 skipped \(--changed HEAD, scopes: docs' "$out"
out=$(plan src/v3d/job.rs)
expect "v3d: its clippy row" '^plan  clippy  host-test,gpu-v3d,shaderc-online,no-shader-table$' "$out"
expect "v3d: its image" '^plan  zephyr  rpi4 ' "$out"
expect "v3d: the Mesa reference" '^plan  run     v3d job_check' "$out"
expect "v3d: no PowerVR row" '^skip  clippy  host-test,gpu-powervr$' "$out"
expect "v3d: no AM62P image" '^skip  zephyr  am62p gl_test image, -Werror' "$out"
expect "v3d: no precompiled tables" '^skip  gen     precompiled table gpu-powervr:' "$out"
# A harness script runs in the row that calls it: tools/hostharness/glmark2/run.sh
# is run by the glmark2 corpus row (a change there once passed a gate that
# skipped the only row able to fail on it).
out=$(plan tools/hostharness/glmark2/run.sh)
expect "hostharness: the glmark2 corpus row" '^plan  test    glmark2 corpus generated from GLMARK2_SRC' "$out"
expect "hostharness: not every row" '^check.sh: .*\(--changed HEAD, scopes: tools \)' "$out"
out=$(plan driver/tools/pvrlab/gl_test.c)
expect "gl_test: the three images" '^plan  zephyr  imx8mp ' "$out"
expect "gl_test: its cc arms" '^plan  cc      gl_test.c ' "$out"
expect "gl_test: the shader header" '^plan  gen     gl_test_shaders' "$out"
expect "gl_test: no host suite" '^skip  test    host-test,gpu-powervr$' "$out"
out=$(plan "src/gl/draw.rs docs/x.md")
expect "a shared layer: every row" '^check.sh: 1[0-9][0-9] ok, 0 failed, [0-9] gap\(s\).*, 0 skipped \(--changed HEAD, scopes: all\)' "$out"
expect "a shared layer: the path is named" '^      every row: src/gl/draw.rs$' "$out"
# The tier tables are made exactly when a selected row reads them (#127):
# with CONFIGS narrowed to a set without `precompiled-shaders`, the
# no-compiler rows (not CONFIGS rows) still run, so the table row is planned,
# once and before them.
line_of() { printf '%s\n' "$2" | grep -nxF -m1 "$1" | cut -d: -f1; }
order() {
  local label=$1 first=$2 second=$3 out=$4 a b
  checks=$((checks + 1))
  a=$(line_of "$first" "$out")
  b=$(line_of "$second" "$out")
  if [ -z "$a" ] || [ -z "$b" ] || [ "$a" -ge "$b" ]; then
    echo "FAIL $label: '$first' (line ${a:-absent}) is not before '$second' (line ${b:-absent})"
    fails=$((fails + 1))
  fi
}
count() {
  local label=$1 pattern=$2 want=$3 out=$4 got
  checks=$((checks + 1))
  got=$(printf '%s\n' "$out" | grep -cE "$pattern")
  [ "$got" -eq "$want" ] || { echo "FAIL $label: $got line(s) match '$pattern', expected $want"; fails=$((fails + 1)); }
}
gen='plan  gen     precompiled tables for the tier configurations'
nocomp='plan  clippy  host-test,gpu-powervr,precompiled-shaders,no-shader-table (no compiler; lib and its tests)'
out=$(YAKOGL_CHECK_CONFIGS="host-test,gpu-powervr" plan src/pvr/draw.rs)
expect "narrowed CONFIGS: the no-compiler rows" "^${nocomp//[()]/.}\$" "$out"
order "narrowed CONFIGS: the tables before their first reader" "$gen" "$nocomp" "$out"
count "narrowed CONFIGS: the table row once" "^plan  gen     precompiled tables for the tier" 1 "$out"
# the control: the full plan keeps its rows, the table row once, before the
# first CONFIGS row that reads the tables
out=$(plan src/gl/draw.rs)
# 152 rows: ok plus gaps, because a row whose tool this host lacks is a gap
# here and a plan row on the gate host; the sum is the same on both.
checks=$((checks + 1))
line=$(printf '%s\n' "$out" | grep -E '^check.sh: [0-9]+ ok, 0 failed, [0-9]+ gap\(s\).*, 0 skipped \(--changed HEAD, scopes: all\)')
total=$(printf '%s\n' "$line" | sed -E 's/^check.sh: ([0-9]+) ok, 0 failed, ([0-9]+) gap.*/\1 \2/' | awk '{ print $1 + $2 }')
[ "$total" = 152 ] || { echo "FAIL full plan: its row count: ${total:-no summary line} rows (ok + gaps), expected 152:$line"; fails=$((fails + 1)); }
count "full plan: the table row once" "^plan  gen     precompiled tables for the tier" 1 "$out"
order "full plan: the tables before their first reader" "$gen" \
  'plan  clippy  host-test,gpu-powervr,shaderc-online,precompiled-shaders' "$out"
# no reader selected, no table row (a docs change)
out=$(plan docs/HANDOFF.md)
count "docs: no table row" "precompiled tables for the tier" 0 "$out"
# --local: a shared-layer change runs the local set only
local_plan() {
  ZEPHYR_WS=$PWD/build/check/plan-ws YAKOGL_CHECK_PLAN=1 YAKOGL_CHECK_PATHS="$1" tools/check.sh --local HEAD 2>&1 |
    sed 's/\x1b\[[0-9;]*m//g'
}
out=$(local_plan src/gl/draw.rs)
expect "local: fmt" '^plan  cargo fmt --check$' "$out"
expect "local: the PowerVR compiler suite" '^plan  test    host-test,gpu-powervr,shaderc-online$' "$out"
expect "local: the GC7000 compiler suite" '^plan  test    host-test,gpu-vivante,shaderc-online$' "$out"
expect "local: the V3D suite" '^plan  test    host-test,gpu-v3d,shaderc-online,no-shader-table \(--lib' "$out"
expect "local: the gl_test arms" '^plan  cc      gl_test.c ' "$out"
expect "local: no report-es2 pair" '^skip  test    host-test,gpu-powervr,shaderc-online,report-es2$' "$out"
expect "local: no image" '^skip  zephyr  am62p gl_test image, -Werror' "$out"
expect "local: no table proof" '^skip  gen     precompiled table gpu-powervr:' "$out"
expect "local: the summary says so" '^check.sh: (9|1[0-9]) ok, 0 failed, 0 gap\(s\).*skipped \(--local HEAD, scopes: all\)' "$out"
out=$(local_plan src/v3d/job.rs)
expect "local on a V3D change: the whole V3D scope" '^plan  zephyr  rpi4 ' "$out"
echo "check_scope_selftest: $checks checks run (planned $checks), $fails failed"
[ "$fails" -eq 0 ]

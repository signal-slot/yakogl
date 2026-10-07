#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
#
# measure-image.sh [app]: what the precompiled tier costs a real image.
#
#   tools/precompile/measure-image.sh gl_test_es100
#
# Builds the AM62P gl_test image (verdin_am62p/am62p54/a53, aarch64-unknown-none,
# release) three ways and prints, for each, the build seconds, zephyr.bin bytes
# and the `size -A` split of .text, .rodata and .bss:
#
#   off    the tier off: the in-process compiler is the back end (the control)
#   app    the tier on, this app's rows in front of the compiler
#   no-cc  the tier on, this app's rows and no compiler at all
#
# Those three are the comparison step 6 of docs/HANDOFF.md (open work item 1)
# turns on: what a compiler that serves any shader costs -- its .text and
# .rodata in flash, its .bss in RAM -- against a table that serves only the apps
# it names and grows with them. An image with the tier on and no app named is a
# configure error (CMakeLists.txt), so the whole list is not an arm here: it is
# what that guard refuses.
#
# Environment: PVR_FW_BLOB (the generated pvr_fw_blob.h the PowerVR loader
# needs), GLSL2USC_CACHE, ZEPHYR_WS (default: the workspace this repository is
# built in), BOARD and SAMPLE (the defaults above). Output goes to stdout; each
# build's log is build/measure-<arm>.log, which the script creates. It exits
# non-zero when an arm failed to build: a failed arm prints no numbers, so a
# run that printed none is not a measurement.
set -u
APP=${1:-gl_test_es100}
cd "$(dirname "$0")/../.." || exit 9
ROOT=$PWD
mkdir -p "$ROOT/build" || exit 9
WS=${ZEPHYR_WS:-/home/tasuku/com/github/signal-slot/zephyr-ws-yakogl}
BOARD=${BOARD:-verdin_am62p/am62p54/a53}
SAMPLE=${SAMPLE:-gl_test}
[ -r "${PVR_FW_BLOB:-}" ] || { echo "measure-image.sh: PVR_FW_BLOB is not readable (driver/src/fw/pvr_fw_blob.h)"; exit 9; }
[ -d "$WS/.venv" ] || { echo "measure-image.sh: no Zephyr workspace at $WS"; exit 9; }
mods=$(PATH="$WS/.venv/bin:$PATH" tools/cts/zephyr-modules.sh "$WS") || exit 1

run() {
  local name=$1; shift
  local t0 log elf
  t0=$(date +%s)
  log=build/measure-$name.log
  elf=build/measure-$name/zephyr/zephyr.elf
  if ! (cd "$WS" && PATH="$WS/.venv/bin:$PATH" ZEPHYR_BASE="$WS/zephyr" \
      west build -p always -b "$BOARD" -d "$ROOT/build/measure-$name" \
      "$ROOT/driver/samples/zephyr/$SAMPLE" -- \
      "-DZEPHYR_MODULES=$mods" "-DYAKOGL_DIR=$ROOT" -DYAKOGL_WERROR=ON "$@") \
      >"$log" 2>&1 </dev/null; then
    echo "$name: BUILD FAILED"; grep -E 'error:|Error ' "$log" | head -15; return 1
  fi
  echo "$name: $(( $(date +%s) - t0 )) s  zephyr.bin $(stat -c %s build/measure-$name/zephyr/zephyr.bin) bytes"
  grep -E "yakogl-precompile: [0-9]+ rows" "$log" | head -1
  size -A "$elf" | grep -E '^\.(text|rodata|bss)\b|^Total' | awk -v n="$name" '{printf "  %-6s %-10s %s\n", n, $2, $1}'
}

fail=0
run off   -DCONFIG_YAKOGL_PRECOMPILED_SHADERS=n || fail=1
run app   "-DCONFIG_YAKOGL_PRECOMPILED_APPS=\"$APP\"" || fail=1
run no-cc -DCONFIG_YAKOGL_SHADERC_PRECOMPILED_ONLY=y "-DCONFIG_YAKOGL_PRECOMPILED_APPS=\"$APP\"" || fail=1
# An arm that did not build prints no numbers, so the exit code is what says
# whether the run measured anything at all.
exit $fail

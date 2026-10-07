#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
#
# measure-image.sh [app]: what the precompiled tier costs a real image.
#
#   tools/precompile/measure-image.sh gl_test_es100
#
# Builds the AM62P gl_test image (verdin_am62p/am62p54/a53, aarch64-unknown-none,
# release) three ways and prints, for each, the build seconds, zephyr.bin bytes
# and the `size -A` split (text, rodata, datas, bss, noinit, Total):
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
# needs; it is copied into this checkout when the checkout does not already
# have it -- driver/src/fw/ is gitignored, so a fresh one never does, and it is
# never committed), GLSL2USC_CACHE, ZEPHYR_WS (default: the workspace this
# repository is built in), BOARD and SAMPLE (the defaults above). Output goes to
# stdout; each build's log is build/measure-<arm>.log, which the script creates.
# It exits non-zero when an arm failed to build or built an image that is not
# the arm: a failed arm prints no numbers, and an arm whose built zephyr/.config
# does not hold the CONFIG_* switches it passed measured something else. So a
# run that printed no numbers, or printed an arm's numbers with no complaint, is
# a measurement; anything else is not.
set -u
APP=${1:-gl_test_es100}
cd "$(dirname "$0")/../.." || exit 9
ROOT=$PWD
mkdir -p "$ROOT/build" || exit 9
WS=${ZEPHYR_WS:-/home/tasuku/com/github/signal-slot/zephyr-ws-yakogl}
BOARD=${BOARD:-verdin_am62p/am62p54/a53}
SAMPLE=${SAMPLE:-gl_test}
BLOB=${PVR_FW_BLOB:-$ROOT/driver/src/fw/pvr_fw_blob.h}
[ -r "$BLOB" ] || { echo "measure-image.sh: no readable pvr_fw_blob.h (PVR_FW_BLOB, or $BLOB)"; exit 9; }
if ! cmp -s "$BLOB" "$ROOT/driver/src/fw/pvr_fw_blob.h"; then
  mkdir -p "$ROOT/driver/src/fw" && cp "$BLOB" "$ROOT/driver/src/fw/pvr_fw_blob.h" || exit 9
fi
[ -d "$WS/.venv" ] || { echo "measure-image.sh: no Zephyr workspace at $WS"; exit 9; }
mods=$(PATH="$WS/.venv/bin:$PATH" tools/cts/zephyr-modules.sh "$WS") || exit 1

# check_switches <arm> <.config> <cmake args...>: every CONFIG_* switch the arm
# passed is a switch the built image has. A Kconfig symbol whose dependency is
# not met is dropped with a warning -- CONFIG_YAKOGL_PRECOMPILED_APPS depends on
# CONFIG_YAKOGL_PRECOMPILED_SHADERS -- so an arm that lost one built a different
# image, and calling its numbers a measurement of this arm would be wrong.
check_switches() {
  local name=$1 conf=$2; shift 2
  local arg sym want got bad=0
  for arg in "$@"; do
    case $arg in -DCONFIG_*=*) ;; *) continue ;; esac
    arg=${arg#-D}; sym=${arg%%=*}; want=${arg#*=}
    got=$(grep -m1 -E "^$sym=" "$conf" | cut -d= -f2-)
    got=${got#\"}; got=${got%\"}
    [ -n "$got" ] || got=n          # a disabled bool reads `# CONFIG_X is not set`
    if [ "$got" != "$want" ]; then
      echo "  $name: $sym is '$got' in ${conf#"$ROOT/"}, but the arm asked for '$want'"
      bad=1
    fi
  done
  return $bad
}

run() {
  local name=$1; shift
  local args=("$@") t0 log elf conf
  t0=$(date +%s)
  log=build/measure-$name.log
  elf=build/measure-$name/zephyr/zephyr.elf
  conf=build/measure-$name/zephyr/.config
  if ! (cd "$WS" && PATH="$WS/.venv/bin:$PATH" ZEPHYR_BASE="$WS/zephyr" \
      west build -p always -b "$BOARD" -d "$ROOT/build/measure-$name" \
      "$ROOT/driver/samples/zephyr/$SAMPLE" -- \
      "-DZEPHYR_MODULES=$mods" "-DYAKOGL_DIR=$ROOT" -DYAKOGL_WERROR=ON "${args[@]}") \
      >"$log" 2>&1 </dev/null; then
    echo "$name: BUILD FAILED"; grep -E 'error:|Error ' "$log" | head -15; return 1
  fi
  check_switches "$name" "$ROOT/$conf" "${args[@]}" || return 1
  echo "$name: $(( $(date +%s) - t0 )) s  zephyr.bin $(stat -c %s build/measure-$name/zephyr/zephyr.bin) bytes"
  grep -E "yakogl-precompile: [0-9]+ rows" "$log" | head -1
  # `size -A` prints the section names with no leading dot.
  size -A "$elf" | awk -v n="$name" \
    '$1 ~ /^(text|rodata|data|datas|bss|noinit|Total)$/ {printf "  %-6s %-8s %s\n", n, $1, $2}'
}

fail=0
run off   -DCONFIG_YAKOGL_PRECOMPILED_SHADERS=n || fail=1
run app   -DCONFIG_YAKOGL_PRECOMPILED_SHADERS=y "-DCONFIG_YAKOGL_PRECOMPILED_APPS=\"$APP\"" || fail=1
run no-cc -DCONFIG_YAKOGL_SHADERC_PRECOMPILED_ONLY=y -DCONFIG_YAKOGL_PRECOMPILED_SHADERS=y "-DCONFIG_YAKOGL_PRECOMPILED_APPS=\"$APP\"" || fail=1
# An arm that did not build prints no numbers, so the exit code is what says
# whether the run measured anything at all.
exit $fail

#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Pins setup-gate-host.sh --check's lines for the Pi 5 rows' inputs: aarch64
# gcc, dtc, V3D_SHC and ZEPHYR_SDK_INSTALL_DIR, present and missing, with a
# fake PATH and environment. Nothing is installed; only --check runs.
set -u
# The host's locale must not reach the checked children: with LC_ALL naming
# a locale the host has not installed, every bash child prints a setlocale
# warning into the output the checks compare (the same family as the glmark2
# selftests' LC_ALL=C). The locale checks below name their own LC_ALL, so
# they are unaffected.
export LC_ALL=C LANG=C
cd "$(dirname "$0")/.." || exit 2
t=$(mktemp -d) || exit 2
trap 'rm -rf "$t"' EXIT
fails=0
checks=0
real=$(command -v bash)
mkdir -p "$t/full" "$t/hide" "$t/sdk/aarch64-zephyr-elf/bin" "$t/sdk0"
for n in aarch64-linux-gnu-gcc dtc; do
  printf '#!/bin/sh\necho "%s fake 1.0"\n' "$n" >"$t/full/$n"
  # a failing stub hides a host tool of the same name
  printf '#!/bin/sh\nexit 127\n' >"$t/hide/$n"
  chmod +x "$t/full/$n" "$t/hide/$n"
done
printf '#!/bin/sh\nexit 0\n' >"$t/sdk/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-gcc"
printf '#!/bin/sh\nexit 0\n' >"$t/shc"
chmod +x "$t/sdk/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-gcc" "$t/shc"
run() { # <pathdir> [VAR=val...]
  local d=$1
  shift
  env -u ZEPHYR_SDK_INSTALL_DIR -u V3D_SHC -u MESA_V3D_TREE -u MESA_ETNA_TREE PATH="$d:$PATH" "$@" \
    "$real" tools/setup-gate-host.sh --check 2>&1
}
want() { # <desc> <regex> <output>
  checks=$((checks + 1))
  if ! printf '%s\n' "$3" | grep -Eq "$2"; then
    echo "FAIL $1: no line matching $2"
    fails=$((fails + 1))
  fi
}
ok=$(run "$t/full" ZEPHYR_SDK_INSTALL_DIR="$t/sdk" V3D_SHC="$t/shc")
want "gcc present" '^ok +aarch64-linux-gnu-gcc ' "$ok"
want "dtc present" '^ok +dtc ' "$ok"
want "sdk present" '^ok +ZEPHYR_SDK_INSTALL_DIR ' "$ok"
want "shc present" '^ok +V3D_SHC ' "$ok"
mi=$(run "$t/hide")
want "gcc missing names package" '^warn +aarch64-linux-gnu-gcc +missing: .*.*gcc-aarch64-linux-gnu' "$mi"
want "dtc missing names package" '^missing +dtc .*device-tree-compiler' "$mi"
want "sdk unset" '^warn +ZEPHYR_SDK_INSTALL_DIR +missing: .*not set' "$mi"
want "shc missing" '^warn +V3D_SHC +missing: .*tools/v3d/README.md' "$mi"
mi2=$(run "$t/hide" ZEPHYR_SDK_INSTALL_DIR="$t/sdk0" V3D_SHC="$t/nonexistent")
want "sdk without gcc" '^warn +ZEPHYR_SDK_INSTALL_DIR +missing: .*no aarch64-zephyr-elf-gcc' "$mi2"
want "shc not executable" '^warn +V3D_SHC +missing: ' "$mi2"
mkdir -p "$t/tree/build-v3d/src/broadcom"
cp "$t/shc" "$t/tree/build-v3d/src/broadcom/v3d_shc"
tr=$(run "$t/hide" MESA_V3D_TREE="$t/tree")
want "shc via MESA_V3D_TREE" '^ok +V3D_SHC .*v3d_shc' "$tr"
e0=$(run "$t/hide")
want "etna unset is a gap" '^warn +MESA_ETNA_TREE +missing: unset: .*mesa-etna-build.sh' "$e0"
checks=$((checks + 1))
if printf '%s\n' "$e0" | grep -Eq '^ok +MESA_ETNA_TREE'; then echo "FAIL etna unset reported ok"; fails=$((fails + 1)); fi
mkdir -p "$t/etna/mesa-26.2.1"
e1=$(run "$t/hide" MESA_ETNA_TREE="$t/etna/mesa-26.2.1")
want "etna incomplete is a gap" '^warn +MESA_ETNA_TREE +missing: .*not a complete tree.*gap: MESA_ETNA_TREE' "$e1"
checks=$((checks + 1))
if printf '%s\n' "$e1" | grep -Eq '^ok +MESA_ETNA_TREE'; then echo "FAIL etna incomplete reported ok"; fails=$((fails + 1)); fi
# an uninstalled locale in LC_ALL, else LC_CTYPE, else LANG is a missing
# line naming the variable and the fix; C, C.* and an installed name are
# silent. A fake locale command on the PATH makes the installed list the
# same on every host.
mkdir -p "$t/loc"
printf '#!/bin/sh\nprintf "C\\nC.utf8\\nPOSIX\\nen_US.utf8\\n"\n' >"$t/loc/locale"
chmod +x "$t/loc/locale"
lc0=$(run "$t/loc" LC_ALL=C)
checks=$((checks + 1))
if printf '%s\n' "$lc0" | grep -Eq '^[a-z]+ +locale '; then echo "FAIL LC_ALL=C reported a locale line"; fails=$((fails + 1)); fi
lc1=$(run "$t/loc" LC_ALL=C.UTF-8)
checks=$((checks + 1))
if printf '%s\n' "$lc1" | grep -Eq '^[a-z]+ +locale '; then echo "FAIL LC_ALL=C.UTF-8 reported a locale line"; fails=$((fails + 1)); fi
lc2=$(run "$t/loc" LC_ALL=xx_YY.UTF-8)
want "uninstalled LC_ALL" '^missing +locale +LC_ALL=xx_YY.UTF-8 .*export LC_ALL=C.*install the locale' "$lc2"
lc3=$(run "$t/loc" LC_ALL=EN_us.UTF-8)
want "installed locale, case and utf8 spelling" '^ok +locale +LC_ALL=EN_us.UTF-8' "$lc3"
lc4=$(run "$t/loc" LC_ALL= LC_CTYPE= LANG=xx_YY.UTF-8)
want "uninstalled LANG" '^missing +locale +LANG=xx_YY.UTF-8 ' "$lc4"
lc5=$(run "$t/loc" LC_ALL= LC_CTYPE=xx_YY.UTF-8 LANG=en_US.utf8)
want "LC_CTYPE before LANG" '^missing +locale +LC_CTYPE=xx_YY.UTF-8 ' "$lc5"
# free-disk threshold: the four cases (docs/ci.md 3.8)
dk0=$(run "$t/full" YAKOGL_CHECK_PRUNE=)
want "disk, no prune: 220 GB" '^[a-z]+ +disk +.*(need|needs about) 220 GB without pruning' "$dk0"
dk1=$(run "$t/full" YAKOGL_CHECK_PRUNE=1)
want "disk, prune: pruned default 50 GB" '^[a-z]+ +disk +.*(need|needs about) 50 GB with YAKOGL_CHECK_PRUNE=1' "$dk1"
dk2=$(run "$t/full" YAKOGL_CHECK_PRUNE=1 YAKOGL_GATE_MIN_FREE_GB=99999999)
want "disk, explicit overrides the pruned default, missing" '^missing +disk +.*99999999 GB with YAKOGL_CHECK_PRUNE=1' "$dk2"
dk2b=$(run "$t/full" YAKOGL_GATE_MIN_FREE_GB=99999999)
want "disk, explicit without prune, missing" '^missing +disk +.*99999999 GB without pruning' "$dk2b"
dk3=$(run "$t/full" YAKOGL_CHECK_PRUNE=1 GATE_FULL_RUN=1 YAKOGL_GATE_MIN_FREE_GB_PRUNED=99999999)
want "disk, GATE_FULL_RUN=1 is missing" '^missing +disk +.*99999999 GB with YAKOGL_CHECK_PRUNE=1' "$dk3"
dk4=$(run "$t/full" YAKOGL_CHECK_PRUNE=1 YAKOGL_GATE_MIN_FREE_GB_PRUNED=99999999)
want "disk, pruned default alone stays warn" '^warn +disk +.*99999999 GB with YAKOGL_CHECK_PRUNE=1' "$dk4"
echo "setup-gate-host_selftest: $((checks - fails))/$checks ok"
[ "$fails" -eq 0 ]

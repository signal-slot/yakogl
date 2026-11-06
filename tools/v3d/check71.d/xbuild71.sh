#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# xbuild71: the driver/src/v3d C files the Zephyr build compiles cross-compile
# for aarch64 without a warning, in both board configurations the V3D
# generations ride on.
#
# The file list, the include directories and the defines are the Zephyr
# module's own (zephyr/CMakeLists.txt, CONFIG_YAKOGL_GPU_V3D branch):
# v3d_job.c, v3d_tex.c, v3d_tiling_mesa.c, v3d_frame.c and v3d_hw.c
# (v3d_tiling_uif.c arrives inside v3d_tex.c), compiled with YAKOGL_OSAL_ZEPHYR
# and YAKOGL_GPU_V3D over Mesa's vendored headers and driver/src/v3d/
# generated/. -std=c11 as the gate's v3d C rows compile them (tools/check.sh),
# -Os (Zephyr's level), -Wall -Wextra -Werror, -c only: objects land in
# build/v3d71/xbuild71/ and nothing links. v3d_hw.c's <zephyr/...> includes
# come from tools/v3d/hostshim, whose mm_reg_t is uintptr_t as arm64 Zephyr's
# is, the same shim tools/v3d/check71.d/hw_mock.sh builds with; the Zephyr
# image rows compile the same file against the real headers.
#
# hw71 made the SoC a runtime parameter (struct v3d_regs.soc), not a compile
# one, so the two configurations are the two boards' toolchain choices:
# -mcpu=cortex-a72, what the 4.2 board's build uses (Zephyr soc/brcm/bcm2711
# selects CPU_CORTEX_A72 and cmake/gcc-m-cpu.cmake turns it into that flag),
# and -mcpu=cortex-a76, the 7.1 board's (soc/brcm/bcm2712 selects
# CPU_CORTEX_A76): the BCM2712 core the Pi 5 image runs on. The pass proves
# the 7.1 code of v3d_hw.c and v3d_regs.h compiles warning-free for the Pi 5's
# core, with the same defines and include set its build will pass it, and that
# the Pi 4 configuration stays as clean as it is.
#
# Compiler: aarch64-zephyr-elf-gcc from ZEPHYR_SDK_INSTALL_DIR (the SDK's
# newlib is its C library), else clang --target=aarch64-none-elf if it can
# find a C library for that target (plain, or with a /usr/aarch64-linux-gnu
# cross sysroot), else aarch64-linux-gnu-gcc. A compiler is used only when a
# TU including the headers these files read directly or through Mesa's util
# headers (<unistd.h>, <string.h>, <stdio.h>, ...) compiles under -std=c11
# -Os -Wall -Wextra -Werror. None of it: the last line is
# "gap: ZEPHYR_SDK_INSTALL_DIR ..." and the exit is 77 (a host without the
# cross toolchain is not a verdict on the code); any other non-zero exit is a
# failure, and a file that does not compile prints the compiler's lines.
# Convention: tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=xbuild71
out=build/v3d71/$name
mkdir -p "$out" || exit 1

# zephyr/CMakeLists.txt, _yakogl_gpu_c of the CONFIG_YAKOGL_GPU_V3D branch
FILES=(
  driver/src/v3d/v3d_job.c
  driver/src/v3d/v3d_tex.c
  driver/src/v3d/v3d_tiling_mesa.c
  driver/src/v3d/v3d_frame.c
  driver/src/v3d/v3d_hw.c
)
for f in "${FILES[@]}" tools/v3d/hostshim/zephyr/kernel.h \
         tools/v3d/hostshim/zephyr/sys/sys_io.h tools/v3d/hostshim/zephyr/logging/log.h; do
  [ -f "$f" ] || { echo "$name: $f missing (run from the repository root), 1 failed"; exit 1; }
done

# zephyr_library_include_directories and the CONFIG_YAKOGL_GPU_V3D additions
INC=(-Isrc -Isrc/osal -Isrc/vivante -Isrc/v3d
  -Idriver/src -Idriver/src/v3d -Idriver/src/v3d/generated
  -Ithird_party/mesa-26.2.1/src/broadcom -Ithird_party/mesa-26.2.1/src
  -Ithird_party/mesa-26.2.1/include)
# zephyr_library_compile_definitions(YAKOGL_OSAL_ZEPHYR YAKOGL_GPU_V3D)
DEFS=(-DYAKOGL_OSAL_ZEPHYR -DYAKOGL_GPU_V3D)
STD=(-std=c11 -Os -Wall -Wextra -Werror)

# ---- the compiler: the first candidate that compiles the header probe ----
cat > "$out/probe.c" <<'EOF'
#include <assert.h>
#include <errno.h>
#include <inttypes.h>
#include <limits.h>
#include <math.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
int xbuild71_probe(void);
int xbuild71_probe(void) { return 0; }
EOF

candidates=()
if [ -n "${ZEPHYR_SDK_INSTALL_DIR:-}" ]; then
  for gcc in "$ZEPHYR_SDK_INSTALL_DIR/gnu/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-gcc" \
             "$ZEPHYR_SDK_INSTALL_DIR/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-gcc"; do
    [ -x "$gcc" ] && candidates+=("$gcc")
  done
fi
if command -v clang >/dev/null 2>&1; then
  candidates+=("clang --target=aarch64-none-elf")
  [ -d /usr/aarch64-linux-gnu ] && candidates+=("clang --target=aarch64-none-elf --sysroot=/usr/aarch64-linux-gnu")
fi
if command -v aarch64-linux-gnu-gcc >/dev/null 2>&1; then
  candidates+=("aarch64-linux-gnu-gcc")
fi

CC=()
for c in "${candidates[@]}"; do
  read -r -a cand <<<"$c"   # no candidate argument contains a space
  if "${cand[@]}" "${STD[@]}" -c "$out/probe.c" -o "$out/probe.o" 2>"$out/probe.err"; then
    CC=("${cand[@]}")
    break
  fi
done
if [ ${#CC[@]} -eq 0 ]; then
  echo "gap: ZEPHYR_SDK_INSTALL_DIR (no aarch64-zephyr-elf-gcc in it, no clang --target=aarch64-none-elf that finds a C library for the target, no aarch64-linux-gnu-gcc)"
  exit 77
fi
rm -f "$out/probe.o" "$out/probe.err"

echo "$name: ${CC[*]}"
failed=0
for cfg in 42 71; do
  case $cfg in
  42) mcpu=cortex-a72 ;; # BCM2711, the V3D 4.2 board (Pi 4)
  71) mcpu=cortex-a76 ;; # BCM2712, the V3D 7.1 board (Pi 5)
  esac
  for f in "${FILES[@]}"; do
    b=$(basename "$f" .c)
    extra=()
    [ "$f" = driver/src/v3d/v3d_hw.c ] && extra=(-Itools/v3d/hostshim)
    if "${CC[@]}" "${STD[@]}" -mcpu="$mcpu" "${INC[@]}" "${DEFS[@]}" "${extra[@]}" \
      -c "$f" -o "$out/$b.$cfg.o" 2>"$out/$b.$cfg.err"; then
      rm -f "$out/$b.$cfg.err"
      echo "ok      $b  $cfg (-mcpu=$mcpu)"
    else
      echo "FAIL    $b  $cfg (-mcpu=$mcpu):"
      tail -n 20 "$out/$b.$cfg.err" | sed 's/^/    /'
      failed=$((failed + 1))
    fi
  done
done

echo "$name: ${#FILES[@]} files x 2 configurations, $failed failed"
[ "$failed" -eq 0 ] || exit 1
exit 0

#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Stage 1 of dEQP-GLES2 on Zephyr: build dEQP's libraries with the Zephyr SDK
# compiler, for driver/samples/zephyr/deqp to link.
#
#   ZEPHYR_BASE=... ZEPHYR_MODULES="$(tools/cts/zephyr-modules.sh <workspace>)" \
#       tools/cts/cts-build-zephyr.sh
#   then: west build -b <board> driver/samples/zephyr/deqp -- \
#       -DZEPHYR_MODULES="$ZEPHYR_MODULES" \
#       -DDEQP_SRC=$WORK/cts -DDEQP_BUILD=$WORK/build
#
# The user's CTS checkout is never written to: a --shared clone of it at its
# HEAD goes to $WORK/cts, its fetched external sources are symlinked in, and
# cts-yakogl-target.patch and cts-yakogl-zephyr.patch are applied there.
#
# The compile flags are the ones a Zephyr application of this board gets: the
# deqp sample is configured (CMake only) and one of its compile commands is
# taken apart. That is what makes delibs' pthread/dirent calls match the
# types Zephyr's POSIX layer really has. Two of its flags are deliberately
# left out or overridden:
#  - -imacros zephyr_stdint.h: it makes int64_t `long long`, the SDK's
#    libstdc++.a was built with `long`, and every libstdc++ call with a
#    64-bit type then mislinks (the sample sets CONFIG_ENFORCE_ZEPHYR_STDINT=n);
#  - DE_CPU is set to DE_CPU_ARM_64: CMake detects x86_64 for a Generic system.
#
# Environment:
#   CTS        VK-GL-CTS checkout (default ~/com/github/KhronosGroup/VK-GL-CTS)
#   WORK       output directory (default <repo>/build/deqp-zephyr)
#   BOARD      Zephyr board (default verdin_am62p/am62p54/a53)
#   ZEPHYR_BASE, ZEPHYR_MODULES and/or ZEPHYR_EXTRA_MODULES   as for any sample of
#              this repository (at least one of the two module variables)
#   ZEPHYR_SDK_INSTALL_DIR   the Zephyr SDK (default <deps>/zephyr-sdk-1.0.1, where <deps> is
#              $CCFARM_DEPS_DIR or ~/.local/share/ccfarm/deps/yakogl, as in deps-env.sh)
#   MODULE     gles2 (default) or gles3: which dEQP package the libraries are
#              for. gles3 also turns on the target's GLES3 direct link, so
#              every ES 3.0 entry point dEQP's table names must resolve at the
#              image's link. Give each module its own WORK.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
CTS=${CTS:-$HOME/com/github/KhronosGroup/VK-GL-CTS}
MODULE=${MODULE:-gles2}
case "$MODULE" in gles2|gles3) ;; *) echo "MODULE must be gles2 or gles3, not '$MODULE'" >&2; exit 1 ;; esac
WORK=${WORK:-$REPO/build/deqp-zephyr}
BOARD=${BOARD:-verdin_am62p/am62p54/a53}
SDK=${ZEPHYR_SDK_INSTALL_DIR:-${CCFARM_DEPS_DIR:-$HOME/.local/share/ccfarm/deps/yakogl}/zephyr-sdk-1.0.1}
: "${ZEPHYR_BASE:?set ZEPHYR_BASE}"
[ -n "${ZEPHYR_MODULES:-}${ZEPHYR_EXTRA_MODULES:-}" ] || \
    { echo "set ZEPHYR_MODULES (tools/cts/zephyr-modules.sh) or ZEPHYR_EXTRA_MODULES" >&2; exit 1; }
[ -d "$CTS" ] || { echo "no VK-GL-CTS checkout at $CTS (set CTS=)" >&2; exit 1; }
mkdir -p "$WORK"

# --- 1. a private CTS copy with both patches ---------------------------------
if [ ! -d "$WORK/cts/.git" ]; then
    git clone -q --shared --no-checkout "$CTS" "$WORK/cts"
    git -C "$WORK/cts" checkout -q --detach "$(git -C "$CTS" rev-parse HEAD)"
    for d in "$CTS"/external/*/src; do
        n=$(basename "$(dirname "$d")")
        mkdir -p "$WORK/cts/external/$n"
        [ -e "$WORK/cts/external/$n/src" ] || ln -s "$d" "$WORK/cts/external/$n/src"
    done
    git -C "$WORK/cts" apply "$HERE/cts-yakogl-target.patch"
    git -C "$WORK/cts" apply "$HERE/cts-yakogl-zephyr.patch"
    if [ "$MODULE" = gles3 ]; then
        # the GLES3 direct link, as the target file does for GLES2
        cat >> "$WORK/cts/targets/yakogl-zephyr/yakogl-zephyr.cmake" <<'G3'

# MODULE=gles3 (tools/cts/cts-build-zephyr.sh): the GLES3 package, linked
# directly like GLES2.
set(DEQP_SUPPORT_GLES3	ON)
set(DEQP_GLES3_LIBRARIES	"")
add_definitions(-DDEQP_GLES3_LIBRARY_PATH=nullptr)
G3
    fi
fi

# --- 2. the flags of a Zephyr application of this board ----------------------
west build -p always --cmake-only -b "$BOARD" -d "$WORK/flags" "$REPO/driver/samples/zephyr/deqp" -- \
    ${ZEPHYR_MODULES:+"-DZEPHYR_MODULES=$ZEPHYR_MODULES"} ${ZEPHYR_EXTRA_MODULES:+"-DZEPHYR_EXTRA_MODULES=$ZEPHYR_EXTRA_MODULES"} \
    -DDEQP_SRC="$WORK/cts" -DDEQP_BUILD="$WORK/build" >/dev/null
# the headers a Zephyr build generates (syscall_list.h, autoconf.h, ...), which
# the POSIX headers include; configure alone does not make them
ninja -C "$WORK/flags" zephyr/syscall_list_h_target zephyr/driver_validation_h_target \
    zephyr/kobj_types_h_target zephyr/offsets_h zephyr/version_h >/dev/null
FLAGS=$(python3 - "$WORK/flags/compile_commands.json" <<'PY'
import json, shlex, sys
cc = json.load(open(sys.argv[1]))
e = [x for x in cc if x['file'].endswith('deqp/src/deqp_log.c')][0]
args = shlex.split(e['command'])[1:]
keep, i = [], 0
while i < len(args):
    a = args[i]
    if a in ('-isystem', '-imacros', '-include', '-I'):
        if not args[i + 1].endswith('zephyr_stdint.h'):
            keep += [a, args[i + 1]]
        i += 2
        continue
    if a.startswith(('-I', '-D', '-mcpu', '-mabi', '--sysroot', '-specs')) and '"' not in a:
        keep.append(a)
    i += 1
print(' '.join(keep))
PY
)
BIN="$SDK/gnu/aarch64-zephyr-elf/bin"
cat > "$WORK/zephyr-toolchain.cmake" <<TC
set(CMAKE_SYSTEM_NAME Generic)
set(CMAKE_SYSTEM_PROCESSOR aarch64)
set(CMAKE_C_COMPILER   $BIN/aarch64-zephyr-elf-gcc)
set(CMAKE_CXX_COMPILER $BIN/aarch64-zephyr-elf-g++)
set(CMAKE_AR           $BIN/aarch64-zephyr-elf-ar)
set(CMAKE_RANLIB       $BIN/aarch64-zephyr-elf-ranlib)
set(CMAKE_C_FLAGS_INIT   "-fno-pic $FLAGS")
set(CMAKE_CXX_FLAGS_INIT "-fno-pic $FLAGS")
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
TC
# CMake reads CMAKE_<LANG>_FLAGS_INIT only when it creates its cache. A stage-1 directory configured from
# another checkout or workspace keeps that one's -I paths (and then fails on a missing GLES2/gl2.h), so a cache
# that lacks the current flags is dropped; ninja then rebuilds what the new flags touch.
if [ -f "$WORK/build/CMakeCache.txt" ] && ! grep -qF -- "-fno-pic $FLAGS" "$WORK/build/CMakeCache.txt"; then
    echo "cts-build-zephyr: the cached CMake flags are stale; dropping $WORK/build/CMakeCache.txt"
    rm -f "$WORK/build/CMakeCache.txt"
fi

# --- 3. configure and build the libraries the sample links -------------------
cmake -S "$WORK/cts" -B "$WORK/build" -G Ninja \
    -DCMAKE_TOOLCHAIN_FILE="$WORK/zephyr-toolchain.cmake" \
    -DDEQP_TARGET=yakogl-zephyr -DDE_OS=DE_OS_UNIX -DDE_CPU=DE_CPU_ARM_64 \
    -DCMAKE_BUILD_TYPE=Release -DDEQP_DISABLE_VK_VIDEO_TESTS=ON \
    -DSELECTED_BUILD_TARGETS=deqp-$MODULE >/dev/null
ninja -C "$WORK/build" -j"$(nproc)" \
    deqp-$MODULE-package deqp-$MODULE-functional deqp-$MODULE-accuracy deqp-$MODULE-performance \
    deqp-$MODULE-stress deqp-gl-shared glutil-sglr glutil glwrapper eglutil eglwrapper \
    tcutil-platform tcutil referencerenderer randomshaders qphelper xexml \
    decpp deutil dethread depool debase png z jsoncpp_static
echo "cts-build-zephyr: libraries in $WORK/build"

# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# CMake toolchain for the Bootlin aarch64 glibc toolchain, targeting the
# Toradex Verdin AM62P (Cortex-A53, Torizon OS 7.6.1, glibc 2.39).
#
# The toolchain's glibc is 2.39 and its libstdc++ is 6.0.32; the board runs
# glibc 2.39 and libstdc++ 6.0.32, so nothing has to be shipped alongside the
# binaries and nothing has to be linked statically.
#
# NOTE: ~/pvr-lab/aarch64--glibc--stable-2024.05-1 is a pruned copy. It has no
# crt1.o / crti.o / crtbegin*.o, so it can compile (which is all the pvrlab
# kernel module needs) but cannot link an executable:
#
#   ld: cannot find Scrt1.o: No such file or directory
#
# cts-build.sh therefore unpacks a complete copy of the same release under
# build/cts/toolchain/ and passes it as BOOTLIN_ROOT. Set BOOTLIN_ROOT
# (environment or -D) to use a copy somewhere else.

set(CMAKE_SYSTEM_NAME      Linux)
set(CMAKE_SYSTEM_PROCESSOR aarch64)

if (NOT DEFINED BOOTLIN_ROOT)
    if (DEFINED ENV{BOOTLIN_ROOT})
        set(BOOTLIN_ROOT "$ENV{BOOTLIN_ROOT}")
    else ()
        message(FATAL_ERROR
            "BOOTLIN_ROOT is not set. Run tools/cts/cts-build.sh, which unpacks "
            "the toolchain and passes it, or set -DBOOTLIN_ROOT=<sdk root>.")
    endif ()
endif ()
set(BOOTLIN_ROOT "${BOOTLIN_ROOT}" CACHE PATH "Bootlin toolchain root")

set(CMAKE_C_COMPILER   "${BOOTLIN_ROOT}/bin/aarch64-linux-gcc")
set(CMAKE_CXX_COMPILER "${BOOTLIN_ROOT}/bin/aarch64-linux-g++")
set(CMAKE_AR           "${BOOTLIN_ROOT}/bin/aarch64-linux-ar"     CACHE FILEPATH "ar")
set(CMAKE_RANLIB       "${BOOTLIN_ROOT}/bin/aarch64-linux-ranlib" CACHE FILEPATH "ranlib")
set(CMAKE_STRIP        "${BOOTLIN_ROOT}/bin/aarch64-linux-strip"  CACHE FILEPATH "strip")

set(CMAKE_SYSROOT      "${BOOTLIN_ROOT}/aarch64-buildroot-linux-gnu/sysroot")
set(CMAKE_FIND_ROOT_PATH "${CMAKE_SYSROOT}")

# Look for programs on the host, for everything else only inside the sysroot,
# so find_package(ZLIB) / find_package(PNG) do not pick up the host's x86
# copies; both then fall back to the CTS's own external/zlib and external/libpng.
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM BEFORE)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)

set(CMAKE_C_FLAGS_INIT   "-march=armv8-a -mtune=cortex-a53")
set(CMAKE_CXX_FLAGS_INIT "-march=armv8-a -mtune=cortex-a53")

#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Build dEQP-GLES2 and dEQP-EGL for the Toradex Verdin AM62P (aarch64, Torizon
# OS, glibc 2.39) against the YakoGL dEQP target.
#
#   tools/cts/cts-build.sh
#
# Output: $BUILD/modules/gles2/deqp-gles2 and $BUILD/modules/egl/deqp-egl, each
# with its data directory alongside. Neither links GL or EGL: the YakoGL target
# dlopen()s libGLESv2.so.2 and libEGL.so.1 at run time.
#
# Environment:
#   CTS           VK-GL-CTS checkout   (default ~/com/github/KhronosGroup/VK-GL-CTS)
#   BUILD         build directory      (default <repo>/build/cts/aarch64)
#   BOOTLIN_ROOT  aarch64 SDK root     (default <repo>/build/cts/toolchain/...)
#   BOOTLIN_TARBALL  a local copy of the SDK tarball, so the build needs no network
#
# The CTS checkout is the user's and is never committed to. This script adds the
# three YakoGL target files to it from cts-yakogl-target.patch if they are not
# already there, and leaves them uncommitted, which is where they belong.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
CTS=${CTS:-$HOME/com/github/KhronosGroup/VK-GL-CTS}
BUILD=${BUILD:-$REPO/build/cts/aarch64}
TCDIR=${TCDIR:-$REPO/build/cts/toolchain}
SDK=aarch64--glibc--stable-2024.05-1
SDK_URL=https://toolchains.bootlin.com/downloads/releases/toolchains/aarch64/tarballs/$SDK.tar.xz

[ -d "$CTS" ] || { echo "no VK-GL-CTS checkout at $CTS (set CTS=)" >&2; exit 1; }

# --- 1. the toolchain -------------------------------------------------------
# ~/pvr-lab/aarch64--glibc--stable-2024.05-1 is pruned: no crt1.o, so it cannot
# link an executable. Unpack a complete copy of the same release instead.
if [ -z "${BOOTLIN_ROOT:-}" ]; then
    BOOTLIN_ROOT=$TCDIR/$SDK
fi
if [ ! -x "$BOOTLIN_ROOT/bin/aarch64-linux-g++" ]; then
    echo "== unpacking $SDK into $TCDIR"
    mkdir -p "$TCDIR"
    tarball=${BOOTLIN_TARBALL:-$TCDIR/$SDK.tar.xz}
    if [ ! -f "$tarball" ]; then
        echo "== downloading $SDK_URL"
        curl -fL -o "$TCDIR/$SDK.tar.xz" "$SDK_URL"
        tarball=$TCDIR/$SDK.tar.xz
    fi
    tar -C "$TCDIR" -xf "$tarball"
    # The SDK is built for a fixed prefix; relocate-sdk.sh rewrites it in place.
    "$TCDIR/$SDK/relocate-sdk.sh"
fi
# A pruned SDK compiles but cannot link, and CMake's compiler check is the first
# thing that would fail with an unhelpful message. Say so here instead.
for o in Scrt1.o crti.o; do
    find "$BOOTLIN_ROOT" -name "$o" -print -quit | grep -q . || {
        echo "$BOOTLIN_ROOT has no $o: this SDK copy is pruned and cannot link." >&2
        echo "Unset BOOTLIN_ROOT to let this script unpack a complete one." >&2
        exit 1; }
done
export BOOTLIN_ROOT

# --- 2. the YakoGL target inside the CTS checkout ---------------------------
if [ ! -f "$CTS/targets/yakogl/yakogl.cmake" ]; then
    echo "== adding the yakogl target to $CTS (uncommitted)"
    git -C "$CTS" apply "$HERE/cts-yakogl-target.patch"
fi

# --- 3. external sources ----------------------------------------------------
if [ ! -d "$CTS/external/spirv-headers/src/include" ]; then
    echo "== fetching CTS external sources"
    (cd "$CTS" && python3 external/fetch_sources.py)
fi

# --- 4. configure and build -------------------------------------------------
# SELECTED_BUILD_TARGETS keeps glcts and the Vulkan modules out: ~780 build
# steps instead of tens of thousands.
cmake -S "$CTS" -B "$BUILD" -G Ninja \
    -DCMAKE_TOOLCHAIN_FILE="$HERE/toolchain-aarch64-bootlin.cmake" \
    -DBOOTLIN_ROOT="$BOOTLIN_ROOT" \
    -DCMAKE_BUILD_TYPE=Release \
    -DDEQP_TARGET=yakogl \
    -DSELECTED_BUILD_TARGETS="deqp-gles2 deqp-egl"

cmake --build "$BUILD" -- -j"$(nproc)"

ls -l "$BUILD/modules/gles2/deqp-gles2" "$BUILD/modules/egl/deqp-egl"

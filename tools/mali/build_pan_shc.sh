#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# build_pan_shc.sh - build pan_shc (tools/mali/README-pan_shc.md) inside a
# Mesa 26.2.1 tree.
#
# Usage:
#   tools/mali/build_pan_shc.sh <mesa-26.2.1.tar.xz> [<work dir>]
#       extract the release tarball into <work dir>/mesa-26.2.1 (default: the
#       directory holding the tarball; an existing tree there is deleted
#       first), add pan_shc, configure build-pan and build it.
#   tools/mali/build_pan_shc.sh --update <mesa tree>
#       copy this checkout's pan_shc sources into a tree made by the first
#       form and rebuild (what tools/check.sh does with MESA_PAN_TREE).
# Prints PAN_SHC=<path of the binary> last.
#
# Meson options: -Dgallium-drivers=panfrost -Dtools=panfrost, no Vulkan and
# no GL/EGL/GLX/GBM. A Mesa without any GL, Vulkan or CL user builds NIR as a
# stub (with_nir_headers_only, src/compiler/nir/meson.build:81-95), so a
# panfrost-only tools build cannot link pan_shc; the gallium panfrost driver
# is the GL user here, as v3d is in tools/v3d/README.md's tree. It needs its
# CL library (libpan), so mesa_clc is built in the tree, as
# tools/pco_shc/build_pco_shc.sh does: clang and LLVM of one version,
# SPIRV-LLVM-Translator, SPIRV-Tools, meson, ninja, python mako, pyyaml and
# packaging, libdrm, expat, zlib, zstd. LLVM_DIR (default /usr/lib/llvm/23)
# selects the LLVM. pan_shc itself uses none of libpan.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
LLVM_DIR="${LLVM_DIR:-/usr/lib/llvm/23}"
export PATH="$LLVM_DIR/bin:$PATH"
export PKG_CONFIG_PATH="$LLVM_DIR/lib64/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
ROOT="$(cd "$HERE/../.." && pwd)"

usage() { sed -n '8,15p' "$0" >&2; exit 2; }

# install_sources <tree>: pan_shc.c, the shim, the meson file, the three
# gallium passes (copied unmodified from the same tree) and the build line.
install_sources() {
  local dst=$1/src/panfrost/pan_shc g=$1/src/gallium/drivers/panfrost f
  mkdir -p "$dst"
  # copy only on a change: a fresh mtime on meson.build makes ninja rerun
  # Meson's regenerate on every --update, which fails in a stale build-pan
  cp_changed() { cmp -s "$1" "$2" || cp "$1" "$2"; }
  for f in "$HERE/pan_shc.c" "$HERE"/pan_shc-shim/pan_context.h "$HERE"/pan_shc-shim/pan_shader.h \
           "$g/pan_nir_lower_sysvals.c" "$g/pan_nir_lower_res_indices.c" \
           "$g/pan_nir_remove_fragcolor_stores.c"; do
    cp_changed "$f" "$dst/$(basename "$f")"
  done
  cp_changed "$HERE/pan_shc.meson.build" "$dst/meson.build"
  local describe
  describe=$(git -C "$ROOT" describe --always --dirty 2>/dev/null || echo unknown)
  printf '#define PAN_SHC_YAKOGL_DESCRIBE "%s"\n' "$describe" > "$dst/pan_shc_build.h.new"
  # rewrite only on a change, so ninja does not rebuild for nothing
  if cmp -s "$dst/pan_shc_build.h.new" "$dst/pan_shc_build.h"; then
    rm "$dst/pan_shc_build.h.new"
  else
    mv "$dst/pan_shc_build.h.new" "$dst/pan_shc_build.h"
  fi
}

if [ "${1-}" = "--update" ]; then
  [ $# -eq 2 ] || usage
  SRC="$(cd "$2" && pwd)"
  [ -f "$SRC/build-pan/build.ninja" ] ||
    { echo "build_pan_shc.sh: $SRC has no build-pan (make it with the first form)" >&2; exit 1; }
  grep -q "subdir('pan_shc')" "$SRC/src/panfrost/meson.build" ||
    { echo "build_pan_shc.sh: $SRC/src/panfrost/meson.build lacks the pan_shc subdir" >&2; exit 1; }
  install_sources "$SRC"
else
  [ $# -ge 1 ] && [ $# -le 2 ] || usage
  TARBALL="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
  WORK="$(mkdir -p "${2:-$(dirname "$TARBALL")}" && cd "${2:-$(dirname "$TARBALL")}" && pwd)"
  UPSTREAM="$ROOT/third_party/mesa-26.2.1/UPSTREAM"
  SHA=$(sed -n 's/^tarball-sha256: *//p' "$UPSTREAM")
  [ -n "$SHA" ] || { echo "no tarball-sha256 in $UPSTREAM" >&2; exit 1; }
  echo "$SHA  $TARBALL" | sha256sum -c -
  SRC="$WORK/mesa-26.2.1"
  rm -rf "$SRC"
  tar -C "$WORK" -xf "$TARBALL"
  patch -d "$SRC" -p1 < "$HERE/mesa-pan_shc.patch"
  install_sources "$SRC"
  meson setup "$SRC/build-pan" "$SRC" \
    -Dbuildtype=debug -Dtools=panfrost -Dgallium-drivers=panfrost -Dvulkan-drivers= \
    -Dplatforms= -Dglx=disabled -Degl=disabled -Dgbm=disabled \
    -Dgles1=disabled -Dgles2=disabled -Dopengl=false
fi

ninja -C "$SRC/build-pan" src/panfrost/pan_shc/pan_shc
echo "PAN_SHC=$SRC/build-pan/src/panfrost/pan_shc/pan_shc"

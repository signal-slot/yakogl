#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# drm_shim_ref.sh - the reference arm pan_shc is checked against on the host:
# Mesa's own GL driver for the Mali-G610 (gallium panfrost on panthor), run
# without a GPU on Mesa's drm-shim, compiling a program the way an
# application's draw makes it compile, with st/mesa's GLSL path.
#
# Usage:
#   tools/mali/drm_shim_ref.sh build <mesa-X.Y.Z.tar.xz> <work dir>
#       extract the tarball into <work dir>/mesa-X.Y.Z (deleted first), build
#       panfrost, EGL, GLES 2 and the drm-shim, and install into <work dir>/inst.
#   tools/mali/drm_shim_ref.sh run <work dir> vs.vert fs.frag <out prefix> [gl_ref.py options]
#       draw once with the program (tools/mali/gl_ref.py: --prim, --size,
#       --viewport, --depth-range) on the shim and keep
#       <out prefix>.trace (pandecode: the programs' bytes), .stdout and
#       .stderr (BIFROST_MESA_DEBUG=shaders,shaderdb: NIR and disassembly).
#
# The shim reports GPU_ID 0xa867 << 16 (PAN_GPU_ID=a867, panfrost_noop.c
# pan_get_gpu_id), revision 0 where the board's is 5; pan_shc's bytes are the
# same for 0xa8670000 and 0xa8670005 (tools/mali/README-pan_shc.md).
# MESA_LOADER_DRIVER_OVERRIDE=panthor: without it Mesa 25.0's loader takes the
# host's own render node (its PCI id) for the shim's and loads no driver.
# Mesa releases before the glibc that defines once_flag (25.0 here) do not
# build against it; tools/mali/mesa-glibc-once_flag.patch is the guard 26.2.1
# carries (src/c11/threads.h, src/c11/impl/threads_posix.c), applied only to a
# tree that lacks it. It touches no compiler or driver code.
# Needs what tools/mali/build_pan_shc.sh needs, and python3 for gl_ref.py.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
LLVM_DIR="${LLVM_DIR:-/usr/lib/llvm/23}"
export PATH="$LLVM_DIR/bin:$PATH"
export PKG_CONFIG_PATH="$LLVM_DIR/lib64/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"

usage() { sed -n '9,18p' "$0" >&2; exit 2; }

case "${1-}" in
build)
  [ $# -eq 3 ] || usage
  TARBALL="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
  WORK="$(mkdir -p "$3" && cd "$3" && pwd)"
  NAME=$(basename "$TARBALL" .tar.xz)
  SRC="$WORK/$NAME"
  rm -rf "$SRC" "$WORK/inst"
  tar -C "$WORK" -xf "$TARBALL"
  if ! grep -q __once_flag_defined "$SRC/src/c11/threads.h"; then
    patch -d "$SRC" -p1 < "$HERE/mesa-glibc-once_flag.patch"
  fi
  meson setup "$SRC/build" "$SRC" -Dbuildtype=debug -Dgallium-drivers=panfrost \
    -Dvulkan-drivers= -Dtools=drm-shim -Dplatforms= -Dglx=disabled -Degl=enabled \
    -Dgbm=disabled -Dgles1=disabled -Dgles2=enabled -Dopengl=false -Dglvnd=disabled \
    -Dprefix="$WORK/inst"
  ninja -C "$SRC/build" install
  echo "drm_shim_ref: $NAME installed in $WORK/inst"
  ;;
run)
  [ $# -ge 5 ] || usage
  WORK="$(cd "$2" && pwd)"
  VS=$3 FS=$4 OUT=$5
  shift 5
  INST=$WORK/inst/lib64
  [ -f "$INST/libpanfrost_noop_drm_shim.so" ] ||
    { echo "drm_shim_ref: $WORK has no build (run the build form)" >&2; exit 1; }
  rm -f "$OUT".trace*
  # pandecode appends .ctx-<n>.<frame> to the dump file name
  # python3 -u is unbuffered by itself, so no stdbuf is needed and the
  # process preloads only the shim.
  LD_PRELOAD=$INST/libpanfrost_noop_drm_shim.so LD_LIBRARY_PATH=$INST \
    PAN_GPU_ID=a867 MESA_LOADER_DRIVER_OVERRIDE=panthor MESA_SHADER_CACHE_DISABLE=true \
    BIFROST_MESA_DEBUG=shaders,shaderdb PAN_MESA_DEBUG=trace PANDECODE_DUMP_FILE="$OUT.trace" \
    python3 -u "$HERE/gl_ref.py" "$INST" "$VS" "$FS" "$@" > "$OUT.stdout" 2> "$OUT.stderr" || {
      tail -5 "$OUT.stderr" >&2
      echo "drm_shim_ref: the draw failed" >&2
      exit 1
    }
  traces=("$OUT".trace.*)
  [ -f "${traces[0]}" ] || { echo "drm_shim_ref: no trace was written" >&2; exit 1; }
  cat "${traces[@]}" > "$OUT.trace"
  rm -f "${traces[@]}"
  grep '^gl_ref:' "$OUT.stdout"
  echo "drm_shim_ref: $OUT.trace"
  ;;
*) usage ;;
esac

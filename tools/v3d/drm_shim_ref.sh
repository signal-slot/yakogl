#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# drm_shim_ref.sh - Mesa's own GL driver for the Broadcom V3D (gallium v3d),
# run without a GPU on Mesa's drm-shim, as the reference for the V3D 7.1
# (Raspberry Pi 5) frame builder and packet work. Same method as
# tools/mali/drm_shim_ref.sh. Also builds v3d_shc (tools/v3d/v3d_shc.c).
#
# Usage:
#   tools/v3d/drm_shim_ref.sh build <mesa-X.Y.Z.tar.xz> <work dir>
#       extract the tarball into <work dir>/mesa-X.Y.Z (deleted first), add
#       tools/v3d/v3d_shc.c and its meson target (mesa-v3d_shc.patch), build the
#       v3d driver, EGL, GLES 2, the drm-shim and v3d_shc, and install into
#       <work dir>/inst. v3d_shc is <work dir>/mesa-X.Y.Z/build/src/broadcom/v3d_shc.
#   tools/v3d/drm_shim_ref.sh run <work dir> <case> <out prefix> [GPU id]
#       <case> is clear or triangle (tools/v3d/gl_ref.py). Draws once on the
#       shim, which reports the GPU <GPU id> (42 or 71; 71 by default), with
#       V3D_DEBUG=clif, and writes <out prefix>.clif (the control lists Mesa
#       submits, in CLIF text), .stdout and .stderr (everything else).
#
# The shim's SUBMIT_CL is a no-op (drm-shim/v3d_noop.c), but Mesa writes the
# CLIF dump before it calls the ioctl (v3d_job.c v3d_clif_dump), so the lists
# are complete. MESA_LOADER_DRIVER_OVERRIDE=v3d: without it Mesa's loader takes
# the host's own render node for the shim's and loads no driver.
# Needs meson, ninja, a C compiler, python3 with mako and pyyaml, libdrm and expat
# (the packages Mesa's build lists), and python3 for gl_ref.py.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"

usage() { sed -n '9,20p' "$0" >&2; exit 2; }

case "${1-}" in
build)
  [ $# -eq 3 ] || usage
  TARBALL="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
  WORK="$(mkdir -p "$3" && cd "$3" && pwd)"
  NAME=$(basename "$TARBALL" .tar.xz)
  SRC="$WORK/$NAME"
  rm -rf "$SRC" "$WORK/inst"
  tar -C "$WORK" -xf "$TARBALL"
  cp "$HERE/v3d_shc.c" "$SRC/src/broadcom/v3d_shc.c"
  patch -d "$SRC" -p1 < "$HERE/mesa-v3d_shc.patch"
  meson setup "$SRC/build" "$SRC" -Dbuildtype=debug -Dgallium-drivers=v3d \
    -Dvulkan-drivers= -Dtools=drm-shim -Dplatforms= -Dglx=disabled -Degl=enabled \
    -Dgbm=disabled -Dgles1=disabled -Dgles2=enabled -Dopengl=false -Dglvnd=disabled \
    -Dllvm=disabled -Dprefix="$WORK/inst"
  ninja -C "$SRC/build" install
  ninja -C "$SRC/build" src/broadcom/v3d_shc
  echo "drm_shim_ref: $NAME installed in $WORK/inst; v3d_shc in $SRC/build/src/broadcom/v3d_shc"
  ;;
run)
  [ $# -ge 4 ] || usage
  WORK="$(cd "$2" && pwd)"
  CASE=$3 OUT=$4 GPU=${5-71}
  INST=$WORK/inst/lib64
  [ -f "$INST/libv3d_noop_drm_shim.so" ] ||
    { echo "drm_shim_ref: $WORK has no build (run the build form)" >&2; exit 1; }
  LD_PRELOAD=$INST/libv3d_noop_drm_shim.so LD_LIBRARY_PATH=$INST \
    V3D_GPU_ID=$GPU MESA_LOADER_DRIVER_OVERRIDE=v3d MESA_SHADER_CACHE_DISABLE=true \
    V3D_DEBUG=clif \
    python3 -u "$HERE/gl_ref.py" "$INST" "$CASE" > "$OUT.stdout" 2> "$OUT.stderr" || {
      tail -5 "$OUT.stderr" >&2
      echo "drm_shim_ref: the draw failed" >&2
      exit 1
    }
  # Mesa's log lines carry the CLIF; everything else on stderr is kept apart.
  grep -c . "$OUT.stderr" >/dev/null || { echo "drm_shim_ref: no CLIF was written" >&2; exit 1; }
  cp "$OUT.stderr" "$OUT.clif"
  grep '^gl_ref:' "$OUT.stdout"
  echo "drm_shim_ref: $OUT.clif"
  ;;
*) usage ;;
esac

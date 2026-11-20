#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# drm_shim_ref.sh - Mesa's own GL driver for the VideoCore IV (gallium vc4,
# Raspberry Pi Zero 2 W), run without a GPU on Mesa's drm-shim, as the
# reference for the VC4 packet and QPU work. Same method as
# tools/v3d/drm_shim_ref.sh. Also builds vc4_shc (tools/vc4/vc4_shc.c) and the
# submit recorder (tools/vc4/vc4_submit_rec.c).
#
# Usage:
#   tools/vc4/drm_shim_ref.sh build <mesa-X.Y.Z.tar.xz> <work dir>
#       extract the tarball into <work dir>/mesa-X.Y.Z (deleted first), add
#       tools/vc4/vc4_shc.c and its meson target (mesa-vc4_shc.patch), build the
#       vc4 driver, EGL, GLES 2, the drm-shim and vc4_shc, install into
#       <work dir>/inst, and build the recorder into
#       <work dir>/inst/libvc4_submit_rec.so. vc4_shc is
#       <work dir>/mesa-X.Y.Z/build/src/gallium/drivers/vc4/vc4_shc.
#   tools/vc4/drm_shim_ref.sh run <work dir> <case> <out prefix>
#       <case> is clear, triangle or flat (tools/v3d/gl_ref.py). Draws once on
#       the shim with VC4_DEBUG=cl,qpu and the recorder preloaded, and writes
#       <out prefix>.submit<N>.* (the raw bytes of each DRM_IOCTL_VC4_SUBMIT_CL
#       and of the buffers it names, tools/vc4/README.md), .log (Mesa's text
#       dump: the binner list as vc4_cl_dump.c prints it and the QPU
#       disassembly) and .stdout (gl_ref.py's two lines).
#
# The shim stubs the submit: drm-shim/vc4_noop.c has no DRM_VC4_SUBMIT_CL entry,
# so the shim answers it -EINVAL and Mesa logs "Draw call returned Invalid
# argument" once. The recorder sits in front of the shim (first in
# LD_PRELOAD), writes the submit out and then hands the ioctl on unchanged.
# MESA_LOADER_DRIVER_OVERRIDE=vc4: without it Mesa's loader takes the host's
# own render node for the shim's and loads no driver.
# Needs meson, ninja, a C compiler, python3 with mako and pyyaml, libdrm and
# expat (the packages Mesa's build lists), and python3 for gl_ref.py.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"

usage() { sed -n '11,25p' "$0" >&2; exit 2; }

case "${1-}" in
build)
  [ $# -eq 3 ] || usage
  TARBALL="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
  WORK="$(mkdir -p "$3" && cd "$3" && pwd)"
  NAME=$(basename "$TARBALL" .tar.xz)
  SRC="$WORK/$NAME"
  rm -rf "$SRC" "$WORK/inst"
  tar -C "$WORK" -xf "$TARBALL"
  cp "$HERE/vc4_shc.c" "$SRC/src/gallium/drivers/vc4/vc4_shc.c"
  patch -d "$SRC" -p1 < "$HERE/mesa-vc4_shc.patch"
  meson setup "$SRC/build" "$SRC" -Dbuildtype=debug -Dgallium-drivers=vc4 \
    -Dvulkan-drivers= -Dtools=drm-shim -Dplatforms= -Dglx=disabled -Degl=enabled \
    -Dgbm=disabled -Dgles1=disabled -Dgles2=enabled -Dopengl=false -Dglvnd=disabled \
    -Dllvm=disabled -Dprefix="$WORK/inst"
  ninja -C "$SRC/build" install
  ninja -C "$SRC/build" src/gallium/drivers/vc4/vc4_shc
  # The recorder: libc and Mesa's copy of the kernel's vc4_drm.h only.
  cc -O2 -Wall -Werror -shared -fPIC -I"$SRC/include" \
    -o "$WORK/inst/libvc4_submit_rec.so" "$HERE/vc4_submit_rec.c" -ldl
  echo "drm_shim_ref: $NAME installed in $WORK/inst; vc4_shc in $SRC/build/src/gallium/drivers/vc4/vc4_shc"
  ;;
run)
  [ $# -eq 4 ] || usage
  WORK="$(cd "$2" && pwd)"
  CASE=$3 OUT=$4
  case $CASE in clear | triangle | flat) ;; *) usage ;; esac
  # meson's libdir: lib64 (Fedora), lib/<multiarch> (Debian, Ubuntu) or lib
  INST=$WORK/inst/lib64
  for d in "$WORK"/inst/lib/*-linux-gnu "$WORK/inst/lib"; do
    [ -f "$INST/libvc4_noop_drm_shim.so" ] || INST=$d
  done
  [ -f "$INST/libvc4_noop_drm_shim.so" ] && [ -f "$WORK/inst/libvc4_submit_rec.so" ] ||
    { echo "drm_shim_ref: $WORK has no build (run the build form)" >&2; exit 1; }
  rm -f "$OUT".submit[0-9]*.* "$OUT.log" "$OUT.stdout"
  # The recorder comes first so that its ioctl() and mmap() see the calls
  # before the shim's do.
  LD_PRELOAD="$WORK/inst/libvc4_submit_rec.so:$INST/libvc4_noop_drm_shim.so" \
    LD_LIBRARY_PATH=$INST VC4_SUBMIT_REC="$OUT" \
    MESA_LOADER_DRIVER_OVERRIDE=vc4 MESA_SHADER_CACHE_DISABLE=true VC4_DEBUG=cl,qpu \
    python3 -u "$HERE/../v3d/gl_ref.py" "$INST" "$CASE" > "$OUT.stdout" 2> "$OUT.log" || {
      tail -5 "$OUT.log" >&2
      echo "drm_shim_ref: the draw failed" >&2
      exit 1
    }
  ls "$OUT".submit1.txt >/dev/null 2>&1 || { echo "drm_shim_ref: no submit was recorded" >&2; exit 1; }
  grep '^gl_ref:' "$OUT.stdout"
  echo "drm_shim_ref: $(ls "$OUT".submit[0-9]*.txt | wc -l) submit(s), $OUT.submit*.txt"
  ;;
*) usage ;;
esac

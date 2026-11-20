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
#       v3d driver, EGL, GLES 2, the drm-shim (with a 7.1.10 GPU added) and
#       v3d_shc, and install into <work dir>/inst. v3d_shc is
#       <work dir>/mesa-X.Y.Z/build/src/broadcom/v3d_shc.
#   tools/v3d/drm_shim_ref.sh run <work dir> <case> <out prefix> [GPU id]
#       <case> is clear or triangle (tools/v3d/gl_ref.py), or a gl_test shape
#       of tools/v3d/pi5/src/gl_shapes.py. Draws once on the shim, which
#       reports the GPU <GPU id> (42, 71 or 7.1.10; 71 by default), with
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

usage() { sed -n '9,22p' "$0" >&2; exit 2; }

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
  # A third shim GPU, "7.1.10": the Raspberry Pi 5's identity registers as its
  # own Linux reads them (tools/v3d/pi5/v3d_regs.txt): HUB_IDENT3 0x00020a10 is
  # rev 10, compat_rev 2 (Mesa's "V3D 7.1.10.2", the BCM2712 D0 draw-index
  # shader record), CORE0_IDENT1 0x81001431 is 3 slices of 4 QPUs. Appended
  # after "71" and "42", whose entries and selection are unchanged ("7.1.10"
  # does not match "71" in v3d_noop.c's strncasecmp loop, and "71" stays first).
  SHIM="$SRC/src/broadcom/drm-shim/v3d_noop.c"
  awk '
    /^static const struct v3d_shim_gpu gpus\[\] = \{/ { in_gpus = 1 }
    in_gpus && /^};/ {
      print "        {"
      print "                .name = \"7.1.10\","
      print "                .reg_map = (const uint32_t[]) {"
      print "                        [DRM_V3D_PARAM_V3D_UIFCFG]       = 0x00000045,"
      print "                        [DRM_V3D_PARAM_V3D_HUB_IDENT1]   = 0x00081117,"
      print "                        [DRM_V3D_PARAM_V3D_HUB_IDENT2]   = 0x00001900,"
      print "                        [DRM_V3D_PARAM_V3D_HUB_IDENT3]   = 0x00020a10,"
      print "                        [DRM_V3D_PARAM_V3D_CORE0_IDENT0] = 0x07443356,"
      print "                        [DRM_V3D_PARAM_V3D_CORE0_IDENT1] = 0x81001431,"
      print "                        [DRM_V3D_PARAM_V3D_CORE0_IDENT2] = 0xc0078101,"
      print "                }"
      print "        },"
      in_gpus = 0
    }
    { print }
  ' "$SHIM" > "$SHIM.new"
  mv "$SHIM.new" "$SHIM"
  grep -q '"7.1.10"' "$SHIM" || { echo "drm_shim_ref: the 7.1.10 shim GPU was not added" >&2; exit 1; }
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
  # meson's libdir: lib64 (Fedora), lib/<multiarch> (Debian, Ubuntu) or lib
  INST=$WORK/inst/lib64
  for d in "$WORK"/inst/lib/*-linux-gnu "$WORK/inst/lib"; do
    [ -f "$INST/libv3d_noop_drm_shim.so" ] || INST=$d
  done
  # clear and triangle are gl_ref.py's; every other case is a gl_test shape of
  # tools/v3d/pi5/src/gl_shapes.py (its docstring lists them).
  case $CASE in
  clear|triangle) PROG=$HERE/gl_ref.py ;;
  *) PROG=$HERE/pi5/src/gl_shapes.py ;;
  esac
  [ -f "$INST/libv3d_noop_drm_shim.so" ] ||
    { echo "drm_shim_ref: $WORK has no build (run the build form)" >&2; exit 1; }
  LD_PRELOAD=$INST/libv3d_noop_drm_shim.so LD_LIBRARY_PATH=$INST \
    V3D_GPU_ID=$GPU MESA_LOADER_DRIVER_OVERRIDE=v3d MESA_SHADER_CACHE_DISABLE=true \
    V3D_DEBUG=clif \
    python3 -u "$PROG" "$INST" "$CASE" > "$OUT.stdout" 2> "$OUT.stderr" || {
      tail -5 "$OUT.stderr" >&2
      echo "drm_shim_ref: the draw failed" >&2
      exit 1
    }
  # Mesa's log lines carry the CLIF; everything else on stderr is kept apart.
  grep -c . "$OUT.stderr" >/dev/null || { echo "drm_shim_ref: no CLIF was written" >&2; exit 1; }
  cp "$OUT.stderr" "$OUT.clif"
  grep '^gl_\(ref\|shapes\):' "$OUT.stdout"
  echo "drm_shim_ref: $OUT.clif"
  ;;
*) usage ;;
esac

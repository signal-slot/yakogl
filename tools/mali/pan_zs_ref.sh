#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# pan_zs_ref.sh - Mesa 26.2.1's frame descriptors for dEQP-GLES2's
# fbo.render cases with shared or recreated depth/stencil renderbuffers and
# the rgb565 stencil clear: the reference of the Mali frame builder's
# depth/stencil sharing and stencil clears (plan #373 items zs-share and
# misc8-fix). Writes tools/mali/testdata/pan_zs_ref/<case>.txt, one file
# per case, byte for byte the same on every run.
#
#   MESA_PAN_TREE=<tree> tools/mali/pan_zs_ref.sh [<work dir>]
#   tools/mali/pan_zs_ref.sh --tree <tree> [<work dir>]
#
# Inputs:
#   - MESA_PAN_TREE (or --tree): a Mesa 26.2.1 source tree, the one
#     tools/mali/build_pan_shc.sh makes (its VERSION must read 26.2.1). The
#     script builds gallium panfrost, EGL (surfaceless), GLES 2 and the
#     drm-shim in <tree>/build-shim, installed into <tree>/build-shim/inst,
#     with tools/mali/drm_shim_ref.sh's options (configured once; later runs
#     only run `meson install`, which rebuilds what changed). Needs meson,
#     ninja, a C/C++ compiler and LLVM (LLVM_DIR, /usr/lib/llvm-21 unless
#     set), as drm_shim_ref.sh does.
#   - tools/mali/testdata/pan_zs_ref/zs_seq.py: the GL sequence of each case
#     (`zs_seq.py --list` names them; each is transcribed from VK-GL-CTS
#     modules/gles2/functional/es2fFboRenderTest.cpp), run on the shim with
#     PAN_GPU_ID=a867 and PAN_MESA_DEBUG=linear,nocrc,trace,dump (linear
#     surfaces and no CRC, as the Mali back end draws, as in
#     tools/mali/mesa-board-ref.sh; no `sync`, which aborts on the shim,
#     where no job completes).
#   - tools/mali/testdata/pan_zs_ref/cut.py: keeps, per frame, the draws'
#     scissor and depth/stencil descriptors, the decoded framebuffer
#     descriptor, ZS/CRC extension and render targets (their clear words),
#     and the raw bytes of those descriptors from the memory dump.
#
# <work dir> ($TMPDIR/pan_zs_ref unless given) keeps each case's full trace
# (<case>.trace, whose lines the excerpts' `## ... lines a-b` headers count),
# .stdout and .stderr. The full trace is not committed and has no stable
# hash: its host CPU addresses (pandecode's `Shader 0x...` lines, pointers
# in some dumped buffers) change from run to run; nothing cut.py keeps holds
# one, so the excerpts are the same on every run.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
DATA=$HERE/testdata/pan_zs_ref
if [ "${1-}" = --tree ]; then
  [ $# -ge 2 ] || { echo "pan_zs_ref: --tree needs a directory" >&2; exit 2; }
  MESA_PAN_TREE=$2
  shift 2
fi
TREE="${MESA_PAN_TREE:?set MESA_PAN_TREE to a Mesa 26.2.1 tree (tools/mali/build_pan_shc.sh)}"
TREE="$(cd "$TREE" && pwd)"
WORK="${1:-${TMPDIR:?set TMPDIR or name a work dir}/pan_zs_ref}"
mkdir -p "$WORK"
WORK="$(cd "$WORK" && pwd)"
LLVM_DIR="${LLVM_DIR:-/usr/lib/llvm-21}"
export PATH="$LLVM_DIR/bin:$PATH"
export PKG_CONFIG_PATH="$LLVM_DIR/lib/pkgconfig:$LLVM_DIR/lib64/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
BUILD=$TREE/build-shim
[ "$(cat "$TREE/VERSION")" = 26.2.1 ] || { echo "pan_zs_ref: $TREE is not Mesa 26.2.1" >&2; exit 1; }
if [ ! -f "$BUILD/build.ninja" ]; then
  meson setup "$BUILD" "$TREE" -Dbuildtype=debug -Dgallium-drivers=panfrost \
    -Dvulkan-drivers= -Dtools=drm-shim -Dplatforms= -Dglx=disabled -Degl=enabled \
    -Dgbm=disabled -Dgles1=disabled -Dgles2=enabled -Dopengl=false -Dglvnd=disabled \
    -Dprefix="$BUILD/inst"
fi
meson install -C "$BUILD" --quiet >/dev/null
INST=$BUILD/inst/lib64
# Debian-style hosts install under lib/<multiarch> instead of lib64
[ -d "$INST" ] || INST=$BUILD/inst/lib/$(gcc -dumpmachine)
[ -f "$INST/libpanfrost_noop_drm_shim.so" ] || { echo "pan_zs_ref: no drm-shim in $INST" >&2; exit 1; }

cases=$(python3 -I "$DATA/zs_seq.py" --list)
for c in $cases; do
  out=$WORK/$c
  rm -f "$out".trace*
  # pandecode appends .<process>.ctx-<n>.<frame> to the dump file name
  LD_PRELOAD=$INST/libpanfrost_noop_drm_shim.so LD_LIBRARY_PATH=$INST \
    PAN_GPU_ID=a867 MESA_LOADER_DRIVER_OVERRIDE=panthor MESA_SHADER_CACHE_DISABLE=true \
    PAN_MESA_DEBUG=linear,nocrc,trace,dump PANDECODE_DUMP_FILE="$out.trace" \
    python3 -I -u "$DATA/zs_seq.py" "$INST" "$c" > "$out.stdout" 2> "$out.stderr" || {
      tail -5 "$out.stderr" >&2
      echo "pan_zs_ref: $c: the run failed" >&2
      exit 1
    }
  traces=("$out".trace.*)
  [ -f "${traces[0]}" ] || { echo "pan_zs_ref: $c: no trace was written" >&2; exit 1; }
  cat "${traces[@]}" > "$out.trace"
  rm -f "${traces[@]}"
  python3 -I "$DATA/cut.py" "$out.trace" "$out.stdout" "$c.trace" > "$DATA/$c.txt"
  echo "pan_zs_ref: $DATA/$c.txt ($(grep -m1 '^# Frames:' "$DATA/$c.txt"))"
done
# an excerpt no case writes any more is stale
for f in "$DATA"/*.txt; do
  grep -qx "$(basename "$f" .txt)" <<< "$cases" ||
    { echo "pan_zs_ref: $f is no case's (remove it)" >&2; exit 1; }
done

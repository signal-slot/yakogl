#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# vs_cube_edge_ref.sh - Mesa 26.2.1's references for issue #7489 on the
# Mali-G610: a vertex-shader cube look-up with GL_REPEAT and a LINEAR filter
# near a face edge (dEQP-GLES2 texture.vertex.cube.filtering.
# linear_mipmap_linear_{linear,nearest}_repeat, .wrap.repeat_repeat; row 5 of
# docs/es2-cts-failures.md section M.1) and textureCubeLod
# (shaders.texture_functions.vertex.texturecubelod, row 11, and its fragment
# stage counterpart). Rewrites, byte for byte, in tools/mali/testdata/:
#
#   <prog>.expect.json   the programs of the case's draw (pan_ref_diff.py
#                        extract --draw: words, disassembly, registers,
#                        preload, FAU words), for prog vs_cube_edge,
#                        cube_lod_vs, cube_lod_fs;
#   <case>.<ctx>.desc    the case's sampler and texture descriptors, decoded
#                        and as raw 32-bit words, with the texture's plane
#                        descriptors (vs_cube_edge_cut.py), for ctx
#                        es3 (the context Mesa makes for a GLES 2 request:
#                        ES 3.2, seamless cube maps) and es2
#                        (MESA_GLES_VERSION_OVERRIDE=2.0: an ES 2.0 context,
#                        no seamless filtering, Appendix F.2 of ES 3.0);
#                        cube_lod_fs (GLSL ES 3.00) in es3 only. The
#                        `centre` case is drawn and checked, not written.
#
# Usage:
#   tools/mali/testdata/vs_cube_edge_ref.sh <drm_shim_ref work dir>
#
# <drm_shim_ref work dir> is a Mesa 26.2.1 build of tools/mali/drm_shim_ref.sh's
# build form (its inst/ holds the drm-shim, libEGL and libGLESv2), e.g.
# $CONNECTED_DEPS_DIR/mesa-shim. The cases and their GL state are
# vs_cube_edge_draw.py's (--list). Each case runs in its own process with
# drm_shim_ref.sh run's environment (PAN_GPU_ID=a867) and
# PAN_MESA_DEBUG=trace,dump (the memory dump holds the descriptors' bytes).
# The texture uploads are blits with draws of their own, so the case's draw
# is the trace's last RUN_IDVS. VS_CUBE_EDGE_TRACES=<dir> keeps each case's
# whole trace there.
#
# Checks (the script fails otherwise): every case of one program draws the
# same program words in both contexts (so one <prog>.expect.json holds for
# all of them; the `edge` and `centre` directions in particular), and the
# `edge` and `centre` cases have the same descriptors.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
TOOLS="$(cd "$HERE/.." && pwd)"
[ $# -eq 1 ] || { sed -n '23,24p' "$0" >&2; exit 2; }
WORK="$(cd "$1" && pwd)"
INST=$WORK/inst/lib64
[ -d "$INST" ] || INST=$WORK/inst/lib/$(gcc -dumpmachine)
[ -f "$INST/libpanfrost_noop_drm_shim.so" ] ||
  { echo "vs_cube_edge_ref: $WORK has no drm_shim_ref.sh build" >&2; exit 1; }
TMP="$(mktemp -d "${TMPDIR:?set TMPDIR}/vs_cube_edge_ref.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

run_case() { # <case> <ctx>
  local name=$1 ctx=$2 prog=${1%%.*} over=()
  [ "$ctx" = es2 ] && over=(MESA_GLES_VERSION_OVERRIDE=2.0)
  rm -f "$TMP/t.trace"*
  env ${over[@]+"${over[@]}"} LD_PRELOAD="$INST/libpanfrost_noop_drm_shim.so" LD_LIBRARY_PATH="$INST" \
    PAN_GPU_ID=a867 MESA_LOADER_DRIVER_OVERRIDE=panthor MESA_SHADER_CACHE_DISABLE=true \
    PAN_MESA_DEBUG=trace,dump PANDECODE_DUMP_FILE="$TMP/t.trace" \
    python3 -I "$HERE/vs_cube_edge_draw.py" "$INST" "$HERE" "$name" > "$TMP/draw.out" 2>&1 ||
    { tail -5 "$TMP/draw.out" >&2; echo "vs_cube_edge_ref: $name $ctx: the draw failed" >&2; exit 1; }
  cat "$TMP/t.trace".* > "$TMP/full"
  rm -f "$TMP/t.trace".*
  [ -z "${VS_CUBE_EDGE_TRACES-}" ] || cp "$TMP/full" "$VS_CUBE_EDGE_TRACES/$name.$ctx.trace"
  python3 -I "$HERE/vs_cube_edge_cut.py" "$TMP/full" \
    "$(python3 -I "$HERE/vs_cube_edge_draw.py" --title "$name"); context $ctx" "$TMP/draw.out" \
    "$TMP/draw.trace" > "$TMP/$name.$ctx.desc"
  python3 -I "$TOOLS/pan_ref_diff.py" extract "$TMP/draw.trace" --draw > "$TMP/$name.$ctx.json"
  sed -i "s/\"trace\": \"[^\"]*\"/\"trace\": \"$prog.trace\"/" "$TMP/$name.$ctx.json"
  if [ -f "$TMP/$prog.expect.json" ]; then
    cmp -s "$TMP/$prog.expect.json" "$TMP/$name.$ctx.json" ||
      { echo "vs_cube_edge_ref: $name $ctx draws other words than $prog's first case" >&2; exit 1; }
  else
    cp "$TMP/$name.$ctx.json" "$TMP/$prog.expect.json"
  fi
  echo "vs_cube_edge_ref: $name.$ctx"
}

cases=$(python3 -I "$HERE/vs_cube_edge_draw.py" --list)
for c in $cases; do
  for ctx in $(python3 -I "$HERE/vs_cube_edge_draw.py" --contexts "$c"); do
    run_case "$c" "$ctx"
  done
done
# the direction changes no descriptor word
for ctx in es3 es2; do
  diff <(grep -v '^#' "$TMP/vs_cube_edge.lml_linear_repeat.edge.$ctx.desc") \
    <(grep -v '^#' "$TMP/vs_cube_edge.lml_linear_repeat.centre.$ctx.desc") >&2 ||
    { echo "vs_cube_edge_ref: the edge and centre directions differ in their descriptors ($ctx)" >&2; exit 1; }
done
for p in vs_cube_edge cube_lod_vs cube_lod_fs; do
  cp "$TMP/$p.expect.json" "$HERE/$p.expect.json"
  echo "vs_cube_edge_ref: $HERE/$p.expect.json"
done
for c in $cases; do
  case $c in *.centre) continue ;; esac
  for ctx in $(python3 -I "$HERE/vs_cube_edge_draw.py" --contexts "$c"); do
    cp "$TMP/$c.$ctx.desc" "$HERE/$c.$ctx.desc"
    echo "vs_cube_edge_ref: $HERE/$c.$ctx.desc"
  done
done

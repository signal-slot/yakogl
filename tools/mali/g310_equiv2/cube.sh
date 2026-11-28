#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# g310_equiv2/cube.sh - G310v2 = G610 for the three cube references that
# g310_check.sh cannot draw (vs_cube_edge, cube_lod_vs, cube_lod_fs: made by
# testdata/vs_cube_edge_draw.py, not gl_ref.py). Draws each case (ES 3 context,
# the one the references were made in) with PAN_GPU_ID=a867 (G610) and
# ac74:1 (G310v2), checks the G610 extract equals the committed
# <prog>.expect.json and the G310v2 extract equals the G610's, `trace` aside.
#
#   tools/mali/g310_equiv2/cube.sh [<shim dir>]   (default $CONNECTED_DEPS_DIR/mesa-pan-shim)
# Last line "N identical, M differ"; exit 1 if any differs.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
TD="$HERE/../testdata"
WORK="$(cd "${1:-${CONNECTED_DEPS_DIR:?set CONNECTED_DEPS_DIR}/mesa-pan-shim}" && pwd)"
INST=$WORK/inst/lib64
[ -d "$INST" ] || INST=$WORK/inst/lib/$(gcc -dumpmachine)
TMP="$(mktemp -d "${TMPDIR:?set TMPDIR}/g310_equiv2_cube.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
same=0; bad=0
for c in $(python3 -I "$TD/vs_cube_edge_draw.py" --list); do
  prog=${c%%.*}
  for g in a867 ac74:1; do
    rm -f "$TMP/t.trace"*
    LD_PRELOAD="$INST/libpanfrost_noop_drm_shim.so" LD_LIBRARY_PATH="$INST" PAN_GPU_ID=$g \
      MESA_LOADER_DRIVER_OVERRIDE=panthor MESA_SHADER_CACHE_DISABLE=true \
      PAN_MESA_DEBUG=trace,dump PANDECODE_DUMP_FILE="$TMP/t.trace" \
      python3 -I "$TD/vs_cube_edge_draw.py" "$INST" "$TD" "$c" > "$TMP/draw.out" 2>&1 ||
      { tail -5 "$TMP/draw.out" >&2; echo "cube: $c $g draw failed" >&2; exit 1; }
    cat "$TMP/t.trace".* > "$TMP/full"
    python3 -I "$HERE/../testdata/vs_cube_edge_cut.py" "$TMP/full" "x" "$TMP/draw.out" "$TMP/draw.trace" > /dev/null
    python3 -I "$HERE/../pan_ref_diff.py" extract "$TMP/draw.trace" --draw |
      sed "s/\"trace\": \"[^\"]*\"/\"trace\": \"$prog.trace\"/" > "$TMP/$g.json"
  done
  if cmp -s "$TMP/a867.json" "$TD/$prog.expect.json" && cmp -s "$TMP/a867.json" "$TMP/ac74:1.json"; then
    echo "IDENTICAL $c"; same=$((same+1))
  else
    cmp -s "$TMP/a867.json" "$TD/$prog.expect.json" || echo "DIFFERS $c (G610 redraw != committed)"
    cmp -s "$TMP/a867.json" "$TMP/ac74:1.json" || echo "DIFFERS $c (G310v2 != G610)"
    bad=$((bad+1))
  fi
done
echo "$same identical, $bad differ"
[ "$bad" = 0 ]

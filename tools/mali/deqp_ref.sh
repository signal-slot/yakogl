#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# deqp_ref.sh - one command from a GLSL ES 1.00 shader pair to a Mesa 26.2.1
# reference of the gm2_* kind (tools/mali/testdata/gm2_*): <prefix>.vert,
# <prefix>.frag and <prefix>.expect.json (trace, secondary_offset, regions).
# Used for the deqp_<family>_<shape> references (tools/mali/README-deqp_ref.md).
#
# Usage:
#   tools/mali/deqp_ref.sh <in.vert> <in.frag> <out-prefix> [--instrument pan_shc|shim] [--prim triangles|points]
#
#   Example (the pair under testdata/deqp_shapes/, the reference beside it):
#     tools/mali/deqp_ref.sh tools/mali/testdata/deqp_shapes/intval_1.vert \
#         tools/mali/testdata/deqp_shapes/intval_1.frag tools/mali/testdata/deqp_intval_1
#
#   --instrument shim     Mesa's gallium panfrost on drm-shim, one draw with
#                         --uniform-values, expect.json by
#                         `pan_ref_diff.py extract --draw` (the gm2_* method).
#   --instrument pan_shc  pan_shc through glsl2pan.py (the same regions and
#                         facts, from pan_shc's JSON; a FAU word the driver
#                         fills is null, a constant is its hex). The trace
#                         field names <prefix>.pan_shc.json.
#   default               shim when the fragment shader has a mediump or lowp
#                         value (a precision statement, a qualifier) or reads
#                         gl_FragCoord (README-pan_shc.md, Known gaps: the GLSL
#                         path lowers those to fp16 and pan_shc's SPIR-V path
#                         does not); pan_shc otherwise.
#   --prim                triangles (default) or points.
#
# Environment (each is built the documented way into $CONNECTED_DEPS_DIR when
# absent: the tarball of third_party/mesa-26.2.1/UPSTREAM, fetched there once):
#   MESA_PAN_TREE  a Mesa 26.2.1 tree from build_pan_shc.sh
#                  (default $CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1)
#   PAN_SHC        its pan_shc binary (default $MESA_PAN_TREE/build-pan/src/panfrost/pan_shc/pan_shc)
#   MESA_SHIM_WORK drm_shim_ref.sh's work dir (default $CONNECTED_DEPS_DIR/mesa-shim);
#                  needed by the shim instrument only
#   GLSLANG        glslangValidator (pan_shc instrument only)
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"

usage() { sed -n '9,11p' "$0" >&2; exit 2; }
die() { echo "deqp_ref: $*" >&2; exit 1; }

[ $# -ge 3 ] || usage
VS=$1 FS=$2 OUT=$3
shift 3
INSTRUMENT="" PRIM=triangles
while [ $# -gt 0 ]; do
  case "$1" in
  --instrument) [ $# -ge 2 ] || usage; INSTRUMENT=$2; shift 2 ;;
  --prim) [ $# -ge 2 ] || usage; PRIM=$2; shift 2 ;;
  *) usage ;;
  esac
done
case "$INSTRUMENT" in ""|pan_shc|shim) ;; *) usage ;; esac
case "$PRIM" in triangles|points) ;; *) usage ;; esac
[ -f "$VS" ] && [ -f "$FS" ] || die "no such shader: $VS or $FS"

if [ -z "$INSTRUMENT" ]; then
  # comments do not count; the language has no mediump text outside them
  if sed -e 's://.*$::' "$FS" | tr '\n' ' ' | sed -e 's:/\*[^*]*\*\+\([^/*][^*]*\*\+\)*/::g' |
       grep -Eq '\b(mediump|lowp)\b|gl_FragCoord'; then
    INSTRUMENT=shim
  else
    INSTRUMENT=pan_shc
  fi
fi

DEPS="${CONNECTED_DEPS_DIR:-}"
TARBALL_SHA=$(sed -n 's/^tarball-sha256: *//p' "$ROOT/third_party/mesa-26.2.1/UPSTREAM")
TARBALL_URL=$(sed -n 's/^tarball: *//p' "$ROOT/third_party/mesa-26.2.1/UPSTREAM")

# tarball: the Mesa 26.2.1 tarball, fetched once and checked against UPSTREAM
tarball() {
  [ -n "$DEPS" ] || die "set CONNECTED_DEPS_DIR (or MESA_PAN_TREE/PAN_SHC/MESA_SHIM_WORK) to build Mesa"
  local t="$DEPS/mesa-tarball/mesa-26.2.1.tar.xz"
  if [ ! -f "$t" ]; then
    mkdir -p "$DEPS/mesa-tarball"
    curl -fsSL -o "$t.part" "$TARBALL_URL"
    echo "$TARBALL_SHA  $t.part" | sha256sum -c - >&2 || { rm -f "$t.part"; die "tarball sha256 differs from UPSTREAM"; }
    mv "$t.part" "$t"
  fi
  echo "$t"
}

TMP="$(mktemp -d "${TMPDIR:-/tmp}/deqp_ref.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
NAME="$(basename "$OUT")"
mkdir -p "$(dirname "$OUT")"

if [ "$INSTRUMENT" = shim ]; then
  WORK="${MESA_SHIM_WORK:-${DEPS:+$DEPS/mesa-shim}}"
  [ -n "$WORK" ] || die "set MESA_SHIM_WORK or CONNECTED_DEPS_DIR"
  if [ ! -f "$WORK/inst/lib64/libpanfrost_noop_drm_shim.so" ] &&
     [ ! -f "$WORK/inst/lib/$(gcc -dumpmachine)/libpanfrost_noop_drm_shim.so" ]; then
    "$HERE/drm_shim_ref.sh" build "$(tarball)" "$WORK" >&2
  fi
  "$HERE/drm_shim_ref.sh" run "$WORK" "$VS" "$FS" "$TMP/$NAME" --prim "$PRIM" --uniform-values >&2
  # the trace field is the trace's file name, so the trace carries <name>.trace
  python3 -I "$HERE/pan_ref_diff.py" extract "$TMP/$NAME.trace" --draw > "$TMP/expect.json"
else
  TREE="${MESA_PAN_TREE:-${DEPS:+$DEPS/mesa-pan/mesa-26.2.1}}"
  [ -n "$TREE" ] || die "set MESA_PAN_TREE or CONNECTED_DEPS_DIR"
  PAN_SHC="${PAN_SHC:-$TREE/build-pan/src/panfrost/pan_shc/pan_shc}"
  if [ ! -x "$PAN_SHC" ]; then
    if [ -f "$TREE/build-pan/build.ninja" ]; then
      "$HERE/build_pan_shc.sh" --update "$TREE" >&2
    else
      "$HERE/build_pan_shc.sh" "$(tarball)" "$(dirname "$TREE")" >&2
    fi
  fi
  [ -x "$PAN_SHC" ] || die "no pan_shc at $PAN_SHC"
  export PAN_SHC
  python3 -I "$HERE/glsl2pan.py" "$VS" "$FS" --prim "$PRIM" > "$TMP/$NAME.pan_shc.json" 2> "$TMP/$NAME.disasm" ||
    { tail -5 "$TMP/$NAME.disasm" >&2; die "pan_shc failed"; }
  python3 -I - "$HERE" "$TMP/$NAME.pan_shc.json" "$NAME.pan_shc.json" > "$TMP/expect.json" <<'PY'
import json
import sys
sys.path.insert(0, sys.argv[1])
import pan_ref_diff as p
build, regions, position, sec = p.read_pan_shc(sys.argv[2])
out = {}
for name, r in regions.items():
    out[name] = {"va": r["va"], "found": True, "words": ["%016x" % w for w in r["words"]],
                 "text": [], "work_reg_count": r["work_reg_count"],
                 "preload": "0x%016x" % r["preload"],
                 "fau": [None if w is None else "%08x" % w for w in r["fau"]]}
json.dump({"trace": sys.argv[3], "secondary_offset": sec, "regions": out}, sys.stdout, indent=1)
print()
PY
fi

cp "$VS" "$OUT.vert"
cp "$FS" "$OUT.frag"
mv "$TMP/expect.json" "$OUT.expect.json"
echo "deqp_ref: $OUT.{vert,frag,expect.json} ($INSTRUMENT, $PRIM)"

#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# glsl2usc.sh - regenerate the offline shader table (src/shaderc/blobs.rs) from
# the whole tools/shaders/ starter set, and the internal framebuffer-load
# blobs in src/pvr/blobs.rs.
#
# The real tool is `yakogl-tools glsl2usc` (tools/yakogl-tools, built here with
# cargo); this script fixes the canonical invocation (the source list
# tools/shaders/precompiled.list, output paths). Pass extra .vert/.frag paths
# as args to add them to the table.
#
# Config (env vars, forwarded to the tool; defaults point at the scratchpad
# Mesa build):
#   PCO_SHC         path to the pco_shc binary
#   GLSLANG         glslangValidator (default: PATH)
#   GLSL2USC_CACHE  directory caching the compiler output (a rerun takes seconds)
#   YAKOGL_TOOLS    a prebuilt yakogl-tools binary (default: cargo run)
#   GLSL2USC_OUT    write the table here instead of src/shaderc/blobs.rs and
#                   stop there, leaving src/pvr/blobs.rs alone: the
#                   reproduction tools/check.sh compares with the committed
#                   table
#
# Usage:
#   tools/glsl2usc.sh                       # regenerate from tools/shaders/precompiled.list
#   tools/glsl2usc.sh path/to/extra.frag    # ...plus extra shaders
#   PCO_SHC=/my/pco_shc tools/glsl2usc.sh
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
SHADERS="$HERE/shaders"
OUT="${GLSL2USC_OUT:-$HERE/../src/shaderc/blobs.rs}"
PVR_BLOBS="$HERE/../src/pvr/blobs.rs"
if [ -n "${YAKOGL_TOOLS:-}" ]; then
  TOOL=("$YAKOGL_TOOLS")
else
  TOOL=(cargo run --quiet --manifest-path "$HERE/../Cargo.toml" -p yakogl-tools --)
fi

# The inputs, their blend / colour-mask / render-target variants and
# --skip-failures are the `src`, `blend`, `for` and `skip-failures` lines of
# the one source list every table is built from (its head documents the
# syntax). The inputs are in its order, which is the table's row order.
LIST="$SHADERS/precompiled.list"

"${TOOL[@]}" glsl2usc -o "$OUT" --sources "$LIST" "$@"
[ -n "${GLSL2USC_OUT:-}" ] && exit 0

# Internal shaders the PowerVR backend binds itself (not app-facing): the
# framebuffer-load background object (colour preservation across kicks).
# fbload variants for the float targets (the load quad's FS packs what it read).
# Each updates its block in src/pvr/blobs.rs in place (--after/--title: where
# and under which heading a missing block is added).
FBLOAD_FS="$SHADERS/internal/fbload.frag"
"${TOOL[@]}" usc-blob "$FBLOAD_FS" "$PVR_BLOBS" PVR_USC_FBLOAD_F16X4 replace@f16x4 \
  --after PVR_USC_FBLOAD --title "fbload.frag @f16x4: the load FS packing RGBA16F (four halves)"
"${TOOL[@]}" usc-blob "$FBLOAD_FS" "$PVR_BLOBS" PVR_USC_FBLOAD_F16 replace@f16 \
  --after PVR_USC_FBLOAD_F16X4 --title "fbload.frag @f16: the load FS packing R16F"
"${TOOL[@]}" usc-blob "$FBLOAD_FS" "$PVR_BLOBS" PVR_USC_FBLOAD_F32 replace@f32 \
  --after PVR_USC_FBLOAD_F16 --title "fbload.frag @f32: the load FS packing R32F"
"${TOOL[@]}" usc-blob "$FBLOAD_FS" "$PVR_BLOBS" PVR_USC_FBLOAD
"${TOOL[@]}" usc-blob "$SHADERS/internal/fbload.vert" "$PVR_BLOBS" PVR_USC_FBLOAD_VS

# The masked clear (glColorMask with only some channels on): clear.frag with
# the mask lowered as pco lowers it (the channels left off are read back from
# the pixel), for every partial mask M (bit 0 R .. 3 A, set = written), for
# RGBA8 and RGBA16F targets. The one-channel float targets need none: their
# channel is either written (the plain clear) or not (no colour clear).
CLEAR_FS="$SHADERS/internal/clear.frag"
RGBA=rgba
PREV=PVR_USC_FBLOAD_VS
for M in 1 2 3 4 5 6 7 8 9 10 11 12 13 14; do
  CH=""
  for b in 0 1 2 3; do
    if (( M >> b & 1 )); then CH="$CH${RGBA:b:1}"; fi
  done
  for RT in "" f16x4; do
    SYM=$(printf 'PVR_USC_CLEAR_M%X' "$M")${RT:+_F16X4}
    "${TOOL[@]}" usc-blob "$CLEAR_FS" "$PVR_BLOBS" "$SYM" "replace:$CH${RT:+@$RT}" \
      --after "$PREV" --title "clear.frag, channels $CH written${RT:+, RGBA16F}: the masked clear"
    PREV=$SYM
  done
done

#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# glsl2usc.sh - regenerate the offline shader table (src/shaderc/blobs.rs) from
# the whole tools/shaders/ starter set, and the internal framebuffer-load
# blobs in src/pvr/blobs.rs.
#
# The real tool is `yakogl-tools glsl2usc` (tools/yakogl-tools, built here with
# cargo); this script fixes the canonical invocation (input list, variants,
# output paths). Pass extra .vert/.frag paths as args to add them to the table.
#
# Config (env vars, forwarded to the tool; defaults point at the scratchpad
# Mesa build):
#   PCO_SHC         path to the pco_shc binary
#   GLSLANG         glslangValidator (default: PATH)
#   GLSL2USC_CACHE  directory caching the compiler output (a rerun takes seconds)
#   YAKOGL_TOOLS    a prebuilt yakogl-tools binary (default: cargo run)
#
# Usage:
#   tools/glsl2usc.sh                       # regenerate from tools/shaders/*
#   tools/glsl2usc.sh path/to/extra.frag    # ...plus extra shaders
#   PCO_SHC=/my/pco_shc tools/glsl2usc.sh
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
SHADERS="$HERE/shaders"
OUT="$HERE/../src/shaderc/blobs.rs"
PVR_BLOBS="$HERE/../src/pvr/blobs.rs"
if [ -n "${YAKOGL_TOOLS:-}" ]; then
  TOOL=("$YAKOGL_TOOLS")
else
  TOOL=(cargo run --quiet --manifest-path "$HERE/../Cargo.toml" -p yakogl-tools --)
fi

# Deterministic order (matches the committed blobs.rs).
INPUTS=(
  "$SHADERS/uniform_color.frag"
  "$SHADERS/passthrough.vert"
  "$SHADERS/interp_color.frag"
  "$SHADERS/color_uv.vert"
  "$SHADERS/color_uv.frag"
  "$SHADERS/mvp.vert"
  "$SHADERS/texture.frag"
  "$SHADERS/uv.vert"
  "$SHADERS/texture2.frag"
  "$SHADERS/compact.frag"
  "$SHADERS/points.vert"
  "$SHADERS/mvp_color.vert"
  "$SHADERS/zorder.vert"
  "$SHADERS/es100/matattr2.vert"
  "$SHADERS/es100/matattr3.vert"
  "$SHADERS/es100/matattr4.vert"
  "$SHADERS/es100/matattr.frag"
  "$SHADERS/es100/hello.vert"
  "$SHADERS/es100/hello.frag"
  "$SHADERS/es100/textured.vert"
  "$SHADERS/es100/textured.frag"
  "$SHADERS/es300/cube.vert"
  "$SHADERS/es300/cube.frag"
  "$SHADERS/es300/array.frag"
  "$SHADERS/es300/vid.vert"
  "$SHADERS/es300/vcolor.frag"
  "$SHADERS/es300/hdrsample.frag"
  "$SHADERS/es300/dynidx.frag"
  "$SHADERS/es300/texsize.frag"
  "$SHADERS/es300/inst.vert"
  "$SHADERS/es300/inst2.vert"
  "$SHADERS/es300/vol.frag"
  "$SHADERS/es300/facing.frag"
  # FemtoVG 0.26 (Slint GPU renderer): 1 VS + 15 FS (+14 NO_DISCARD), reproduced byte-exactly by
  # `yakogl-tools femtovg-shaders`. All are GLSL ES 1.00 with EDGE_AA on.
  "$SHADERS/femtovg/main.vert"
  "$SHADERS/femtovg/fill_color.frag"
  "$SHADERS/femtovg/fill_gradient.frag"
  "$SHADERS/femtovg/stencil.frag"
  "$SHADERS/femtovg/fill_image_gradient.frag"
  "$SHADERS/femtovg/filter_image.frag"
  "$SHADERS/femtovg/texture_copy_unclipped.frag"
  "$SHADERS/femtovg/fill_gradient_conic.frag"
  "$SHADERS/femtovg/fill_image_gradient_conic.frag"
  "$SHADERS/femtovg/fill_image.frag"
  "$SHADERS/femtovg/fill_color.glyph.frag"
  "$SHADERS/femtovg/fill_gradient.glyph.frag"
  "$SHADERS/femtovg/fill_image_gradient.glyph.frag"
  "$SHADERS/femtovg/fill_gradient_conic.glyph.frag"
  "$SHADERS/femtovg/fill_image_gradient_conic.glyph.frag"
  "$SHADERS/femtovg/fill_image.glyph.frag"
  # The same fragment shaders without the stroke-threshold discard (NO_DISCARD):
  # the no_std fork picks them for every draw whose threshold is negative (all
  # fills), which keeps those draws off the punch-through path.
  "$SHADERS/femtovg/fill_color.nodiscard.frag"
  "$SHADERS/femtovg/fill_gradient.nodiscard.frag"
  "$SHADERS/femtovg/stencil.nodiscard.frag"
  "$SHADERS/femtovg/fill_image_gradient.nodiscard.frag"
  "$SHADERS/femtovg/filter_image.nodiscard.frag"
  "$SHADERS/femtovg/fill_gradient_conic.nodiscard.frag"
  "$SHADERS/femtovg/fill_image_gradient_conic.nodiscard.frag"
  "$SHADERS/femtovg/fill_image.nodiscard.frag"
  "$SHADERS/femtovg/fill_color.glyph.nodiscard.frag"
  "$SHADERS/femtovg/fill_gradient.glyph.nodiscard.frag"
  "$SHADERS/femtovg/fill_image_gradient.glyph.nodiscard.frag"
  "$SHADERS/femtovg/fill_gradient_conic.glyph.nodiscard.frag"
  "$SHADERS/femtovg/fill_image_gradient_conic.glyph.nodiscard.frag"
  "$SHADERS/femtovg/fill_image.glyph.nodiscard.frag"
  # gl_test cases 28..33: the bisection of the FemtoVG "invisible fill" (main.vert
  # with a tiny diagnostic FS each), kept as regression tests
  "$SHADERS/fvgtest_green.frag"
  "$SHADERS/fvgtest_ftcoord.frag"
  "$SHADERS/fvgtest_inner.frag"
  "$SHADERS/fvgtest_cover.frag"
  "$SHADERS/fvgtest_discard.frag"
  "$SHADERS/fvgtest_discard2.frag"
  "$SHADERS/es300/cubelodv.vert"
  "$SHADERS/es300/cubelodv.frag"
  "$SHADERS/es300/cubelodf.vert"
  "$SHADERS/es300/cubelodf.frag"
)

# Blend / colour-mask variants precompiled for every fragment shader (both
# are lowered into the FS on this GPU): GL's usual SRC_ALPHA/ONE_MINUS_SRC_ALPHA
# (also with alpha masked off), premultiplied ONE/ONE_MINUS_SRC_ALPHA, additive
# ONE/ONE, CONSTANT_COLOR/ONE_MINUS_CONSTANT_COLOR (reads glBlendColor) and a
# plain write of the green and alpha channels only.
BLENDS=(
  --blend src_alpha,inv_src_alpha
  --blend src_alpha,inv_src_alpha:rgb
  --blend one,inv_src_alpha
  --blend one,one
  --blend const_color,inv_const_color
  --blend replace:ga
  # Colour mask with every channel off for shaders that `discard`: the GL core
  # cannot disable the ISP tag writes for those (the punch-through pass then
  # never updates depth/stencil, gl_test case 38), so it runs the shader with
  # this variant, which writes the destination back unchanged.
  --blend zero,one
  # FemtoVG composite operations (lib.rs CompositeOperationState; src==dst for
  # both channels). SourceOver (one,inv_src_alpha), Lighter (one,one) and Copy
  # (= the base shader) are already covered above.
  --blend dst_alpha,zero
  --blend inv_dst_alpha,zero
  --blend dst_alpha,inv_src_alpha
  --blend inv_dst_alpha,one
  --blend zero,src_alpha
  --blend zero,inv_src_alpha
  --blend inv_dst_alpha,src_alpha
  --blend inv_dst_alpha,inv_src_alpha
  --blend src_alpha,one
)

# Qt Quick's scene-graph shaders (qt6-for-zephyr: the GLSL ES 1.00 text
# Qt's GLES2 RHI backend hands to glShaderSource, extracted from the Stage 1
# .qsb files by zephyr-module/tools/qsb2glsl) live in shaders/qt/ and are
# picked up by name; the directory is empty until they are generated.
# Qt's scene graph blends with ONE/ONE_MINUS_SRC_ALPHA only (premultiplied
# materials; opaque ones do not blend; stencil clips mask every channel,
# which needs no variant), so the Qt fragment shaders get just that state
# instead of the FemtoVG list.  A Qt shader pco_shc cannot build (so far one
# Particles vertex shader) is left out rather than failing the whole table.
QT_OPTS=()
for f in "$SHADERS"/qt/*.vert "$SHADERS"/qt/*.frag; do
  [ -e "$f" ] && INPUTS+=("$f")
done
if [ -e "$SHADERS/qt/texture.frag" ]; then
  QT_OPTS=(--for "$SHADERS/qt/*=one,inv_src_alpha" --skip-failures)
fi
# Qt Quick 3D (shaders/qt3d/): the runtime renderer's built-in shaders from
# the Stage 1 .qsb files plus the application's materials from shadergen's
# .qsbc collection (qsb2glsl reads both).  Opaque geometry does not blend;
# transparent materials blend premultiplied like the scene graph, so the
# same one variant.  The ES 3.0-only built-ins have no GLSL 100 text and
# never reach this list.
QT3D_OPTS=()
for f in "$SHADERS"/qt3d/*.vert "$SHADERS"/qt3d/*.frag; do
  [ -e "$f" ] && INPUTS+=("$f")
done
if ls "$SHADERS"/qt3d/*.frag >/dev/null 2>&1; then
  # Float render targets (Qt Quick 3D's prefiltered probes are RGBA16F, its
  # shadow maps R16F/R32F): the colour output packed for each class.
  # --for: first matching glob wins, so the .frag rule (blend variant plus the
  # float render-target classes) must precede the catch-all.
  # one,inv_src_alpha@f16x4: Quick 3D's SourceOver (premultiplied) blended
  # into the RGBA16F target a layer with post-processing renders to
  # (qquick3dscenerenderer.cpp, postProcessingNeeded = firstEffect).
  QT3D_OPTS=(--for "$SHADERS/qt3d/*.frag=one,inv_src_alpha;replace@f16x4;replace@f16;replace@f32;one,inv_src_alpha@f16x4"
             --for "$SHADERS/qt3d/*=one,inv_src_alpha"
             --for "$SHADERS/es300/*.frag=one,inv_src_alpha;replace@f16x4;replace@f16;replace@f32;one,inv_src_alpha@f16x4" --skip-failures)
fi

"${TOOL[@]}" glsl2usc -o "$OUT" "${BLENDS[@]}" "${QT_OPTS[@]}" "${QT3D_OPTS[@]}" "${INPUTS[@]}" "$@"

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

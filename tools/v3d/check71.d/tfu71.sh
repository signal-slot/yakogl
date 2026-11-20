#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# tfu71: driver/src/v3d/v3d_tfu71.c (the V3D 7.1 TFU job's register values:
# ICFG, IIA, IIS, ICA, IUA, IOA, IOS, COEF0..3 and the 7.1-only IOC) against
# Mesa 26.2.2's own 7.1 fill, with tools/v3d/tfu71_check.c.
#
# third_party/mesa-26.2.1 vendors no TFU file (v3dx_tfu.c, v3d_tfu.h), so
# Mesa's answers are committed: tools/v3d/tfu71_golden.tsv (the constants,
# the supported data types, and the decision and twelve words of the 27
# named cases and the sweep's first 1,000 inputs). Every run compares ours
# with that table. When a Mesa 26.2.2 tree is present ($CONNECTED_MESA_26_2,
# else $CONNECTED_DEPS_DIR/mesa-26.2; the mesa-26.2.2 tag of
# https://gitlab.freedesktop.org/mesa/mesa.git), the run also builds Mesa's
# own fill from it, compares ours with it live (all 20,027 inputs), and
# requires the committed table to equal the table that fill writes, byte for
# byte. Without such a tree the run says so on one line and passes on the
# table alone: not a gap, since the table is the committed reference (as
# sched71_oracle.tsv is for the sched71_oracle check).
#
# The Mesa half, build/v3d71/tfu71/mesa_tfu71.c, is made here from the tree.
# Copied verbatim, after a check that each range still holds the expected
# line:
#   src/gallium/drivers/v3d/v3dx_tfu.c:64-66   no mipmaps into raster
#   src/gallium/drivers/v3d/v3dx_tfu.c:88-91   the data type check
#   src/gallium/drivers/v3d/v3dx_tfu.c:98      the IOS initializer
#   src/gallium/drivers/v3d/v3dx_tfu.c:106-127 IIA, IOA, IIS
#   src/gallium/drivers/v3d/v3dx_tfu.c:165-202 the V3D_VERSION >= 71 branch
#   src/gallium/drivers/v3d/v3dx_format_table.c:343-378 v3dX(tfu_supports_tex_format)
#   src/broadcom/common/v3d_tiling.h:42-64     enum v3d_tiling_mode
#   src/broadcom/common/v3d_tiling.c:55-70     v3d_utile_height
# with Mesa's v3d_tfu.h and include/drm-uapi/v3d_drm.h included as they are,
# and enum V3D71_Texture_Data_Formats out of Mesa's gen_pack_header.py run on
# the tree's v3d_packet.xml for 71 (the command of src/broadcom/cle/meson.build).
# The shim stands in for the Gallium resource only: bo->offset is the
# address, v3d_layer_offset is 0, base_level 0 and last_level = num_mm.
# Built -std=gnu11 -Wall -Wextra -Werror (the Mesa half with -DNDEBUG: its
# assert(for_mipmap) at v3dx_tfu.c:89 is the Gallium caller's contract, and
# the check also asks about types that caller never passes).
#
# Usage: tfu71.sh            check
#        tfu71.sh --update   rewrite tools/v3d/tfu71_golden.tsv from the Mesa
#                            tree (after a change of the check's inputs;
#                            commit the table in the same commit)
#
# Exit 0 with "tfu71: N of N checks, 0 failed"; 1 on a difference (or a
# Mesa 26.2.2 tree whose cited lines moved); 77 with "gap: CC" (no C
# compiler); --update without a usable Mesa tree (none, not 26.2.2, or no
# python3 for Mesa's generator) exits 77 with "gap: CONNECTED_MESA_26_2".
# Convention: tools/v3d/check71.d/README.md.
set -u
cd "$(dirname "$0")/../../.." || exit 1

name=tfu71
dir=build/v3d71/$name
mkdir -p "$dir" || exit 1

cc=${CC:-cc}
if ! command -v "$cc" >/dev/null 2>&1; then
  echo "gap: CC (no C compiler '$cc')"
  exit 77
fi
update=0
[ "${1:-}" = "--update" ] && update=1
table=tools/v3d/tfu71_golden.tsv
flags=(-std=gnu11 -O1 -g -Wall -Wextra -Werror)

if [ -n "${CONNECTED_MESA_26_2:-}" ]; then
  m=$CONNECTED_MESA_26_2
elif [ -n "${CONNECTED_DEPS_DIR:-}" ]; then
  m=$CONNECTED_DEPS_DIR/mesa-26.2
else
  m=
fi
why=
if [ -z "$m" ] || [ ! -f "$m/src/gallium/drivers/v3d/v3dx_tfu.c" ]; then
  why="no Mesa 26.2.2 tree at '${m:-<unset>}' (git clone --depth 1 --branch mesa-26.2.2 https://gitlab.freedesktop.org/mesa/mesa.git \"\$CONNECTED_DEPS_DIR/mesa-26.2\")"
elif [ "$(cat "$m/VERSION" 2>/dev/null)" != "26.2.2" ]; then
  why="the Mesa tree at $m is '$(cat "$m/VERSION" 2>/dev/null)', not 26.2.2 (the line citations are of 26.2.2)"
elif ! command -v python3 >/dev/null 2>&1; then
  why="python3 is not installed; Mesa's gen_pack_header.py makes the 7.1 data types"
fi

if [ -n "$why" ]; then
  if [ "$update" = 1 ]; then
    echo "$why"
    echo "gap: CONNECTED_MESA_26_2 (--update needs a Mesa 26.2.2 tree)"
    exit 77
  fi
  # the committed table alone
  "$cc" "${flags[@]}" -Idriver/src/v3d -c driver/src/v3d/v3d_tfu71.c -o "$dir/v3d_tfu71.o" \
    >"$dir/build.log" 2>&1 &&
    "$cc" "${flags[@]}" -Idriver/src/v3d tools/v3d/tfu71_check.c "$dir/v3d_tfu71.o" \
      -o "$dir/tfu71_check" >>"$dir/build.log" 2>&1 || {
    tail -40 "$dir/build.log"
    echo "tfu71: build failed, 1 failed"
    exit 1
  }
  echo "tfu71: against the committed $table only: $why"
  exec "$dir/tfu71_check" "$table"
fi

tfu=$m/src/gallium/drivers/v3d/v3dx_tfu.c
fmt=$m/src/gallium/drivers/v3d/v3dx_format_table.c
tilh=$m/src/broadcom/common/v3d_tiling.h
tilc=$m/src/broadcom/common/v3d_tiling.c
cle=$m/src/broadcom/cle

# expect FILE LINE TEXT: the line holds TEXT (a fixed string)
bad=0
expect() {
  if ! sed -n "$2p" "$1" | grep -qF -- "$3"; then
    echo "$1:$2 does not hold '$3' (the citations need an update)"
    bad=1
  fi
}
expect "$tfu" 64 "TFU requires UIF when generating mipmaps"
expect "$tfu" 66 "return false;"
expect "$tfu" 88 "tfu_supports_tex_format)(tex_format, for_mipmap)"
expect "$tfu" 91 "}"
expect "$tfu" 98 ".ios = (height << 16) | width,"
expect "$tfu" 106 "uint32_t src_offset"
expect "$tfu" 112 "tfu.ioa |= dst_offset;"
expect "$tfu" 114 "switch (src_base_slice->tiling)"
expect "$tfu" 127 "}"
expect "$tfu" 165 "#if V3D_VERSION >= 71"
expect "$tfu" 173 "V3D71_TFU_ICFG_OTYPE_SHIFT"
expect "$tfu" 176 "V3D71_TFU_IOC_DIMTW"
expect "$tfu" 201 "V3D71_TFU_IOC_NUMMM_SHIFT"
expect "$tfu" 202 "#endif"
expect "$fmt" 343 "bool"
expect "$fmt" 344 "v3dX(tfu_supports_tex_format)"
expect "$fmt" 378 "}"
expect "$tilh" 42 "enum v3d_tiling_mode {"
expect "$tilh" 64 "};"
expect "$tilc" 55 "uint32_t"
expect "$tilc" 56 "v3d_utile_height(int cpp)"
expect "$tilc" 70 "}"
[ "$bad" = 0 ] || {
  echo "tfu71: the Mesa tree's lines moved, 1 failed"
  exit 1
}

# Mesa's 7.1 data types, from Mesa's generator (no .pyc into the tree)
python3 -I -B "$cle/gen_pack_header.py" "$cle/v3d_packet.xml" 71 \
  >"$dir/v3d_packet_v71_pack.h" 2>"$dir/gen.log" || {
  cat "$dir/gen.log"
  echo "tfu71: gen_pack_header.py failed, 1 failed"
  exit 1
}
sed -n '/^enum V3D71_Texture_Data_Formats {/,/^};/p' "$dir/v3d_packet_v71_pack.h" >"$dir/tex_formats.h"
grep -q 'TEXTURE_DATA_FORMAT_RGBA8 ' "$dir/tex_formats.h" || {
  echo "tfu71: no enum V3D71_Texture_Data_Formats in the generator's output, 1 failed"
  exit 1
}

s=$dir/mesa_tfu71.c
{
  cat <<'EOF'
/* Generated by tools/v3d/check71.d/tfu71.sh from the Mesa 26.2.2 tree: the
 * marked ranges are Mesa's own lines (MIT, Copyright Raspberry Pi Ltd,
 * Broadcom), the rest is the shim. */
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <assert.h>
#include "drm-uapi/v3d_drm.h"
#include "broadcom/common/v3d_tfu.h"

#define V3D_VERSION 71
#define V3DX(x) V3D71_##x
#define v3dX(x) v3d71_##x
#define UNREACHABLE(s) abort()

EOF
  echo "/* gen_pack_header.py v3d_packet.xml 71: enum V3D71_Texture_Data_Formats */"
  cat "$dir/tex_formats.h"
  echo "/* src/broadcom/common/v3d_tiling.h:42-64 */"
  sed -n '42,64p' "$tilh"
  echo "/* src/broadcom/common/v3d_tiling.c:55-70 */"
  echo "uint32_t v3d_utile_height(int cpp);"
  sed -n '55,70p' "$tilc"
  echo "/* src/gallium/drivers/v3d/v3dx_format_table.c:343-378 */"
  echo "bool v3dX(tfu_supports_tex_format)(enum V3DX(Texture_Data_Formats) tex_format, bool for_mipmap);"
  sed -n '343,378p' "$fmt"
  cat <<'EOF'

struct tfu71_in {
	uint32_t ttype, cpp, width, height, num_mm;
	uint32_t src_addr, src_tiling, src_sp;
	uint32_t dst_addr, dst_tiling, dst_sp;
};
struct mesa_tfu71_out {
	uint32_t icfg, iia, iis, ica, iua, ioa, ios, coef[4], ioc;
};
int mesa_tfu71(const struct tfu71_in *in, struct mesa_tfu71_out *out);
int mesa_tfu71_supports(uint32_t ttype, int for_mipmap);

/* the shim of the Gallium resource: v3d_resource, v3d_bo, v3d_resource_slice */
struct shim_bo {
	uint32_t offset;
};
struct shim_resource {
	struct shim_bo *bo;
	int cpp;
};
struct shim_slice {
	enum v3d_tiling_mode tiling;
	uint32_t padded_height;
	uint32_t stride;
};
static uint32_t v3d_layer_offset(const void *prsc, unsigned level, unsigned layer)
{
	(void)prsc;
	(void)level;
	(void)layer;
	return 0;
}

int mesa_tfu71_supports(uint32_t ttype, int for_mipmap)
{
	return v3dX(tfu_supports_tex_format)((enum V3DX(Texture_Data_Formats))ttype, for_mipmap != 0);
}

static int fill(const struct tfu71_in *in, struct drm_v3d_submit_tfu *out)
{
	struct shim_bo sbo = { in->src_addr }, dbo = { in->dst_addr };
	struct shim_resource src_r = { &sbo, (int)in->cpp }, dst_r = { &dbo, (int)in->cpp };
	struct shim_resource *src = &src_r, *dst = &dst_r;
	struct shim_slice ss = { (enum v3d_tiling_mode)in->src_tiling, in->src_sp, in->src_sp };
	struct shim_slice ds = { (enum v3d_tiling_mode)in->dst_tiling, in->dst_sp, in->dst_sp };
	struct shim_slice *src_base_slice = &ss, *dst_base_slice = &ds;
	const void *psrc = 0, *pdst = 0;
	unsigned int src_level = 0, src_layer = 0, dst_layer = 0;
	unsigned int base_level = 0, last_level = in->num_mm;
	bool for_mipmap = in->num_mm != 0;
	int width = (int)in->width, height = (int)in->height;
	enum V3DX(Texture_Data_Formats) tex_format = (enum V3DX(Texture_Data_Formats))in->ttype;

EOF
  echo "/* src/gallium/drivers/v3d/v3dx_tfu.c:64-66 */"
  sed -n '64,66p' "$tfu"
  echo "/* src/gallium/drivers/v3d/v3dx_tfu.c:88-91 */"
  sed -n '88,91p' "$tfu"
  echo "struct drm_v3d_submit_tfu tfu = {"
  echo "/* src/gallium/drivers/v3d/v3dx_tfu.c:98 */"
  sed -n '98p' "$tfu"
  echo "};"
  echo "/* src/gallium/drivers/v3d/v3dx_tfu.c:106-127 */"
  sed -n '106,127p' "$tfu"
  echo "/* src/gallium/drivers/v3d/v3dx_tfu.c:165-202 */"
  sed -n '165,202p' "$tfu"
  cat <<'EOF'
	*out = tfu;
	return 1;
}

int mesa_tfu71(const struct tfu71_in *in, struct mesa_tfu71_out *out)
{
	struct drm_v3d_submit_tfu t;
	if (!fill(in, &t))
		return 0;
	out->icfg = t.icfg;
	out->iia = t.iia;
	out->iis = t.iis;
	out->ica = t.ica;
	out->iua = t.iua;
	out->ioa = t.ioa;
	out->ios = t.ios;
	for (int i = 0; i < 4; i++)
		out->coef[i] = t.coef[i];
	out->ioc = t.v71.ioc;
	return 1;
}

const uint32_t mesa_tfu71_consts[] = {
	V3D71_TFU_IOC_DIMTW,
	V3D71_TFU_IOC_FORMAT_SHIFT,
	V3D71_TFU_IOC_FORMAT_RASTER,
	V3D71_TFU_IOC_FORMAT_LINEARTILE,
	V3D71_TFU_IOC_STRIDE_SHIFT,
	V3D71_TFU_IOC_NUMMM_SHIFT,
	V3D71_TFU_ICFG_OTYPE_SHIFT,
	V3D71_TFU_ICFG_IFORMAT_SHIFT,
	V3D71_TFU_ICFG_FORMAT_RASTER,
	V3D71_TFU_ICFG_FORMAT_LINEARTILE,
	V3D_TFU_MAX_DIM,
	TEXTURE_DATA_FORMAT_R8,
	TEXTURE_DATA_FORMAT_RGBA8,
	TEXTURE_DATA_FORMAT_RGB565,
	V3D_TILING_RASTER,
	V3D_TILING_LINEARTILE,
	V3D_TILING_UBLINEAR_1_COLUMN,
	V3D_TILING_UBLINEAR_2_COLUMN,
	V3D_TILING_UIF_NO_XOR,
	V3D_TILING_UIF_XOR,
};
const char *const mesa_tfu71_const_names[] = {
	"V3D71_TFU_IOC_DIMTW", "V3D71_TFU_IOC_FORMAT_SHIFT", "V3D71_TFU_IOC_FORMAT_RASTER",
	"V3D71_TFU_IOC_FORMAT_LINEARTILE", "V3D71_TFU_IOC_STRIDE_SHIFT", "V3D71_TFU_IOC_NUMMM_SHIFT",
	"V3D71_TFU_ICFG_OTYPE_SHIFT", "V3D71_TFU_ICFG_IFORMAT_SHIFT", "V3D71_TFU_ICFG_FORMAT_RASTER",
	"V3D71_TFU_ICFG_FORMAT_LINEARTILE", "V3D_TFU_MAX_DIM", "TEXTURE_DATA_FORMAT_R8",
	"TEXTURE_DATA_FORMAT_RGBA8", "TEXTURE_DATA_FORMAT_RGB565", "V3D_TILING_RASTER",
	"V3D_TILING_LINEARTILE", "V3D_TILING_UBLINEAR_1_COLUMN", "V3D_TILING_UBLINEAR_2_COLUMN",
	"V3D_TILING_UIF_NO_XOR", "V3D_TILING_UIF_XOR",
};
const unsigned mesa_tfu71_nconsts = sizeof(mesa_tfu71_consts) / sizeof(mesa_tfu71_consts[0]);
EOF
} >"$s"

"$cc" "${flags[@]}" -DNDEBUG -I"$m/include" -I"$m/src" -c "$s" -o "$dir/mesa_tfu71.o" \
  >"$dir/build.log" 2>&1 &&
  "$cc" "${flags[@]}" -Idriver/src/v3d -c driver/src/v3d/v3d_tfu71.c -o "$dir/v3d_tfu71.o" \
    >>"$dir/build.log" 2>&1 &&
  "$cc" "${flags[@]}" -DTFU71_MESA -Idriver/src/v3d tools/v3d/tfu71_check.c "$dir/v3d_tfu71.o" \
    "$dir/mesa_tfu71.o" -o "$dir/tfu71_check" >>"$dir/build.log" 2>&1 || {
  tail -40 "$dir/build.log"
  echo "tfu71: build failed, 1 failed"
  exit 1
}
# Mesa's answers as the table, then the committed table against them
"$dir/tfu71_check" --emit "$dir/tfu71_golden.tsv" || {
  echo "tfu71: --emit failed, 1 failed"
  exit 1
}
if [ "$update" = 1 ]; then
  cp "$dir/tfu71_golden.tsv" "$table" || exit 1
  echo "tfu71: wrote $table from Mesa 26.2.2 at $m"
fi
if ! cmp -s "$dir/tfu71_golden.tsv" "$table"; then
  diff -u "$table" "$dir/tfu71_golden.tsv" | head -20
  echo "tfu71: $table differs from Mesa 26.2.2's fill at $m (tfu71.sh --update rewrites it), 1 failed"
  exit 1
fi
echo "tfu71: $table equals Mesa 26.2.2's fill at $m"
exec "$dir/tfu71_check" "$table"

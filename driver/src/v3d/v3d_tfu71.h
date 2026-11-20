/* SPDX-License-Identifier: MIT */
/*
 * V3D 7.1 (BCM2712, Raspberry Pi 5) texture formatting unit (TFU): the
 * register values of one TFU job, packed as Mesa 26.2.2's 7.1 fill does.
 * The 4.2 job is v3d_tex.h's v3d_tex_tfu_pack; this file is 7.1 only and
 * changes nothing at 4.2.
 *
 * After Mesa 26.2.2 (MIT), cited as file:line of the mesa-26.2.2 tag:
 *   src/gallium/drivers/v3d/v3dx_tfu.c        v3dX(tfu), the V3D_VERSION >= 71
 *                                             branch (the values)
 *   src/broadcom/common/v3d_tfu.h             the V3D71_TFU_* field positions
 *   src/gallium/drivers/v3d/v3dx_format_table.c
 *                                             v3dX(tfu_supports_tex_format)
 *   src/broadcom/common/v3d_tiling.h, .c      enum v3d_tiling_mode, v3d_utile_height
 *   include/drm-uapi/v3d_drm.h                struct drm_v3d_submit_tfu (v71.ioc)
 *   src/broadcom/simulator/v3dx_simulator.c   the register write order
 *
 *   Copyright (C) 2021-2022 Raspberry Pi Ltd
 *
 * The values only: no register is written here (the 7.1 kick belongs to the
 * backend table, roadmap P9). Checked against Mesa's own fill code by
 * tools/v3d/check71.d/tfu71.sh (tools/v3d/tfu71_check.c).
 */
#ifndef V3D_TFU71_H
#define V3D_TFU71_H

#include <stdbool.h>
#include <stdint.h>

/* Mesa's enum v3d_tiling_mode, in its order (v3d_tiling.h:42-64): the
 * format fields are LINEARTILE's code plus the distance from LINEARTILE
 * (v3dx_tfu.c:169-171, 181-183). */
enum v3d_tfu71_tiling {
	V3D_TFU71_TILING_RASTER = 0,       /* v3d_tiling.h:44 */
	V3D_TFU71_TILING_LINEARTILE,       /* v3d_tiling.h:47 */
	V3D_TFU71_TILING_UBLINEAR_1_COLUMN, /* v3d_tiling.h:50 */
	V3D_TFU71_TILING_UBLINEAR_2_COLUMN, /* v3d_tiling.h:53 */
	V3D_TFU71_TILING_UIF_NO_XOR,       /* v3d_tiling.h:58 */
	V3D_TFU71_TILING_UIF_XOR,          /* v3d_tiling.h:63 */
};

/* Field positions and codes (v3d_tfu.h). */
#define V3D_TFU71_MAX_DIM 16384u                 /* v3d_tfu.h:30 */
#define V3D_TFU71_ICFG_OTYPE_SHIFT 16u           /* v3d_tfu.h:69 */
#define V3D_TFU71_ICFG_IFORMAT_SHIFT 23u         /* v3d_tfu.h:70 */
#define V3D_TFU71_ICFG_FORMAT_RASTER 0u          /* v3d_tfu.h:71 */
#define V3D_TFU71_ICFG_FORMAT_LINEARTILE 3u      /* v3d_tfu.h:74 */
#define V3D_TFU71_IOC_DIMTW (1u << 0)            /* v3d_tfu.h:57 */
#define V3D_TFU71_IOC_NUMMM_SHIFT 4u             /* v3d_tfu.h:67 */
#define V3D_TFU71_IOC_FORMAT_SHIFT 12u           /* v3d_tfu.h:58 */
#define V3D_TFU71_IOC_FORMAT_RASTER 0u           /* v3d_tfu.h:60 */
#define V3D_TFU71_IOC_FORMAT_LINEARTILE 3u       /* v3d_tfu.h:61 */
#define V3D_TFU71_IOC_STRIDE_SHIFT 16u           /* v3d_tfu.h:66 */

/* Texture data types (the OTYPE field) of the cases checked first; the
 * numbers are the 7.1 "Texture Data Formats" of Mesa's
 * src/broadcom/cle/v3d_packet.xml:333-339 (enum V3D71_Texture_Data_Formats).
 * Any type v3d_tfu71_supports() accepts may be packed. */
#define V3D_TFU71_TTYPE_R8 0u     /* v3d_packet.xml:333 */
#define V3D_TFU71_TTYPE_RGBA8 4u  /* v3d_packet.xml:337 */
#define V3D_TFU71_TTYPE_RGB565 6u /* v3d_packet.xml:339 */

/* One 7.1 TFU job: the fields of struct drm_v3d_submit_tfu that reach the
 * registers (v3d_drm.h:317-347; ioc is the 7.1-only v71.ioc,
 * v3d_drm.h:343-346). Mesa's simulator writes them in the order IIA, IIS,
 * ICA, IUA, IOA, IOC, IOS, COEF0..3, then ICFG, which starts the job
 * (v3dx_simulator.c:185-199). */
struct v3d_tfu71_job {
	uint32_t icfg, iia, iis, ica, iua, ioa, ios;
	uint32_t coef[4];
	uint32_t ioc;
};

/* One side of the conversion. `stride_or_padded_height` is what Mesa passes
 * as the slice's stride (bytes per row, raster) or padded height (rows, UIF);
 * it is unused for LINEARTILE and UB-linear (v3dx_tfu.c:114-127, 186-199). */
struct v3d_tfu71_surface {
	uint32_t addr; /* GPU address of the (base) level, Mesa's bo->offset + v3d_layer_offset */
	enum v3d_tfu71_tiling tiling;
	uint32_t stride_or_padded_height;
};

/* The job's inputs: Mesa's v3dX(tfu) with one format on both sides
 * (v3dx_tfu.c:51), base_level's size and `num_mm` = last_level -
 * base_level levels after it (0: a plain copy; > 0: for_mipmap, the levels
 * after the base generated and the base not written, DIMTW). */
struct v3d_tfu71_args {
	uint32_t ttype; /* texture data type (V3D71_Texture_Data_Formats) */
	uint32_t cpp;   /* bytes per texel of that type: 1, 2, 4, 8 or 16 */
	uint32_t width, height;
	uint32_t num_mm;
	struct v3d_tfu71_surface src, dst;
};

/* v3dX(tfu_supports_tex_format) (v3dx_format_table.c:343-378): may the TFU
 * convert `ttype`, and generate mipmaps of it? */
bool v3d_tfu71_supports(uint32_t ttype, bool for_mipmap);

/* Pack the job. False, with `j` untouched, where Mesa's v3dX(tfu) declines
 * (an unsupported type, v3dx_tfu.c:88-91; mipmaps into a raster
 * destination, v3dx_tfu.c:64-66) and where a value would not fit its field
 * or would not describe the image: a cpp other than 1/2/4/8/16
 * (v3d_tiling.c:55-70 knows no other), a width or height of 0 or above
 * V3D_TFU71_MAX_DIM, more levels than the base's larger side halves to 1
 * (log2), a raster stride that is not whole texels or is shorter than a
 * row, a UIF padded height that is not whole UIF blocks or is lower than
 * the image, and an IOC stride above 16 bits. */
bool v3d_tfu71_pack(const struct v3d_tfu71_args *a, struct v3d_tfu71_job *j);

#endif

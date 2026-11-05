/* SPDX-License-Identifier: MIT */
#ifndef TEX_CHECK_MESA_FMT_H
#define TEX_CHECK_MESA_FMT_H

#include <stdbool.h>
#include <stdint.h>

/* Mesa's row for one format name (tex_check_mesa_fmt.c). */
struct mesa_fmt {
	bool in_table;   /* Mesa's V3D 4.2 table has a texture type for it */
	uint32_t type;   /* TEXTURE_DATA_FORMAT_* */
	uint32_t cpp;    /* util_format_get_blocksize */
	uint32_t block_w, block_h; /* util_format_get_blockwidth/height */
	bool srgb;       /* util_format_is_srgb */
	uint8_t swizzle[4];
	uint32_t return_size;
};

/* False when the name is not one tex_check knows. */
bool mesa_v3d_format(const char *name, struct mesa_fmt *out);
/* Mesa's block count of a level (util_format_get_nblocksx/y of the
 * minified size); false when the name is not one tex_check knows. */
bool mesa_v3d_level_blocks(const char *name, uint32_t w0, uint32_t h0, uint32_t level, uint32_t *bx,
			   uint32_t *by);

#endif

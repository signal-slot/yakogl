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
	uint8_t swizzle[4];
	uint32_t return_size;
};

/* False when the name is not one tex_check knows. */
bool mesa_v3d_format(const char *name, struct mesa_fmt *out);

#endif

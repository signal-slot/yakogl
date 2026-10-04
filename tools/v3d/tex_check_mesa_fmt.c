/* SPDX-License-Identifier: MIT
 *
 * tex_check_mesa_fmt: Mesa's answer for one source format, for tex_check.c.
 * A separate file because Mesa's format table header needs the gallium v3d
 * context headers, which tex_check.c's own packet includes clash with. It
 * is compiled with Mesa's flags for libv3d-v42 (tex_check.c's header has the
 * command) and linked with Mesa's own v3dx_format_table.c (V3D_VERSION=42)
 * and libbroadcom_v3d.a (v3d_translate_pipe_swizzle).
 */
#include <stdbool.h>
#include <stdint.h>
#include <string.h>

#include "util/format/u_format.h"
#include "broadcom/common/v3d_util.h"
#include "v3d_context.h"
#include "v3d_format_table.h"

#include "tex_check_mesa_fmt.h"

/* The pipe format of each name tex_check asks about: the same memory layout
 * as the GL format/type pair v3d_tex.h names for it. */
static enum pipe_format pipe_of(const char *name)
{
	static const struct {
		const char *name;
		enum pipe_format f;
	} map[] = {
		{ "RGBA8", PIPE_FORMAT_R8G8B8A8_UNORM }, { "BGRA8", PIPE_FORMAT_B8G8R8A8_UNORM },
		{ "RGB8", PIPE_FORMAT_R8G8B8_UNORM },    { "L8", PIPE_FORMAT_L8_UNORM },
		{ "A8", PIPE_FORMAT_A8_UNORM },          { "L8A8", PIPE_FORMAT_L8A8_UNORM },
		{ "RGB565", PIPE_FORMAT_B5G6R5_UNORM },  { "RGBA4", PIPE_FORMAT_A4B4G4R4_UNORM },
		{ "RGB5_A1", PIPE_FORMAT_A1B5G5R5_UNORM }, { "R32F", PIPE_FORMAT_R32_FLOAT },
	};
	for (unsigned i = 0; i < sizeof(map) / sizeof(map[0]); i++)
		if (strcmp(map[i].name, name) == 0)
			return map[i].f;
	return PIPE_FORMAT_NONE;
}

bool mesa_v3d_format(const char *name, struct mesa_fmt *out)
{
	enum pipe_format pf = pipe_of(name);
	memset(out, 0, sizeof(*out));
	if (pf == PIPE_FORMAT_NONE)
		return false;
	out->cpp = util_format_get_blocksize(pf);
	const struct v3d_format *vf = v3d42_get_format_desc(pf);
	/* the table has a row only for a format it can sample (every row's
	 * texture column names a real type) */
	if (!vf)
		return true; /* in_table stays false */
	out->in_table = true;
	out->type = vf->tex_type;
	for (int i = 0; i < 4; i++)
		out->swizzle[i] = v3d_translate_pipe_swizzle(vf->swizzle[i]);
	out->return_size = vf->return_size;
	return true;
}

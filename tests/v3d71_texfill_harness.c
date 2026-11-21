/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Test-only harness of tests/v3d71_texfill.rs: reads one struct
 * v3d_tex71_state and one struct v3d_tex71_sampler from stdin (the raw bytes
 * of the Rust mirrors in src/v3d/ffi71.rs), packs them with the driver's
 * v3d_tex71.c, writes the two 24-byte records to stdout. Exit 0 on success,
 * 2 for a short read, 3 when a packer refuses. */
#include <stdio.h>

#include "v3d_tex71.h"

int main(void)
{
	struct v3d_tex71_state s;
	struct v3d_tex71_sampler m;
	unsigned char out[V3D_TEX71_STATE_BYTES + V3D_TEX71_SAMPLER_BYTES];

	if (fread(&s, sizeof s, 1, stdin) != 1 || fread(&m, sizeof m, 1, stdin) != 1)
		return 2;
	if (!v3d_tex71_pack_state(out, &s) || !v3d_tex71_pack_sampler(out + V3D_TEX71_STATE_BYTES, &m))
		return 3;
	return fwrite(out, sizeof out, 1, stdout) == 1 ? 0 : 4;
}

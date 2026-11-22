/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Test-only harness of tests/v3d71_mip.rs: reads struct v3d_tfu71_args values
 * from stdin until EOF (the raw bytes of the Rust mirror V3dTfu71Args in
 * src/v3d/ffi71.rs), packs each with the driver's v3d_tfu71.c, writes the
 * twelve words of each job (icfg iia iis ica iua ioa ios coef0..3 ioc) as
 * native-endian u32 to stdout. Exit 0 on success, 3 when the packer refuses. */
#include <stdio.h>

#include "v3d_tfu71.h"

int main(void)
{
	struct v3d_tfu71_args a;

	while (fread(&a, sizeof a, 1, stdin) == 1) {
		struct v3d_tfu71_job j;
		uint32_t w[12];

		if (!v3d_tfu71_pack(&a, &j))
			return 3;
		w[0] = j.icfg;
		w[1] = j.iia;
		w[2] = j.iis;
		w[3] = j.ica;
		w[4] = j.iua;
		w[5] = j.ioa;
		w[6] = j.ios;
		w[7] = j.coef[0];
		w[8] = j.coef[1];
		w[9] = j.coef[2];
		w[10] = j.coef[3];
		w[11] = j.ioc;
		if (fwrite(w, sizeof w, 1, stdout) != 1)
			return 4;
	}
	return 0;
}

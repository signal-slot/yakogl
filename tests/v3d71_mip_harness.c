/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Test-only harness of tests/v3d71_mip.rs: reads struct v3d_tfu71_args
 * records from stdin until EOF (the raw bytes of the Rust mirror in
 * src/v3d/ffi71.rs), packs each with the driver's v3d_tfu71.c and writes,
 * per record, one byte (1 packed, 0 refused) and the struct v3d_tfu71_job
 * (zeroes when refused). Exit 0 at EOF, 2 for a short read, 4 for a short
 * write. */
#include <stdio.h>
#include <string.h>

#include "v3d_tfu71.h"

int main(void)
{
	struct v3d_tfu71_args a;
	size_t n;

	while ((n = fread(&a, 1, sizeof a, stdin)) == sizeof a) {
		struct v3d_tfu71_job j;
		unsigned char ok;

		memset(&j, 0, sizeof j);
		ok = v3d_tfu71_pack(&a, &j) ? 1 : 0;
		if (fwrite(&ok, 1, 1, stdout) != 1 || fwrite(&j, sizeof j, 1, stdout) != 1)
			return 4;
	}
	return n == 0 ? 0 : 2;
}

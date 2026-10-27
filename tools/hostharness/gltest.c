/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gltest.c - harness program: the driver repository's on-silicon test suite
 * (tools/pvrlab/gl_test.c, compiled next to this file by run.sh) through the
 * library with the recording OSAL.
 *
 * usage: gltest <case> [debug]   (case 7 = gl_test's "all")
 *   HH_PRETRANSFORM=1 / HH_NOCOALESCE=1 select gl_test's options.
 *   HH_LIST=17,19 walks that case list instead of <case>, the way a
 *   board's sweep list does (gl_test_set_list).
 */
#include <stdio.h>
#include <stdlib.h>
#include "hh.h"

int gl_test_run(int gl_case, int pretransform, unsigned int ppp_ctrl, int debug, int no_coalesce, int panel);
int gl_test_set_list(const int *cases, unsigned n, unsigned *stored);

int main(int argc, char **argv)
{
	int gl_case = argc > 1 ? atoi(argv[1]) : 7;
	int debug = argc > 2 ? atoi(argv[2]) : 0;
	int rc;

	const char *list = getenv("HH_LIST");

	if (list) {
		static int cases[256];
		unsigned n = 0;
		char *end;

		for (const char *p = list; *p && n < 256; p = *end ? end + 1 : end) {
			cases[n++] = (int)strtol(p, &end, 10);
			if (end == p)
				break;
		}
		gl_case = gl_test_set_list(cases, n, NULL);
	}
	hh_install();
	rc = gl_test_run(gl_case, getenv("HH_PRETRANSFORM") != NULL, 0, debug, getenv("HH_NOCOALESCE") != NULL, 0);
	printf("gl_test rc=%d errors=%u\n", rc, hh_errors());
	return 0;
}

/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* gl_test's set walker count (set_tally_begin / set_tally_case): a by-name
 * skip (-3, from the walker's own filter or from the case, as case 16 does
 * without a 4x config) is out of the cases run and into the skips, never a
 * failure; the first two failures are named; a timeout and "no such case"
 * fail. These counts are what the i.MX8MP's CASE_SET record carries
 * (gc_gl_test main.c gc_result_set), where c4ccc966 recorded 47 / 49 with
 * case 16's skip counted as the first failure.
 *
 * It includes gl_test.c itself (GL_TEST_HOST), so it runs the real walker
 * count; the GL/EGL/OSAL entry points are never called, so the link ignores
 * them. tools/check.sh runs it ("run     gl_test set walker count"):
 *   mkdir -p build && cc -std=gnu11 -Wall -Wextra -Werror -DGL_TEST_HOST=1 -Iinclude -Isrc -Isrc/osal \
 *     -Idriver/tools/pvrlab -Idriver/src tests/gltest_set_tally.c -o build/gltest_set_tally \
 *     -no-pie -Wl,--unresolved-symbols=ignore-all && build/gltest_set_tally
 */
#include "../driver/tools/pvrlab/gl_test.c"

static struct yakogl_backend fake;
const struct yakogl_backend *g_backend = &fake;

/* the checks below, so a run that stops early cannot pass */
#define PLANNED 17

static unsigned run, failed;

static void expect(const char *what, long got, long want)
{
	printf("%-58s got %ld want %ld %s\n", what, got, want, got == want ? "ok" : "FAIL");
	run++;
	failed += got != want;
}

int main(void)
{
	/* the i.MX8MP default set's shape: 49 listed, case 16 skips itself,
	 * one other case (44 here) fails */
	set_tally_begin(49);
	for (unsigned i = 0; i < 49; i++) {
		int c = i == 23 ? 16 : i == 34 ? 44 : (int)i + 100;

		set_tally_case(c, i, c == 16 ? -3 : c == 44 ? -1 : 0);
	}
	expect("default set: passed", gl_test_passed, 47);
	expect("default set: run (the skip is not run)", gl_test_total, 48);
	expect("default set: skipped", gl_test_skipped, 1);
	expect("default set: first skip", gl_test_first_skip, 16);
	expect("default set: failed", s_set_fails, 1);
	expect("default set: first failure (not the skip)", gl_test_first_fail, 44);
	expect("default set: its index", gl_test_first_fail_at, 34);
	expect("default set: no second failure", gl_test_second_fail, -1);

	/* a clean set with a skip passes: passed == run */
	set_tally_begin(3);
	set_tally_case(1, 0, 0);
	set_tally_case(16, 1, -3);
	set_tally_case(2, 2, 0);
	expect("clean set with a skip: passed == run", gl_test_passed == gl_test_total, 1);
	expect("clean set with a skip: no failure", s_set_fails, 0);

	/* failures of every kind, the first two named, and the counts reset */
	set_tally_begin(5);
	set_tally_case(10, 0, -4);
	set_tally_case(2, 1, -1);
	set_tally_case(99, 2, -2);
	set_tally_case(17, 3, -3);
	set_tally_case(18, 4, -3);
	expect("failures: a timeout, a fail and no such case all fail", s_set_fails, 3);
	expect("failures: timed out", s_set_timed_out, 1);
	expect("failures: first", gl_test_first_fail, 10);
	expect("failures: second", gl_test_second_fail, 2);
	expect("failures: run", gl_test_total, 3);
	expect("failures: skipped (two, the first named)", gl_test_skipped * 100 + gl_test_first_skip, 217);
	expect("failures: passed (reset from the last list)", gl_test_passed, 0);
	printf("gltest_set_tally: %u checks run (planned %u), %u failed\n", run, PLANNED, failed);
	return failed || run != PLANNED;
}

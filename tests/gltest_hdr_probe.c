/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* gl_test case 44's GC7000 layout probe (hdr_layout_probe, #6 item 3): the
 * slot codes it records in the i.MX8MP's CASE record, worked on stored
 * pictures of the three layouts the one board datum allows -- every pixel in
 * place, the 16-byte halves of each 32-byte group swapped (pixel x stored at
 * x ^ 2), and every row shifted two pixels right -- so a record can be
 * decoded against this file. It also ties the decode to that datum: the
 * colour step 2 gives pixel (5,0), sampled at a quarter, is the 0x40042814
 * case 44 read where (7,0)'s 0x40043904 was wanted (records/imx8mp/
 * gc44-f2a13877).
 *
 * It includes gl_test.c itself, as the GC7000 arm (GL_TEST_USER,
 * YAKOGL_GPU_VIVANTE); no GL entry point is called, so the link ignores
 * them. tools/check.sh runs it ("run     gl_test case 44 probe decode"):
 *   mkdir -p build && cc -std=gnu11 -Wall -Wextra -Werror -DGL_TEST_USER=1 -DYAKOGL_GPU_VIVANTE=1 \
 *     -Iinclude -Isrc -Isrc/osal -Idriver/tools/pvrlab -Idriver/src -Idriver/src/vivante \
 *     tests/gltest_hdr_probe.c -o build/gltest_hdr_probe -no-pie -Wl,--unresolved-symbols=ignore-all \
 *     && build/gltest_hdr_probe
 */
#include "../driver/tools/pvrlab/gl_test.c"

static struct yakogl_backend fake;
const struct yakogl_backend *g_backend = &fake;

/* the checks below, so a run that stops early cannot pass */
#define PLANNED 21

static unsigned run, failed;
static float pic[HDR_PROBE_WMAX * HDR_PROBE_H * 4];

static void expect(const char *what, long got, long want)
{
	printf("%-58s got 0x%lx want 0x%lx %s\n", what, got, want, got == want ? "ok" : "FAIL");
	run++;
	failed += got != want;
}

enum layout { IN_PLACE, XOR2, SHIFT2 };

/* the picture a pixel engine with `l` leaves in a cleared w x 8 target */
static void store(int w, enum layout l)
{
	for (int i = 0; i < w * HDR_PROBE_H; i++) {
		pic[i * 4 + 0] = 2.0f; pic[i * 4 + 1] = 0.5f; pic[i * 4 + 2] = 0.25f; pic[i * 4 + 3] = 1.0f;
	}
	for (int y = 0; y < HDR_PROBE_H; y++) {
		for (int x = 0; x < w; x++) {
			int at = l == IN_PLACE ? x : l == XOR2 ? (x ^ 2) : x + 2;

			if (at < w)
				hdr_probe_expect(w, x, y, 0, &pic[(y * w + at) * 4]);
		}
	}
}

static int whole_rows(int w)
{
	int rows = 0;

	for (int y = 0; y < HDR_PROBE_H; y++) {
		bool whole = true;

		for (int x = 0; x < w && whole; x++)
			whole = hdr_probe_code(pic, w, x, y) == (uint32_t)(y << 3);
		rows += whole;
	}
	return rows;
}

static uint32_t quarter_rgba8(const float c[4])
{
	uint32_t v = 0;

	for (int i = 0; i < 4; i++)
		v |= (uint32_t)(c[i] * 0.25f * 255.0f + 0.5f) << (8 * i);
	return v;
}

int main(void)
{
	float c[4];

	/* the board's datum: (5,0)'s colour read where (7,0)'s was wanted */
	hdr_probe_expect(8, 5, 0, 0, c);
	expect("(5,0) x0.25 is case 44's got", quarter_rgba8(c), 0x40042814);
	hdr_probe_expect(8, 7, 0, 0, c);
	expect("(7,0) x0.25 is within case 44's tolerance of its want", chan_diff(quarter_rgba8(c), 0x40043904) <= 12, 1);

	/* in place: all whole, every slot its own pixel */
	for (int w = 4; w <= HDR_PROBE_WMAX; w *= 2) {
		store(w, IN_PLACE);
		expect("in place: rows whole", whole_rows(w), 8);
	}
	store(24, IN_PLACE);
	expect("in place, 24 wide (diagonal ties): rows whole", whole_rows(24), 8);
	store(8, IN_PLACE);
	expect("in place: slot (0,1) is row 1, offset 0", hdr_probe_code(pic, 8, 0, 1), 0x08);
	expect("in place: slot (7,7) is row 7, offset 0", hdr_probe_code(pic, 8, 7, 7), 0x38);

	/* x ^ 2: slot (0,0) holds (2,0), slot (7,0) holds (5,0) -- the datum */
	store(8, XOR2);
	expect("x^2: no row whole", whole_rows(8), 0);
	expect("x^2: slot (0,0) = +2", hdr_probe_code(pic, 8, 0, 0), 0x02);
	expect("x^2: slot (1,0) = +2", hdr_probe_code(pic, 8, 1, 0), 0x02);
	expect("x^2: slot (2,0) = -2", hdr_probe_code(pic, 8, 2, 0), 0x06);
	expect("x^2: slot (7,0) = -2 (pixel 5)", hdr_probe_code(pic, 8, 7, 0), 0x06);
	expect("x^2: slot (7,7) = row 7, -2", hdr_probe_code(pic, 8, 7, 7), 0x3e);

	/* shifted two right: slots 0 and 1 never drawn, the datum again at 7 */
	store(8, SHIFT2);
	expect("shift: slot (0,0) = the clear colour", hdr_probe_code(pic, 8, 0, 0), 0x40);
	expect("shift: slot (1,0) = the clear colour", hdr_probe_code(pic, 8, 1, 0), 0x40);
	expect("shift: slot (2,0) = -2 (pixel 0)", hdr_probe_code(pic, 8, 2, 0), 0x06);
	expect("shift: slot (7,0) = -2 (pixel 5)", hdr_probe_code(pic, 8, 7, 0), 0x06);

	/* nothing written at all, and a colour no pixel has */
	memset(pic, 0, sizeof(pic));
	expect("zeros: 0x41", hdr_probe_code(pic, 8, 0, 0), 0x41);
	pic[0] = 0.5f; pic[1] = 0.5f; pic[2] = 0.5f; pic[3] = 0.5f;
	expect("no pixel's colour: 0x7f", hdr_probe_code(pic, 8, 0, 0), 0x7f);

	printf("gltest_hdr_probe: %u checks run (planned %u), %u failed\n", run, PLANNED, failed);
	return failed || run != PLANNED;
}

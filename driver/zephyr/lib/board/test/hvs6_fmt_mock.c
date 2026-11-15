/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * hvs6_fmt_mock: every entry of the hvs6_fmt table, the lookup, the inverse
 * and the unknown codes. Built by tools/v3d/check71.d/hvs6_fmt.sh.
 *
 * The window values are mirrored, not included: yakogl_drivers.h declares
 * Rust-side symbols, and src/gl/egl.rs:47-51 holds GLES_NATIVE_FORMAT_RGBA8888
 * = 1, _XRGB8888 = 2, _RGB565 = 3. The static assertion pins the mirror.
 */
#include <stdio.h>
#include <string.h>

#include "hvs6_fmt.h"

_Static_assert(HVS6F_WIN_RGBA8888 == 1 && HVS6F_WIN_XRGB8888 == 2 && HVS6F_WIN_RGB565 == 3,
	       "window values mirror GLES_NATIVE_FORMAT_* in src/gl/egl.rs:47-51");

static int checks, failed;

#define CHECK(c)                                                              \
	do {                                                                  \
		checks++;                                                     \
		if (!(c)) {                                                   \
			failed++;                                             \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c);   \
		}                                                             \
	} while (0)

struct expect {
	const char *name;
	uint32_t code, order, bpp, alpha, window;
	int primary; /* 0: an alias, lookup returns the X entry */
};

static const struct expect exp_tab[] = {
	{"XRGB8888", 7, 2, 4, 0, 2, 1},    {"XBGR8888", 7, 3, 4, 0, 1, 1},
	{"RGBX8888", 7, 0, 4, 0, 0, 1},    {"BGRX8888", 7, 1, 4, 0, 0, 1},
	{"ARGB8888", 7, 2, 4, 1, 2, 0},    {"ABGR8888", 7, 3, 4, 1, 1, 0},
	{"RGBA8888", 7, 0, 4, 1, 0, 0},    {"BGRA8888", 7, 1, 4, 1, 0, 0},
	{"RGB565", 4, 0, 2, 0, 3, 1},      {"RGB888/RGBA", 5, 0, 3, 0, 0, 1},
	{"RGB888/BGRA", 5, 1, 3, 0, 0, 1}, {"RGB888/ARGB", 5, 2, 3, 0, 0, 1},
	{"RGB888/ABGR", 5, 3, 3, 0, 0, 1},
};

int main(void)
{
	size_t n = sizeof(exp_tab) / sizeof(exp_tab[0]);

	CHECK(HVS6F_COUNT == n);
	CHECK(hvs6_fmt_at(n) == NULL);
	CHECK(hvs6_fmt_at((size_t)-1) == NULL);

	for (size_t i = 0; i < n; i++) {
		const struct expect *e = &exp_tab[i];
		const struct hvs6_fmt *f = hvs6_fmt_at(i);
		const struct hvs6_fmt *l;

		CHECK(f != NULL);
		if (!f)
			continue;
		CHECK(strcmp(f->name, e->name) == 0);
		CHECK(f->code == e->code && f->order == e->order);
		CHECK(f->bpp == e->bpp);
		CHECK(f->alpha == e->alpha);
		CHECK(f->window == e->window);
		l = hvs6_fmt_lookup(e->code, e->order);
		CHECK(l != NULL);
		if (l) {
			CHECK(l->code == e->code && l->order == e->order);
			CHECK((l == f) == (e->primary != 0));
			CHECK(l->alpha == 0);
			CHECK(l->window == e->window);
		}
	}

	/* The inverse, and the round trip of every supported window format. */
	for (uint32_t w = 1; w <= 3; w++) {
		const struct hvs6_fmt *f = hvs6_fmt_for_window(w);

		CHECK(f != NULL);
		if (!f)
			continue;
		CHECK(f->window == w && f->alpha == 0);
		CHECK(hvs6_fmt_lookup(f->code, f->order) == f);
	}
	CHECK(strcmp(hvs6_fmt_for_window(HVS6F_WIN_XRGB8888)->name, "XRGB8888") == 0);
	CHECK(strcmp(hvs6_fmt_for_window(HVS6F_WIN_RGBA8888)->name, "XBGR8888") == 0);
	CHECK(strcmp(hvs6_fmt_for_window(HVS6F_WIN_RGB565)->name, "RGB565") == 0);
	CHECK(hvs6_fmt_for_window(0) == NULL);
	CHECK(hvs6_fmt_for_window(4) == NULL);
	CHECK(hvs6_fmt_for_window(0x80000002u) == NULL); /* BOTTOM_UP bit set */
	CHECK(hvs6_fmt_for_window(0xffffffffu) == NULL);

	/* Unknown codes and orders. */
	CHECK(hvs6_fmt_lookup(0, 0) == NULL);  /* RGB332 */
	CHECK(hvs6_fmt_lookup(1, 0) == NULL);  /* RGBA4444 */
	CHECK(hvs6_fmt_lookup(8, 0) == NULL);  /* YUV */
	CHECK(hvs6_fmt_lookup(16, 2) == NULL); /* RGBA1010102 */
	CHECK(hvs6_fmt_lookup(31, 0) == NULL);
	CHECK(hvs6_fmt_lookup(7, 4) == NULL);  /* order out of range */
	CHECK(hvs6_fmt_lookup(4, 2) == NULL);  /* RGB565 in another order */
	CHECK(hvs6_fmt_lookup(0xffffffffu, 0xffffffffu) == NULL);
	CHECK(hvs6_fmt_lookup(0x107u, 2) == NULL); /* no truncation to 8 bits */

	printf("hvs6_fmt_mock: %d of %d checks, %d failed\n", checks - failed, checks, failed);
	return failed ? 1 : 0;
}

// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/* Host mock for rpi5_scanout/status_text.h. Built by
 * tools/v3d/check71.d/status_text.sh. */
#include <stdio.h>
#include <string.h>
#include "status_text.h"

static int checks, failed;

#define CHECK(c)                                                          \
	do {                                                              \
		checks++;                                                 \
		if (!(c)) {                                               \
			failed++;                                         \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); \
		}                                                         \
	} while (0)

#define COLS 80
#define MAXL 16
static char buf[MAXL * (COLS + 1)];

static const char *line(uint32_t cols, uint32_t i)
{
	return buf + (size_t)i * (cols + 1);
}

static struct status_text full(void)
{
	struct status_text s = {0};

	s.route = 2;
	s.have_scanout = 1;
	strcpy(s.format, "XRGB8888");
	s.width = 1280;
	s.height = 720;
	s.pitch = 5120;
	s.have_plane_off = 1;
	s.plane_off = 0x1000;
	s.have_fw_rev = 1;
	s.fw_rev = 0x6a3b2c10;
	s.have_throttled = 1;
	s.throttled = 0x50005;
	s.pattern = STATUS_VERDICT_PASS;
	s.readback = STATUS_VERDICT_FAIL;
	return s;
}

static int all_ascii(uint32_t cols, uint32_t n)
{
	for (uint32_t i = 0; i < n; i++) {
		const char *p = line(cols, i);
		for (size_t k = 0; p[k]; k++)
			if (p[k] < 0x20 || p[k] > 0x7e)
				return 0;
	}
	return 1;
}

int main(void)
{
	struct status_text s = full();
	uint32_t n, c, r;

	/* geometry from the 8x16 cell */
	status_text_geometry(640, 480, &c, &r);
	CHECK(c == 80 && r == 15);
	status_text_geometry(1280, 720, &c, &r);
	CHECK(c == 160 && r == 22);
	status_text_geometry(1920, 1080, &c, &r);
	CHECK(c == 120 && r == 16);

	/* golden full case */
	memset(buf, 0x7f, sizeof(buf));
	n = status_text_lines(&s, COLS, buf, MAXL);
	CHECK(n == 8);
	CHECK(!strcmp(line(COLS, 0), "yakogl Pi 5 display probe"));
	CHECK(!strcmp(line(COLS, 1), "route: 2"));
	CHECK(!strcmp(line(COLS, 2), "scanout: XRGB8888 1280x720 pitch 5120"));
	CHECK(!strcmp(line(COLS, 3), "plane: offset +0x1000 in fb"));
	CHECK(!strcmp(line(COLS, 4), "firmware: 0x6a3b2c10"));
	CHECK(!strcmp(line(COLS, 5), "throttled: 0x50005"));
	CHECK(!strcmp(line(COLS, 6), "pattern: pass"));
	CHECK(!strcmp(line(COLS, 7), "readback: FAIL"));
	CHECK(all_ascii(COLS, n));

	/* routes */
	s.route = 1;
	n = status_text_lines(&s, COLS, buf, MAXL);
	CHECK(!strcmp(line(COLS, 1), "route: 1"));
	s.route = 3;
	n = status_text_lines(&s, COLS, buf, MAXL);
	CHECK(!strcmp(line(COLS, 1), "route: 3"));
	s.route = 0;
	n = status_text_lines(&s, COLS, buf, MAXL);
	CHECK(!strcmp(line(COLS, 1), "route: unknown"));
	s.route = 9;
	n = status_text_lines(&s, COLS, buf, MAXL);
	CHECK(!strcmp(line(COLS, 1), "route: unknown"));

	/* each field missing */
	s = full();
	s.have_scanout = 0;
	n = status_text_lines(&s, COLS, buf, MAXL);
	CHECK(!strcmp(line(COLS, 2), "scanout: missing"));
	s = full();
	s.have_plane_off = 0;
	n = status_text_lines(&s, COLS, buf, MAXL);
	CHECK(n == 7);
	CHECK(!strcmp(line(COLS, 3), "firmware: 0x6a3b2c10"));
	s = full();
	s.have_fw_rev = 0;
	n = status_text_lines(&s, COLS, buf, MAXL);
	CHECK(!strcmp(line(COLS, 4), "firmware: missing"));
	s = full();
	s.have_throttled = 0;
	n = status_text_lines(&s, COLS, buf, MAXL);
	CHECK(!strcmp(line(COLS, 5), "throttled: missing"));
	s = full();
	s.pattern = STATUS_VERDICT_UNKNOWN;
	s.readback = STATUS_VERDICT_UNKNOWN;
	n = status_text_lines(&s, COLS, buf, MAXL);
	CHECK(!strcmp(line(COLS, 6), "pattern: n/a"));
	CHECK(!strcmp(line(COLS, 7), "readback: n/a"));

	/* nothing known */
	memset(&s, 0, sizeof(s));
	s.pattern = s.readback = STATUS_VERDICT_UNKNOWN;
	n = status_text_lines(&s, COLS, buf, MAXL);
	CHECK(n == 7);
	CHECK(!strcmp(line(COLS, 1), "route: unknown"));
	CHECK(!strcmp(line(COLS, 2), "scanout: missing"));

	/* clipping at small cols; guard bytes after each row stay intact */
	s = full();
	for (uint32_t cols = 1; cols <= 12; cols++) {
		char small[MAXL * 13 + 1];
		memset(small, 0x7f, sizeof(small));
		n = status_text_lines(&s, cols, small, MAXL);
		CHECK(n == 8);
		for (uint32_t i = 0; i < n; i++)
			CHECK(strlen(small + (size_t)i * (cols + 1)) <= cols);
		CHECK((unsigned char)small[(size_t)n * (cols + 1)] == 0x7f);
	}
	n = status_text_lines(&s, 10, buf, MAXL);
	CHECK(!strcmp(line(10, 0), "yakogl Pi "));
	CHECK(!strcmp(line(10, 4), "firmware: "));

	/* max_lines limits, degenerate arguments */
	memset(buf, 0x7f, sizeof(buf));
	n = status_text_lines(&s, COLS, buf, 3);
	CHECK(n == 3);
	CHECK((unsigned char)buf[3 * (COLS + 1)] == 0x7f);
	CHECK(status_text_lines(&s, COLS, buf, 0) == 0);
	CHECK(status_text_lines(&s, 0, buf, MAXL) == 0);
	CHECK(status_text_lines(NULL, COLS, buf, MAXL) == 0);

	/* non-ASCII input is replaced; no addresses are printed */
	s = full();
	strcpy(s.format, "A\x01\xffZ");
	n = status_text_lines(&s, COLS, buf, MAXL);
	CHECK(!strncmp(line(COLS, 2), "scanout: A??Z ", 14));
	CHECK(all_ascii(COLS, n));
	s.format[0] = 0;
	n = status_text_lines(&s, COLS, buf, MAXL);
	CHECK(!strncmp(line(COLS, 2), "scanout: ? ", 11));

	printf("status_text_mock: %d of %d checks, %d failed\n", checks - failed, checks, failed);
	return failed ? 1 : 0;
}

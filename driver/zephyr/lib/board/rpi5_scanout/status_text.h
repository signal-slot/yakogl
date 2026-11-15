/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Status text for the top half of the Pi 5 split screen: what the display
 * probe found, as fixed-width lines of printable ASCII. Header-only C11, no
 * glyphs (fbcon renders them), no addresses (the plane is shown as an offset
 * in the framebuffer).
 *
 * Font cell: the Pi 4 fbcon uses the 8x16 font (GC_FONT_W 8, GC_FONT_H 16 in
 * driver/zephyr/lib/board/common/gc_font8x16.h:19-20) and sets
 * cols = width / (8 * scale), rows = rows_px / (16 * scale)
 * (driver/zephyr/lib/board/rpi4/fbcon.c:74-75). The text owns the top half,
 * rows_px = height / 2. Scale 1 below 1920 wide, 2 at 1920:
 *   640x480:   80 cols x 15 rows
 *   1280x720: 160 cols x 22 rows
 *   1920x1080: 120 cols x 16 rows
 */
#ifndef RPI5_SCANOUT_STATUS_TEXT_H
#define RPI5_SCANOUT_STATUS_TEXT_H

#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

#define STATUS_TEXT_FONT_W 8u
#define STATUS_TEXT_FONT_H 16u

#define STATUS_VERDICT_UNKNOWN (-1) /* not run */
#define STATUS_VERDICT_FAIL 0
#define STATUS_VERDICT_PASS 1

struct status_text {
	int route;              /* 1, 2, 3; anything else is unknown */
	int have_scanout;       /* format/width/height/pitch valid */
	char format[16];        /* e.g. "XRGB8888" */
	uint32_t width, height, pitch;
	int have_plane_off;
	uint32_t plane_off;     /* plane offset inside the framebuffer */
	int have_fw_rev;
	uint32_t fw_rev;        /* firmware revision */
	int have_throttled;
	uint32_t throttled;
	int pattern;            /* STATUS_VERDICT_* */
	int readback;           /* STATUS_VERDICT_* */
};

/* cols and rows of the top-half text for a screen of width x height. */
static inline void status_text_geometry(uint32_t width, uint32_t height,
					uint32_t *cols, uint32_t *rows)
{
	uint32_t scale = width >= 1920 ? 2 : 1;

	*cols = width / (STATUS_TEXT_FONT_W * scale);
	*rows = (height / 2) / (STATUS_TEXT_FONT_H * scale);
}

static inline char status_text_ascii(char c)
{
	return (c >= 0x20 && c <= 0x7e) ? c : '?';
}

static inline const char *status_text_verdict(int v)
{
	return v == STATUS_VERDICT_PASS ? "pass" :
	       v == STATUS_VERDICT_FAIL ? "FAIL" : "n/a";
}

/* One formatted line, clipped to cols, ASCII only, NUL terminated (out needs
 * cols + 1 bytes). */
static inline void status_text_put(char *out, uint32_t cols, const char *fmt, ...)
	__attribute__((format(printf, 3, 4)));

static inline void status_text_put(char *out, uint32_t cols, const char *fmt, ...)
{
	char tmp[160];
	va_list ap;
	size_t len;
	int n;

	va_start(ap, fmt);
	n = vsnprintf(tmp, sizeof(tmp), fmt, ap);
	va_end(ap);
	len = n < 0 ? 0 : (size_t)n;
	if (len > sizeof(tmp) - 1)
		len = sizeof(tmp) - 1;
	if (len > cols)
		len = cols;
	for (size_t i = 0; i < len; i++)
		out[i] = status_text_ascii(tmp[i]);
	out[len] = '\0';
}

/*
 * Fill up to max_lines lines. out holds max_lines rows of (cols + 1) bytes
 * (row stride cols + 1). Returns the number of lines written.
 */
static inline uint32_t status_text_lines(const struct status_text *s, uint32_t cols,
					 char *out, uint32_t max_lines)
{
	uint32_t n = 0;
	char fmt[sizeof(s->format)];
	size_t k = 0;
	uint32_t stride = cols + 1;

#define ST_EMIT(...)                                                              \
	do {                                                                      \
		if (n < max_lines) {                                              \
			status_text_put(out + (size_t)n * stride, cols, __VA_ARGS__); \
			n++;                                                      \
		}                                                                 \
	} while (0)

	if (!s || !out || cols == 0 || max_lines == 0)
		return 0;

	ST_EMIT("yakogl Pi 5 display probe");
	if (s->route >= 1 && s->route <= 3)
		ST_EMIT("route: %d", s->route);
	else
		ST_EMIT("route: unknown");
	if (s->have_scanout) {
		for (; k < sizeof(s->format) - 1 && s->format[k]; k++)
			fmt[k] = status_text_ascii(s->format[k]);
		fmt[k] = '\0';
		ST_EMIT("scanout: %s %ux%u pitch %u", k ? fmt : "?",
			(unsigned)s->width, (unsigned)s->height, (unsigned)s->pitch);
	} else {
		ST_EMIT("scanout: missing");
	}
	if (s->have_plane_off)
		ST_EMIT("plane: offset +0x%x in fb", (unsigned)s->plane_off);
	if (s->have_fw_rev)
		ST_EMIT("firmware: 0x%08x", (unsigned)s->fw_rev);
	else
		ST_EMIT("firmware: missing");
	if (s->have_throttled)
		ST_EMIT("throttled: 0x%x", (unsigned)s->throttled);
	else
		ST_EMIT("throttled: missing");
	ST_EMIT("pattern: %s", status_text_verdict(s->pattern));
	ST_EMIT("readback: %s", status_text_verdict(s->readback));
#undef ST_EMIT
	return n;
}

#endif /* RPI5_SCANOUT_STATUS_TEXT_H */

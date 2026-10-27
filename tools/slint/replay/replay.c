/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * replay: the Slint sample's Rust archive (examples/slint/rust) drives a real YakoGL
 * (libyakogl.a, Linux OSAL, one GPU back end) on the host. The frames are
 * ../slint_gl_rec.c's: the first frame (slint_sample_first_frame: the build
 * and the default state, then demo_vehicle at t = 0), then the crate's
 * animation (slint_sample_animate_frame) through the idle (0-3 s), the
 * acceleration (3-15 s) and the cruise (15-22 s). slint_gl_rec steps 58 fps
 * (1276 frames); this replay takes REPLAY_FRAMES (default 300) frames spread
 * evenly over the same 22 s, so each phase is covered. See README.md.
 */
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "replay.h"

#define W 1024 /* main.c's WIN_W x WIN_H */
#define H 600
#define SEQUENCE_MS 22000

int slint_sample_first_frame(const void *(*get_proc)(const char *), uint32_t width, uint32_t height);
int slint_sample_animate_frame(const void *(*get_proc)(const char *), uint32_t width, uint32_t height,
			       uint32_t t_ms, uint32_t frame);

void *slint_heap_alloc(size_t alignment, size_t size)
{
	void *p = NULL;

	if (alignment < sizeof(void *))
		alignment = sizeof(void *);
	if (posix_memalign(&p, alignment, size ? size : 1) != 0)
		return NULL;
	return p;
}

void slint_heap_free(void *p)
{
	free(p);
}

int printk(const char *fmt, ...)
{
	va_list ap;
	int n;

	va_start(ap, fmt);
	n = vprintf(fmt, ap);
	va_end(ap);
	return n;
}

int main(void)
{
	const char *e = getenv("REPLAY_FRAMES");
	unsigned frames = e && *e ? (unsigned)strtoul(e, NULL, 0) : 300;
	char label[48];
	int rc;

	if (frames == 0)
		frames = 1;
	setvbuf(stdout, NULL, _IONBF, 0); /* one ordered stream with the library's stderr lines */
	if (replay_egl_up(W, H))
		return 1;
	/* two draws in one call (frame 1 and frame 2 of the check); the library
	 * sees them back to back, as on the board */
	rc = slint_sample_first_frame(replay_get_proc, W, H);
	replay_frame_end("first (frames 1 and 2)", rc);
	for (unsigned i = 0; i < frames; i++) {
		uint32_t t = (uint32_t)((uint64_t)i * SEQUENCE_MS / frames);

		rc = slint_sample_animate_frame(replay_get_proc, W, H, t, 2 + i);
		snprintf(label, sizeof(label), "(t %u ms)", t);
		replay_frame_end(label, rc);
	}
	return replay_finish();
}

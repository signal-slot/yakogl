/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * replay: the cluster image's Rust library (../../rust) drives a real YakoGL
 * (libyakogl.a, Linux OSAL, one GPU back end) on the host. The frame
 * sequence is glcount's (../glcount/glcount.c): the check's two frames
 * (Slint clock 0 and 1000 ms, 400 telemetry ticks between them), then
 * REPLAY_FRAMES frames 200 ms apart (4 ticks each), as the demo draws. The
 * library's GL calls go to YakoGL, not to glcount's counting stub; see
 * README.md for what the run reports.
 */
#include <malloc.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "replay.h"

/* main.c's window is the panel (CONFIG_CLUSTER_PANEL_WIDTH x _HEIGHT of the
 * board file); run.sh passes the GPU's board's as REPLAY_W/REPLAY_H */
static uint32_t env_dim(const char *name, uint32_t def)
{
	const char *e = getenv(name);

	return e && *e ? (uint32_t)strtoul(e, NULL, 0) : def;
}

int slint_cluster_draw_frame(const void *(*get_proc)(const char *), uint32_t w, uint32_t h);
int slint_cluster_advance(uint32_t n);

static uint64_t ui_clock_ms;

uint64_t slint_zephyr_uptime_ms(void)
{
	return ui_clock_ms;
}

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
	unsigned frames = e && *e ? (unsigned)strtoul(e, NULL, 0) : 20;
	uint32_t W = env_dim("REPLAY_W", 1920), H = env_dim("REPLAY_H", 720);
	char label[48];

	setvbuf(stdout, NULL, _IONBF, 0); /* one ordered stream with the library's stderr lines */
	printf("replay: window %ux%u\n", W, H);
	if (replay_egl_up(W, H))
		return 1;
	for (unsigned f = 0; f < 2 + frames; f++) {
		if (f == 1) {
			slint_cluster_advance(400); /* main.c FRAME_TICKS */
			ui_clock_ms = 1000;
		} else if (f >= 2) {
			slint_cluster_advance(4);
			ui_clock_ms += 200;
		}
		int rc = slint_cluster_draw_frame(replay_get_proc, W, H);

		snprintf(label, sizeof(label), "(clock %llu ms)", (unsigned long long)ui_clock_ms);
		replay_frame_end(label, rc);
	}
	return replay_finish();
}

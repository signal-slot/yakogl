/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_clear -- the third step of a Vivante bring-up (docs/notes/
 * imx8mp-gc-bringup.md, step 4: gc_identify, gc_fence, gc_clear, gc_triangle):
 * fill a small linear XRGB8888 render target through the resolve engine and
 * read every pixel back on the CPU.
 *
 * The bring-up is gc_fence's (the GPU device's identify and reset, then
 * gc_render_init: hw init, MMU, interrupt, ring). The clear is gc_draw_clear(),
 * a resolve with a fill value: this family has no BLT engine, and the resolve
 * engine touches none of the 3D state, so the picture depends on memory, the
 * MMU, the submit path and one fixed-function block only. If gc_clear passes
 * and a triangle does not, the fault is in the 3D pipe.
 *
 * The target is poisoned first and cleared twice with different colours, so
 * neither "the allocator zeroed it" nor "only the first clear landed" can
 * pass; every pixel of the target is compared after each clear, and the guard
 * rows after it must keep the poison (a clear that ran past the window).
 *
 * Only the board layer (yakogl_board.h) and the GPU driver's own interface
 * (gc.h, gc_render.h, gc_draw.h) are used, so the sample builds for every
 * board of the board layer with a vivante,gc node.
 *
 * The result line:
 *   gc_clear: <W>x<H> wrong pixels <n> (clear 1 <n1>, clear 2 <n2>, guard <g>)
 * The record: on a pass kind APP, w1 = status 1, w2 = the clears (2),
 * w3 = W << 16 | H; when pixels are wrong kind CHECK, tag CHECK_UNDRAWN,
 * w1 = the wrong pixels (both clears and the guard rows); on any other
 * failure kind APP, w1 = stage << 8 | status 2 (the step that failed),
 * w2 = the clears completed, w3 = W << 16 | H. A failed step's errno is on
 * the console.
 */
#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>
#include <errno.h>

#include <yakogl_board.h>
#include <yakogl_record.h>

#include <gc.h>
#include <gc_render.h>
#include <gc_draw.h>

#define NAME "gc_clear"

#define W CONFIG_GC_CLEAR_WIDTH
#define H CONFIG_GC_CLEAR_HEIGHT
/* Rows after the target that the clear must leave alone. */
#define GUARD_ROWS 2u
#define POISON 0x00a55a3cu

BUILD_ASSERT(W % 16 == 0, "the resolve engine takes widths in multiples of 16");

enum step {
	STEP_DEVICE = 1, /* the GPU device: power, identify, reset */
	STEP_RENDER,     /* gc_render_init: hw init, MMU, interrupt, ring */
	STEP_ALLOC,      /* the render target */
	STEP_CLEAR,      /* gc_draw_clear: the submit */
	STEP_WAIT,       /* the clear's fence */
	STEP_CHECK,      /* the read-back */
};

static struct gc_target s_target;
static uint32_t s_clears;

static void stage(enum step s)
{
	board_progress(YK_RECORD_WHERE_STAGE | (uint32_t)s);
}

static int fail(enum step s, int rc)
{
	printk(NAME ": FAIL at step %d: %d\n", (int)s, rc);
	board_result(YK_RECORD_KIND_APP, YK_RECORD_STATUS_FAIL | (uint32_t)s << 8, s_clears,
		     (uint32_t)W << 16 | H);
	return rc != 0 ? rc : -EIO;
}

static volatile uint32_t *row(uint32_t y)
{
	return (volatile uint32_t *)((uint8_t *)s_target.cpu + (size_t)y * s_target.stride);
}

/* Pixels of rows [y0, y1) that are not `want` (X byte masked); the first one
 * is printed. */
static uint32_t count_wrong(uint32_t y0, uint32_t y1, uint32_t want, const char *what)
{
	uint32_t wrong = 0;

	for (uint32_t y = y0; y < y1; y++) {
		volatile uint32_t *r = row(y);

		for (uint32_t x = 0; x < W; x++) {
			uint32_t got = r[x] & 0x00ffffffu;

			if (got != want) {
				if (wrong == 0u) {
					printk(NAME ": %s: first wrong pixel (%u,%u) = 0x%06x, want 0x%06x\n",
					       what, x, y, got, want);
				}
				wrong++;
			}
		}
	}
	return wrong;
}

/* One clear, waited for; the wrong pixels of the target, or a negative errno
 * with *step_out set. */
static int clear_and_count(uint32_t colour, enum step *step_out)
{
	uint32_t fence = 0;
	int rc;

	stage(STEP_CLEAR);
	rc = gc_draw_clear(&s_target, colour, &fence);
	if (rc) {
		*step_out = STEP_CLEAR;
		return rc;
	}
	stage(STEP_WAIT);
	rc = gc_render_wait(fence, CONFIG_GC_CLEAR_WAIT_MS);
	if (rc) {
		gc_render_stall_dump(NAME " failed");
		*step_out = STEP_WAIT;
		return rc;
	}
	s_clears++;
	stage(STEP_CHECK);
	gc_render_cache_invd(s_target.cpu, (size_t)s_target.stride * (H + GUARD_ROWS));
	return (int)count_wrong(0, H, colour & 0x00ffffffu, "target");
}

static int run(void)
{
	const struct device *gpu = DEVICE_DT_GET_ONE(vivante_gc);
	const struct gc_identity *id;
	struct gc_bo bo;
	uint32_t wrong1, wrong2, guard;
	enum step at = STEP_CHECK;
	int rc;

	stage(STEP_DEVICE);
	if (!device_is_ready(gpu) || !gc_alive(gpu)) {
		return fail(STEP_DEVICE, -ENODEV);
	}
	id = gc_identity(gpu);
	printk(NAME ": GPU model 0x%x revision 0x%x product 0x%x customer 0x%x eco %u%s\n",
	       id->model, id->revision, id->product_id, id->customer_id, id->eco_id,
	       id->from_table ? " (features from the table)" : "");

	stage(STEP_RENDER);
	rc = gc_render_init(gpu);
	if (rc) {
		return fail(STEP_RENDER, rc);
	}

	stage(STEP_ALLOC);
	/* The stride is the width rounded up to the pixel engine's 64 bytes. */
	s_target.width = W;
	s_target.height = H;
	s_target.stride = ROUND_UP(W * 4u, 64u);
	rc = gc_render_alloc((size_t)s_target.stride * (H + GUARD_ROWS), 64, &bo);
	if (rc) {
		return fail(STEP_ALLOC, rc);
	}
	s_target.gpu = bo.gpu;
	s_target.cpu = bo.cpu;
	printk(NAME ": target %ux%u stride %u at GPU 0x%08x\n", W, H, s_target.stride,
	       s_target.gpu);

	/* Poison it, guard rows included. */
	for (uint32_t y = 0; y < H + GUARD_ROWS; y++) {
		volatile uint32_t *r = row(y);

		for (uint32_t x = 0; x < W; x++) {
			r[x] = POISON;
		}
	}
	gc_render_cache_flush(s_target.cpu, (size_t)s_target.stride * (H + GUARD_ROWS));

	rc = clear_and_count(gc_rgb(0x20, 0x80, 0xc0), &at);
	if (rc < 0) {
		return fail(at, rc);
	}
	wrong1 = (uint32_t)rc;
	board_watchdog_feed();
	rc = clear_and_count(gc_rgb(0xff, 0x00, 0x40), &at);
	if (rc < 0) {
		return fail(at, rc);
	}
	wrong2 = (uint32_t)rc;
	guard = count_wrong(H, H + GUARD_ROWS, POISON, "guard rows");

	uint32_t wrong = wrong1 + wrong2 + guard;

	printk(NAME ": %ux%u wrong pixels %u (clear 1 %u, clear 2 %u, guard %u)\n", W, H, wrong,
	       wrong1, wrong2, guard);
	if (wrong != 0u) {
		board_result_tag(YK_RECORD_KIND_CHECK, YK_RECORD_CHECK_UNDRAWN, wrong, 0, 0);
		return -EIO;
	}
	board_result(YK_RECORD_KIND_APP, YK_RECORD_STATUS_PASS, s_clears, (uint32_t)W << 16 | H);
	return 0;
}

int main(void)
{
	board_finish(NAME, run());
}

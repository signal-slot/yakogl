/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_triangle -- the fourth step of a Vivante bring-up (docs/notes/
 * imx8mp-gc-bringup.md, step 4: gc_identify, gc_fence, gc_clear, gc_triangle):
 * clear a small linear XRGB8888 render target to a known background, draw one
 * Gouraud triangle with a red, a green and a blue vertex, and read it back on
 * the CPU.
 *
 * The bring-up is gc_clear's (the GPU device's identify and reset, then
 * gc_render_init, then the target). The background is gc_draw_clear() (the
 * resolve engine, which gc_clear has proved), the triangle is
 * gc_draw_triangle() (the 3D pipe with the pass-through shaders). If gc_clear
 * passes and this does not, the fault is in the 3D pipe.
 *
 * The check is the i.MX8MP's three-apex read-back (docs/notes/
 * imx8mp-gc-bringup.md, the gl_triangle_user lines): the red apex, the green
 * left corner and the blue right corner are each sampled just inside the
 * triangle and must be dominated by their own channel; the pixels outside the
 * triangle's bounding box (and the guard rows after the target) must still be
 * the background; and the count of pixels that differ from the background
 * must be within a plausible range of the triangle's area. Only the CPU reads
 * pixels: the GPU's 0x800+ state space is never read (driver/docs/
 * vivante-kernel-rust.md).
 *
 * With CONFIG_GC_TRIANGLE_LINEAR_OUT the drawn target is then copied by the
 * resolve engine (gc_draw_copy) into the top left of a linear XRGB8888
 * buffer of a given width, height, pitch and address, the kind a display
 * controller scans out, and the same checks run on the copy (lines prefixed
 * "linear: "), with every pixel of the window compared to the target.
 *
 * Only the board layer (yakogl_board.h) and the GPU driver's own interface
 * (gc.h, gc_render.h, gc_draw.h) are used, so the sample builds for every
 * board of the board layer with a vivante,gc node.
 *
 * The result lines (one per check, then the verdict):
 *   gc_triangle: red apex PASS (x,y) = rrggbbaa
 *   gc_triangle: linear: PASS WxH pitch P at GPU 0x... phys 0x... (N failing checks)
 *   gc_triangle: PASS (0 failing checks)
 * The record: on a pass kind APP, w1 = status 1, w2 = the covered pixels,
 * w3 = W << 16 | H; when a check fails kind CHECK, tag CHECK_UNDRAWN,
 * w1 = the failing checks, w2 = the covered pixels, w3 = a bit mask of the
 * failing checks (bit 0 red apex, 1 green left, 2 blue right, 3 outside,
 * 4 covered pixels, 5 a check of the linear copy, 6 a pixel of the copy
 * that differs from the target); on any other failure kind APP, w1 = stage
 * << 8 | status 2 (the step that failed), w2 = the fences retired, w3 =
 * W << 16 | H. A failed step's errno is on the console.
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

#define NAME "gc_triangle"

#define W CONFIG_GC_TRIANGLE_WIDTH
#define H CONFIG_GC_TRIANGLE_HEIGHT
/* Rows after the target that nothing may touch. */
#define GUARD_ROWS 2u
#define POISON 0x00a55a3cu
#define BACKGROUND 0x00102030u
/* The apex is sampled this fraction of the way to the centroid. */
#define INSET 0.10f
/* Pixels around the bounding box that are left out of the outside check. */
#define EDGE_MARGIN 3
/* The covered pixel count may be this far off the geometric area. */
#define AREA_TOLERANCE 0.20f
/* A dominated pixel: its own channel at least this, the others at most that. */
#define DOMINANT_MIN 0xc0u
#define OTHER_MAX 0x40u

BUILD_ASSERT(W % 16 == 0, "the resolve engine takes widths in multiples of 16");

enum step {
	STEP_DEVICE = 1, /* the GPU device: power, identify, reset */
	STEP_RENDER,     /* gc_render_init: hw init, MMU, interrupt, ring */
	STEP_ALLOC,      /* the render target */
	STEP_CLEAR,      /* gc_draw_clear: the submit */
	STEP_DRAW,       /* gc_draw_triangle: the submit */
	STEP_WAIT,       /* a fence's wait */
	STEP_CHECK,      /* the read-back */
	STEP_LINEAR,     /* CONFIG_GC_TRIANGLE_LINEAR_OUT: the buffer, the copy's submit */
};

/* The triangle in clip space (x right, y up): red at the top, green bottom
 * left, blue bottom right. */
static const struct gc_vertex s_tri[3] = {
	{ .pos = { 0.0f, 0.8f, 0.0f, 1.0f }, .color = { 1.0f, 0.0f, 0.0f, 1.0f } },
	{ .pos = { -0.8f, -0.8f, 0.0f, 1.0f }, .color = { 0.0f, 1.0f, 0.0f, 1.0f } },
	{ .pos = { 0.8f, -0.8f, 0.0f, 1.0f }, .color = { 0.0f, 0.0f, 1.0f, 1.0f } },
};

static struct gc_target s_target;
static uint32_t s_fences;

static void stage(enum step s)
{
	board_progress(YK_RECORD_WHERE_STAGE | (uint32_t)s);
}

static int fail(enum step s, int rc)
{
	printk(NAME ": FAIL at step %d: %d\n", (int)s, rc);
	board_result(YK_RECORD_KIND_APP, YK_RECORD_STATUS_FAIL | (uint32_t)s << 8, s_fences,
		     (uint32_t)W << 16 | H);
	return rc != 0 ? rc : -EIO;
}

/* Wait for a submitted fence; a negative errno on a timeout. */
static int wait_fence(uint32_t fence)
{
	int rc;

	stage(STEP_WAIT);
	rc = gc_render_wait(fence, CONFIG_GC_TRIANGLE_WAIT_MS);
	if (rc) {
		gc_render_stall_dump(NAME " failed");
		return rc;
	}
	s_fences++;
	return 0;
}

/* Clip space to pixel coordinates (the viewport covers the whole target, y
 * points down in memory). */
static float px_x(float cx)
{
	return (cx + 1.0f) * 0.5f * (float)W;
}

static float px_y(float cy)
{
	return (1.0f - cy) * 0.5f * (float)H;
}

static uint32_t chan(uint32_t rgb, unsigned int shift)
{
	return (rgb >> shift) & 0xffu;
}

/* One apex check on `t`: the pixel `own` (0 red, 1 green, 2 blue) is
 * dominated by its own channel. Prints the line, `pfx` first; returns 1 when
 * it fails. */
static int check_apex(const struct gc_target *t, const char *pfx, const char *what,
		      unsigned int own, float x, float y)
{
	uint32_t px = (uint32_t)CLAMP((int)x, 0, (int)W - 1);
	uint32_t py = (uint32_t)CLAMP((int)y, 0, (int)H - 1);
	uint32_t v = gc_target_pixel(t, px, py);
	uint32_t mine = chan(v, 16u - 8u * own);
	bool ok = mine >= DOMINANT_MIN;

	for (unsigned int c = 0; c < 3u; c++) {
		if (c != own && chan(v, 16u - 8u * c) > OTHER_MAX) {
			ok = false;
		}
	}
	printk(NAME ": %s%-10s %s (%u,%u) = %08x\n", pfx, what, ok ? "PASS" : "FAIL", px, py,
	       (v << 8) | 0xffu);
	return ok ? 0 : 1;
}

/*
 * The checks of one picture: the three apexes, the outside, the covered
 * pixels. `t` is the buffer, whose width and height may exceed the drawn
 * W x H window at its top left; `rows` are the rows scanned (with the guard
 * rows). A pixel outside the window must still hold the poison. `pfx`
 * prefixes the lines. Adds the failing checks' bits to *mask and returns how
 * many failed.
 */
static uint32_t check_picture(const struct gc_target *t, uint32_t rows, const char *pfx,
			      uint32_t *covered_out, uint32_t *mask)
{
	uint32_t failing = 0, covered = 0, outside_wrong = 0;

	/* The triangle's corners in pixels and its centroid. */
	const float rx = px_x(s_tri[0].pos[0]), ry = px_y(s_tri[0].pos[1]);
	const float gx = px_x(s_tri[1].pos[0]), gy = px_y(s_tri[1].pos[1]);
	const float bx = px_x(s_tri[2].pos[0]), by = px_y(s_tri[2].pos[1]);
	const float cx = (rx + gx + bx) / 3.0f, cy = (ry + gy + by) / 3.0f;

	/* The three apexes, each just inside. */
	if (check_apex(t, pfx, "red apex", 0, rx + (cx - rx) * INSET, ry + (cy - ry) * INSET)) {
		failing++;
		*mask |= BIT(0);
	}
	if (check_apex(t, pfx, "green left", 1, gx + (cx - gx) * INSET, gy + (cy - gy) * INSET)) {
		failing++;
		*mask |= BIT(1);
	}
	if (check_apex(t, pfx, "blue right", 2, bx + (cx - bx) * INSET, by + (cy - by) * INSET)) {
		failing++;
		*mask |= BIT(2);
	}

	/* Outside the bounding box (less a margin) the background must stay; the
	 * guard rows and the buffer past the window must keep the poison;
	 * everything in the window that differs from the background is counted
	 * as covered. */
	const int x0 = (int)gx - EDGE_MARGIN, x1 = (int)bx + EDGE_MARGIN;
	const int y0 = (int)ry - EDGE_MARGIN, y1 = (int)gy + EDGE_MARGIN;
	uint32_t first_x = 0, first_y = 0, first_v = 0;

	for (uint32_t y = 0; y < rows; y++) {
		const volatile uint32_t *r =
			(const volatile uint32_t *)((uint8_t *)t->cpu + (size_t)y * t->stride);

		for (uint32_t x = 0; x < t->width; x++) {
			uint32_t v = r[x] & 0x00ffffffu;
			bool drawn = x < W && y < H;
			bool must_be_bg = drawn && ((int)x < x0 || (int)x > x1 || (int)y < y0 ||
						    (int)y > y1);

			if (drawn && v != BACKGROUND) {
				covered++;
			}
			if ((!drawn && v != POISON) || (must_be_bg && v != BACKGROUND)) {
				if (outside_wrong == 0u) {
					first_x = x;
					first_y = y;
					first_v = v;
				}
				outside_wrong++;
			}
		}
	}
	if (outside_wrong != 0u) {
		printk(NAME ": %soutside     FAIL (%u,%u) = %06x (%u wrong pixels)\n", pfx,
		       first_x, first_y, first_v, outside_wrong);
		failing++;
		*mask |= BIT(3);
	} else {
		printk(NAME ": %soutside     PASS (background kept outside the box and in the guard rows)\n",
		       pfx);
	}

	/* The area of the triangle in pixels. */
	float area = 0.5f * (bx - gx) * (gy - ry);
	float lo = area * (1.0f - AREA_TOLERANCE), hi = area * (1.0f + AREA_TOLERANCE);
	bool area_ok = (float)covered >= lo && (float)covered <= hi;

	printk(NAME ": %scovered     %s %u pixels (%u..%u expected)\n", pfx,
	       area_ok ? "PASS" : "FAIL", covered, (uint32_t)lo, (uint32_t)hi);
	if (!area_ok) {
		failing++;
		*mask |= BIT(4);
	}
	*covered_out = covered;
	return failing;
}

#ifdef CONFIG_GC_TRIANGLE_LINEAR_OUT
#define OUT_W (CONFIG_GC_TRIANGLE_LINEAR_OUT_WIDTH ? CONFIG_GC_TRIANGLE_LINEAR_OUT_WIDTH : W)
#define OUT_H (CONFIG_GC_TRIANGLE_LINEAR_OUT_HEIGHT ? CONFIG_GC_TRIANGLE_LINEAR_OUT_HEIGHT : H)
#define OUT_PITCH                                                                                  \
	((uint32_t)(CONFIG_GC_TRIANGLE_LINEAR_OUT_PITCH ? CONFIG_GC_TRIANGLE_LINEAR_OUT_PITCH      \
							: ROUND_UP(OUT_W * 4u, 64u)))
/* An allocated buffer has guard rows after it; an address given is used for
 * exactly the buffer. */
#define OUT_GUARD (CONFIG_GC_TRIANGLE_LINEAR_OUT_ADDR == 0 ? GUARD_ROWS : 0u)

BUILD_ASSERT(H % 4 == 0, "the resolve engine's copy takes heights in multiples of 4");
BUILD_ASSERT(OUT_W >= W && OUT_H >= H, "the scan-out buffer is smaller than the target");
BUILD_ASSERT(OUT_PITCH % 64 == 0 && OUT_PITCH >= OUT_W * 4,
	     "the scan-out pitch must be a multiple of 64 and hold the width");
BUILD_ASSERT(CONFIG_GC_TRIANGLE_LINEAR_OUT_ADDR % 64 == 0,
	     "the scan-out address must be 64-byte aligned");

/*
 * CONFIG_GC_TRIANGLE_LINEAR_OUT: the drawn target copied by the resolve
 * engine (gc_draw_copy) into the top left of a linear XRGB8888 buffer of
 * OUT_W x OUT_H at OUT_PITCH, the kind a display controller scans out, and
 * the picture's checks repeated on that copy. The buffer is poisoned first,
 * so a pixel outside the window that changed is caught. Adds bit 5 (a check
 * of the copy) and bit 6 (a window pixel that differs from the target) to
 * *mask. Returns the failing checks, or a negative errno after fail().
 */
static int linear_out(uint32_t *mask)
{
	const size_t bytes = (size_t)OUT_PITCH * (OUT_H + OUT_GUARD);
	struct gc_target out = { .width = OUT_W, .height = OUT_H, .stride = OUT_PITCH };
	struct gc_bo bo;
	uint32_t fence = 0, lmask = 0, covered = 0, differ = 0, failing;
	int rc;

	stage(STEP_LINEAR);
	if (CONFIG_GC_TRIANGLE_LINEAR_OUT_ADDR == 0) {
		rc = gc_render_alloc(bytes, 4096, &bo);
	} else {
		rc = gc_render_map_phys(CONFIG_GC_TRIANGLE_LINEAR_OUT_ADDR, bytes, &bo);
	}
	if (rc == 0 && bo.cpu == NULL) {
		rc = -EFAULT;   /* no CPU view of the buffer to check */
	}
	if (rc) {
		return fail(STEP_LINEAR, rc);
	}
	out.gpu = bo.gpu;
	out.cpu = bo.cpu;

	for (size_t i = 0; i < bytes / 4u; i++) {
		((volatile uint32_t *)out.cpu)[i] = POISON;
	}
	gc_render_cache_flush(out.cpu, bytes);

	rc = gc_draw_copy(&s_target, &out, &fence);
	if (rc) {
		return fail(STEP_LINEAR, rc);
	}
	rc = wait_fence(fence);
	if (rc) {
		return fail(STEP_WAIT, rc);
	}
	board_watchdog_feed();
	stage(STEP_CHECK);
	gc_render_cache_invd(out.cpu, bytes);

	failing = check_picture(&out, OUT_H + OUT_GUARD, "linear: ", &covered, &lmask);
	if (lmask != 0u) {
		*mask |= BIT(5);
	}
	for (uint32_t y = 0; y < H; y++) {
		const volatile uint32_t *s =
			(const volatile uint32_t *)((uint8_t *)s_target.cpu + (size_t)y * s_target.stride);
		const volatile uint32_t *d =
			(const volatile uint32_t *)((uint8_t *)out.cpu + (size_t)y * out.stride);

		for (uint32_t x = 0; x < W; x++) {
			differ += ((s[x] ^ d[x]) & 0x00ffffffu) != 0u;
		}
	}
	printk(NAME ": linear: copy       %s (%u of %ux%u pixels differ from the target)\n",
	       differ == 0u ? "PASS" : "FAIL", differ, W, H);
	if (differ != 0u) {
		failing++;
		*mask |= BIT(6);
	}
	printk(NAME ": linear: %s %ux%u pitch %u at GPU 0x%08x phys 0x%llx (%u failing checks)\n",
	       failing == 0u ? "PASS" : "FAIL", OUT_W, OUT_H, OUT_PITCH, out.gpu,
	       (unsigned long long)bo.phys, failing);
	return (int)failing;
}
#endif

static int run(void)
{
	const struct device *gpu = DEVICE_DT_GET_ONE(vivante_gc);
	const struct gc_identity *id;
	struct gc_bo bo;
	uint32_t fence = 0;
	uint32_t failing = 0, mask = 0, covered = 0;
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

	/* Poison the guard rows (the clear and the triangle must not reach them). */
	for (uint32_t y = H; y < H + GUARD_ROWS; y++) {
		volatile uint32_t *r =
			(volatile uint32_t *)((uint8_t *)s_target.cpu + (size_t)y * s_target.stride);

		for (uint32_t x = 0; x < W; x++) {
			r[x] = POISON;
		}
	}
	gc_render_cache_flush(s_target.cpu, (size_t)s_target.stride * (H + GUARD_ROWS));

	stage(STEP_CLEAR);
	rc = gc_draw_clear(&s_target, BACKGROUND, &fence);
	if (rc) {
		return fail(STEP_CLEAR, rc);
	}
	rc = wait_fence(fence);
	if (rc) {
		return fail(STEP_WAIT, rc);
	}
	board_watchdog_feed();

	stage(STEP_DRAW);
	rc = gc_draw_triangle(&s_target, s_tri, &fence);
	if (rc) {
		return fail(STEP_DRAW, rc);
	}
	rc = wait_fence(fence);
	if (rc) {
		return fail(STEP_WAIT, rc);
	}
	board_watchdog_feed();

	stage(STEP_CHECK);
	gc_render_cache_invd(s_target.cpu, (size_t)s_target.stride * (H + GUARD_ROWS));

	failing = check_picture(&s_target, H + GUARD_ROWS, "", &covered, &mask);

#ifdef CONFIG_GC_TRIANGLE_LINEAR_OUT
	rc = linear_out(&mask);
	if (rc < 0) {
		return rc;
	}
	failing += (uint32_t)rc;
#endif

	if (failing != 0u) {
		printk(NAME ": FAIL (%u failing checks)\n", failing);
		board_result_tag(YK_RECORD_KIND_CHECK, YK_RECORD_CHECK_UNDRAWN, failing, covered,
				 mask);
		return -EIO;
	}
	printk(NAME ": PASS (0 failing checks)\n");
	board_result(YK_RECORD_KIND_APP, YK_RECORD_STATUS_PASS, covered, (uint32_t)W << 16 | H);
	return 0;
}

int main(void)
{
	board_finish(NAME, run());
}

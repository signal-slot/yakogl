/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * mp2_gpu_scanout: a GPU-rendered frame on the DK's HDMI. gc_triangle's
 * bring-up and draw (driver/samples/zephyr/gc_triangle: the GPU device,
 * gc_render_init, gc_draw_clear to 0x00102030, one Gouraud triangle, the
 * three-apex CPU read-back), at the display mode's size, then its "linear
 * scan-out copy" (gc_draw_copy, the resolve engine) into a linear XRGB8888
 * buffer from gc_render_alloc, and that buffer as the LTDC's layer.
 *
 * The order:
 *   1. the LTDC device (display_yakogl_stm32_ltdc.c, the chain owner)
 *      chooses the mode (display_get_capabilities: HPD and EDID through the
 *      ADV7535; 1920x1080@60 when the EDID prefers it, else 1280x720@60);
 *   2. the GPU draws a W x H target (the mode's size), the CPU checks it
 *      (gc_triangle's lines);
 *   3. the resolve engine copies it into the scan-out buffer, W x H at
 *      ROUND_UP(W * 4, 64), 4096-aligned in the GPU carveout, and the same
 *      checks run on the copy, with every pixel compared to the target;
 *   4. the buffer's range is cleaned (gc_render_cache_flush), its physical
 *      range checked against the LTDC's RIF/RISAF (stm32_ltdc_board_fb_check),
 *      the chain brought up (display_blanking_off: LTDC, DSI, ADV7535) and the
 *      layer flipped to bo.phys (stm32_ltdc_set_fb, which waits for the
 *      reload);
 *   5. MP2_GPU_SCANOUT_FRAMES frames must come with no transfer or underrun
 *      error (stm32_ltdc_wait_frame). The LTDC's frame CRC is not in the
 *      driver's C ABI (yakogl_drv/stm32_ltdc.h), so there is no CRC check.
 *
 * Why the format and the address need no conversion: the PE's X8R8G8B8 is
 * the word 0x00RRGGBB, the LTDC's format 0 (ARGB8888, constant alpha) reads
 * it unchanged, the pitch is the LTDC driver's (64-byte aligned), and bo.phys
 * is in RISAF4 region 10, below 4 GiB (docs/notes/stm32mp257f-dk-display-
 * bringup.md, "GPU buffer format" and "Cache maintenance" (3)).
 *
 * The verdict, in tools/board-stm32mp2.sh's "<prog>: PASS|FAIL: " shape:
 *   mp2_gpu_scanout: PASS: <w>x<h> pitch <p> phys 0x... <n> frames
 *   mp2_gpu_scanout: FAIL: <reason> [<err>]
 * PASS only when the read-back and the scan-out both succeeded. Then the
 * scan-out is kept CONFIG_MP2_GPU_SCANOUT_HOLD_S seconds (one frame checked a
 * second) for the runner's MP2_HOLD_S and the KVM. The board layer
 * (stm32mp2.c) resets the board through PSCI at its lifetime.
 *
 * Only the CPU reads pixels: the GPU's 0x800+ state space is never read
 * (driver/docs/vivante-kernel-rust.md).
 */
#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/drivers/display.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/util.h>
#include <errno.h>

#include <yakogl_mp25_display.h>
#include <yakogl_drv/adv7535.h>
#include <yakogl_drv/stm32_dsi.h>
#include <yakogl_drv/stm32_ltdc.h>

#include <gc.h>
#include <gc_render.h>
#include <gc_draw.h>

#define NAME "mp2_gpu_scanout"

#define LTDC_NODE DT_NODELABEL(ltdc)
#define DSI_NODE  DT_PHANDLE(LTDC_NODE, dsi_host)
#define ADV_NODE  DT_PHANDLE(DSI_NODE, bridge)

BUILD_ASSERT(DT_NODE_HAS_STATUS_OKAY(LTDC_NODE), "the overlay enables the ltdc node");
BUILD_ASSERT(CONFIG_MP2_GPU_SCANOUT_HOLD_S + 30 < CONFIG_YAKOGL_STM32MP2_LIFETIME_S,
	     "the board layer's lifetime covers the draw and the hold");

/* Frames that must come clean before PASS: one second at 60 Hz. */
#define MP2_GPU_SCANOUT_FRAMES 60

/* gc_triangle's picture and checks (driver/samples/zephyr/gc_triangle/src/main.c). */
#define GUARD_ROWS 2u
#define POISON 0x00a55a3cu
#define BACKGROUND 0x00102030u
#define INSET 0.10f
#define EDGE_MARGIN 3
#define AREA_TOLERANCE 0.20f
#define DOMINANT_MIN 0xc0u
#define OTHER_MAX 0x40u
/* The pixel engine's and the LTDC driver's line alignment. */
#define PITCH_ALIGN 64u
/* gc_triangle's allocation of the scan-out buffer. */
#define OUT_ALIGN 4096u

/* gc_triangle's triangle in clip space (x right, y up): red at the top,
 * green bottom left, blue bottom right. Full screen, it lands at (0.5, 0.1),
 * (0.1, 0.9), (0.9, 0.9) of the width and height. */
static const struct gc_vertex s_tri[3] = {
	{ .pos = { 0.0f, 0.8f, 0.0f, 1.0f }, .color = { 1.0f, 0.0f, 0.0f, 1.0f } },
	{ .pos = { -0.8f, -0.8f, 0.0f, 1.0f }, .color = { 0.0f, 1.0f, 0.0f, 1.0f } },
	{ .pos = { 0.8f, -0.8f, 0.0f, 1.0f }, .color = { 0.0f, 0.0f, 1.0f, 1.0f } },
};

/* The drawn window: the mode's size, both buffers. */
static uint32_t s_w, s_h;
static struct gc_target s_target;

static int fail(const char *reason, int err)
{
	printk(NAME ": FAIL: %s %d\n", reason, err);
	return err != 0 ? err : -EIO;
}

static int wait_fence(uint32_t fence)
{
	int rc = gc_render_wait(fence, CONFIG_MP2_GPU_SCANOUT_WAIT_MS);

	if (rc) {
		gc_render_stall_dump(NAME " failed");
	}
	return rc;
}

static float px_x(float cx)
{
	return (cx + 1.0f) * 0.5f * (float)s_w;
}

static float px_y(float cy)
{
	return (1.0f - cy) * 0.5f * (float)s_h;
}

static uint32_t chan(uint32_t rgb, unsigned int shift)
{
	return (rgb >> shift) & 0xffu;
}

/* gc_triangle's check_apex: the pixel `own` (0 red, 1 green, 2 blue) is
 * dominated by its own channel. Returns 1 when it fails. */
static int check_apex(const struct gc_target *t, const char *pfx, const char *what,
		      unsigned int own, float x, float y)
{
	uint32_t px = (uint32_t)CLAMP((int)x, 0, (int)s_w - 1);
	uint32_t py = (uint32_t)CLAMP((int)y, 0, (int)s_h - 1);
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

/* gc_triangle's check_picture over the s_w x s_h window of `t` and `rows`
 * rows (the guard rows after it must hold the poison). Returns the failing
 * checks; *covered_out the pixels that differ from the background. */
static uint32_t check_picture(const struct gc_target *t, uint32_t rows, const char *pfx,
			      uint32_t *covered_out)
{
	uint32_t failing = 0, covered = 0, outside_wrong = 0;
	const float rx = px_x(s_tri[0].pos[0]), ry = px_y(s_tri[0].pos[1]);
	const float gx = px_x(s_tri[1].pos[0]), gy = px_y(s_tri[1].pos[1]);
	const float bx = px_x(s_tri[2].pos[0]), by = px_y(s_tri[2].pos[1]);
	const float cx = (rx + gx + bx) / 3.0f, cy = (ry + gy + by) / 3.0f;

	failing += check_apex(t, pfx, "red apex", 0, rx + (cx - rx) * INSET,
			      ry + (cy - ry) * INSET);
	failing += check_apex(t, pfx, "green left", 1, gx + (cx - gx) * INSET,
			      gy + (cy - gy) * INSET);
	failing += check_apex(t, pfx, "blue right", 2, bx + (cx - bx) * INSET,
			      by + (cy - by) * INSET);

	const int x0 = (int)gx - EDGE_MARGIN, x1 = (int)bx + EDGE_MARGIN;
	const int y0 = (int)ry - EDGE_MARGIN, y1 = (int)gy + EDGE_MARGIN;
	uint32_t first_x = 0, first_y = 0, first_v = 0;

	for (uint32_t y = 0; y < rows; y++) {
		const volatile uint32_t *r =
			(const volatile uint32_t *)((uint8_t *)t->cpu + (size_t)y * t->stride);

		for (uint32_t x = 0; x < s_w; x++) {
			uint32_t v = r[x] & 0x00ffffffu;
			bool drawn = y < s_h;
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
	} else {
		printk(NAME ": %soutside     PASS (background kept outside the box and in the guard rows)\n",
		       pfx);
	}

	float area = 0.5f * (bx - gx) * (gy - ry);
	float lo = area * (1.0f - AREA_TOLERANCE), hi = area * (1.0f + AREA_TOLERANCE);
	bool area_ok = (float)covered >= lo && (float)covered <= hi;

	printk(NAME ": %scovered     %s %u pixels (%u..%u expected)\n", pfx,
	       area_ok ? "PASS" : "FAIL", covered, (uint32_t)lo, (uint32_t)hi);
	if (!area_ok) {
		failing++;
	}
	*covered_out = covered;
	return failing;
}

/* Poison `rows` rows of `t` from row `from`, then clean them to DDR. */
static void poison_rows(const struct gc_target *t, uint32_t from, uint32_t rows)
{
	for (uint32_t y = from; y < from + rows; y++) {
		volatile uint32_t *r =
			(volatile uint32_t *)((uint8_t *)t->cpu + (size_t)y * t->stride);

		for (uint32_t x = 0; x < t->stride / 4u; x++) {
			r[x] = POISON;
		}
	}
	gc_render_cache_flush((uint8_t *)t->cpu + (size_t)from * t->stride,
			      (size_t)rows * t->stride);
}

/*
 * The GPU half: gc_triangle's bring-up, clear, triangle and read-back into
 * s_target, then its linear copy into *out (from gc_render_alloc, s_w x s_h
 * at `pitch`, two poisoned guard rows after it) and the checks on the copy.
 * 0 when every check passed, else the negative errno after the FAIL line.
 */
static int render(uint32_t pitch, struct gc_target *out, struct gc_bo *out_bo)
{
	const struct device *gpu = DEVICE_DT_GET_ONE(vivante_gc);
	const struct gc_identity *id;
	const size_t tbytes = (size_t)pitch * (s_h + GUARD_ROWS);
	struct gc_bo bo;
	uint32_t fence = 0, failing, covered = 0, differ = 0;
	int rc;

	if (!device_is_ready(gpu) || !gc_alive(gpu)) {
		return fail("gpu-device", -ENODEV);
	}
	id = gc_identity(gpu);
	printk(NAME ": GPU model 0x%x revision 0x%x product 0x%x customer 0x%x eco %u%s\n",
	       id->model, id->revision, id->product_id, id->customer_id, id->eco_id,
	       id->from_table ? " (features from the table)" : "");
	rc = gc_render_init(gpu);
	if (rc) {
		return fail("gpu-render-init", rc);
	}

	/* The render target, the mode's size, linear at the pixel engine's
	 * 64-byte stride, with gc_triangle's guard rows. */
	rc = gc_render_alloc(tbytes, 64, &bo);
	if (rc == 0 && bo.cpu == NULL) {
		rc = -EFAULT;
	}
	if (rc) {
		return fail("gpu-alloc", rc);
	}
	s_target = (struct gc_target){
		.gpu = bo.gpu, .width = s_w, .height = s_h, .stride = pitch, .cpu = bo.cpu,
	};
	printk(NAME ": target %ux%u stride %u at GPU 0x%08x\n", s_w, s_h, pitch, s_target.gpu);
	poison_rows(&s_target, s_h, GUARD_ROWS);

	rc = gc_draw_clear(&s_target, BACKGROUND, &fence);
	if (rc) {
		return fail("gpu-clear", rc);
	}
	rc = wait_fence(fence);
	if (rc) {
		return fail("gpu-wait-clear", rc);
	}
	rc = gc_draw_triangle(&s_target, s_tri, &fence);
	if (rc) {
		return fail("gpu-triangle", rc);
	}
	rc = wait_fence(fence);
	if (rc) {
		return fail("gpu-wait-triangle", rc);
	}
	gc_render_cache_invd(s_target.cpu, tbytes);
	failing = check_picture(&s_target, s_h + GUARD_ROWS, "", &covered);

	/* gc_triangle's linear scan-out copy, at the same size: the buffer the
	 * LTDC scans. Poisoned first, so a pixel the copy missed shows. */
	rc = gc_render_alloc(tbytes, OUT_ALIGN, out_bo);
	if (rc == 0 && out_bo->cpu == NULL) {
		rc = -EFAULT;
	}
	if (rc) {
		return fail("linear-alloc", rc);
	}
	*out = (struct gc_target){
		.gpu = out_bo->gpu, .width = s_w, .height = s_h, .stride = pitch,
		.cpu = out_bo->cpu,
	};
	poison_rows(out, 0, s_h + GUARD_ROWS);
	rc = gc_draw_copy(&s_target, out, &fence);
	if (rc) {
		return fail("linear-copy", rc);
	}
	rc = wait_fence(fence);
	if (rc) {
		return fail("linear-wait", rc);
	}
	gc_render_cache_invd(out->cpu, tbytes);
	failing += check_picture(out, s_h + GUARD_ROWS, "linear: ", &covered);
	for (uint32_t y = 0; y < s_h; y++) {
		const volatile uint32_t *s =
			(const volatile uint32_t *)((uint8_t *)s_target.cpu + (size_t)y * pitch);
		const volatile uint32_t *d =
			(const volatile uint32_t *)((uint8_t *)out->cpu + (size_t)y * pitch);

		for (uint32_t x = 0; x < s_w; x++) {
			differ += ((s[x] ^ d[x]) & 0x00ffffffu) != 0u;
		}
	}
	printk(NAME ": linear: copy       %s (%u of %ux%u pixels differ from the target)\n",
	       differ == 0u ? "PASS" : "FAIL", differ, s_w, s_h);
	if (differ != 0u) {
		failing++;
	}
	printk(NAME ": linear: %s %ux%u pitch %u at GPU 0x%08x phys 0x%llx (%u failing checks)\n",
	       failing == 0u ? "PASS" : "FAIL", s_w, s_h, pitch, out->gpu,
	       (unsigned long long)out_bo->phys, failing);
	if (failing != 0u) {
		printk(NAME ": FAIL: readback (%u failing checks)\n", failing);
		return -EIO;
	}
	return 0;
}

static const char *const step_names[] = {
	"none", "board", "fb", "adv-init", "mode", "clock", "ltdc-probe", "ltdc-configure",
	"ltdc-enable", "dsi-mode", "dsi", "adv-start", "frame", "i2c",
};

static const char *step_name(uint32_t step)
{
	return step < ARRAY_SIZE(step_names) ? step_names[step] : "?";
}

/* mp2_display's record of the chain. */
static void print_record(const struct stm32_ltdc_status *st)
{
	const struct yd_mp25_dsi_data *dd = stm32_dsi_data(DEVICE_DT_GET(DSI_NODE));
	const struct yd_mp25_adv7535_data *ad = adv7535_data(DEVICE_DT_GET(ADV_NODE));
	const struct yd_mp25_mode *m = &st->mode;

	printk(NAME ": chain step %s err %d, hpd %u, edid %u (header %02x %02x, ext %u), "
	       "choice %d\n",
	       step_name(st->step), (int)st->err, st->hpd, st->edid_ok, st->edid[0], st->edid[1],
	       st->edid[126], (int)st->choice);
	printk(NAME ": mode %ux%u %u Hz h %u/%u/%u v %u/%u/%u flags 0x%x, %s vic %u\n",
	       m->hactive, m->vactive, m->pixel_clock_hz, m->hfront, m->hsync, m->hback,
	       m->vfront, m->vsync, m->vback, m->flags, st->sink.hdmi ? "hdmi" : "dvi",
	       st->sink.vic);
	printk(NAME ": ltdc hwver 0x%06x bus %u layers %u on %u, pitch %u, fb 0x%llx, "
	       "clock %u Hz\n",
	       st->ltdc.hwver, st->ltdc.bus_bytes, st->ltdc.layers, st->ltdc.on, st->pitch,
	       (unsigned long long)st->fb_phys, st->pixel_clock_hz);
	if (dd != NULL) {
		printk(NAME ": dsi on %u lanes %u lane %u kbit/s, failed step %u errno %d\n",
		       dd->on, dd->lanes, dd->lane_kbps, dd->failed_step, (int)dd->failed_errno);
	}
	if (ad != NULL) {
		printk(NAME ": adv7535 rev 0x%02x hpd %u started %u\n", ad->revision, ad->hpd,
		       ad->started);
	}
}

/* The whole run; 0 on PASS (the line printed), else the FAIL line's errno. */
static int run(const struct device *dev)
{
	const struct device *dsi = DEVICE_DT_GET(DSI_NODE);
	const struct device *adv = DEVICE_DT_GET(ADV_NODE);
	const struct stm32_ltdc_status *st;
	const struct yd_mp25_dsi_data *dd;
	const struct yd_mp25_adv7535_data *ad;
	struct display_capabilities caps;
	struct gc_target out;
	struct gc_bo out_bo;
	uint32_t pitch, isr = 0;
	size_t bytes;
	int frames, rc;

	/* 1. The mode. */
	if (!device_is_ready(dev)) {
		return fail("ltdc-ready", -ENODEV);
	}
	st = stm32_ltdc_status(dev);
	display_get_capabilities(dev, &caps);
	if (caps.x_resolution == 0u || caps.y_resolution == 0u) {
		print_record(st);
		return fail(step_name(st->step), st->err != 0 ? st->err : -EIO);
	}
	if (caps.current_pixel_format != PIXEL_FORMAT_ARGB_8888) {
		return fail("format", -ENOTSUP);
	}
	s_w = caps.x_resolution;
	s_h = caps.y_resolution;
	pitch = ROUND_UP(s_w * 4u, PITCH_ALIGN);
	/* The flip keeps the chain's pitch: the GPU's buffer must have it. */
	if (st->pitch != pitch) {
		printk(NAME ": the LTDC's pitch %u is not the GPU buffer's %u\n", st->pitch, pitch);
		return fail("pitch", -EINVAL);
	}
	/* The resolve engine: widths in multiples of 16, heights of 4. */
	if (s_w % 16u != 0u || s_h % 4u != 0u) {
		return fail("size", -EINVAL);
	}
	printk(NAME ": mode %ux%u, buffer pitch %u\n", s_w, s_h, pitch);

	/* 2, 3. The GPU's frame and its read-back. */
	rc = render(pitch, &out, &out_bo);
	if (rc) {
		return rc;
	}

	/* 4. The scan-out: the CPU's lines of the buffer cleaned (it only read
	 * them since the invalidate, so this is the note's rule kept, not a
	 * write-back), the range allowed for the LTDC, the chain up, the layer
	 * flipped to the GPU's buffer. */
	bytes = (size_t)pitch * s_h;
	gc_render_cache_flush(out.cpu, bytes);
	rc = stm32_ltdc_board_fb_check(out_bo.phys, bytes);
	if (rc) {
		return fail("fb-check", rc);
	}
	rc = display_blanking_off(dev);
	print_record(st);
	if (rc) {
		return fail(step_name(st->step), rc);
	}
	dd = stm32_dsi_data(dsi);
	ad = adv7535_data(adv);
	if (!st->on || !st->ltdc.on || dd == NULL || !dd->on || ad == NULL || !ad->started) {
		return fail("chain", -EIO);
	}
	rc = stm32_ltdc_set_fb(dev, out_bo.phys);
	if (rc) {
		return fail("set-fb", rc);
	}
	if (st->fb_phys != out_bo.phys) {
		printk(NAME ": the LTDC scans 0x%llx, not 0x%llx\n",
		       (unsigned long long)st->fb_phys, (unsigned long long)out_bo.phys);
		return fail("set-fb", -EIO);
	}
	printk(NAME ": layer at phys 0x%llx (GPU 0x%08x)\n", (unsigned long long)out_bo.phys,
	       out.gpu);

	/* 5. The frames. */
	for (frames = 0; frames < MP2_GPU_SCANOUT_FRAMES; frames++) {
		rc = stm32_ltdc_wait_frame(dev, &isr);
		if (rc) {
			printk(NAME ": frame %d: isr 0x%x\n", frames, isr);
			return fail("frame", rc);
		}
	}
	printk(NAME ": PASS: %ux%u pitch %u phys 0x%llx %d frames\n", s_w, s_h, pitch,
	       (unsigned long long)out_bo.phys, frames);
	return 0;
}

int main(void)
{
	const struct device *dev = DEVICE_DT_GET(LTDC_NODE);
	const struct stm32_ltdc_status *st;
	int good = 0, bad = 0;
	bool up;

	printk(NAME ": build %s, hold %d s\n", MP2_GPU_SCANOUT_BUILD,
	       CONFIG_MP2_GPU_SCANOUT_HOLD_S);
	up = run(dev) == 0;
	/* Whatever the chain shows (the GPU's frame, or on a failure after the
	 * chain came up, what it scans) stays up for the runner and the KVM. */
	st = stm32_ltdc_status(dev);
	if (!up && st != NULL && st->on) {
		up = true;
	}
	for (int s = 0; s < CONFIG_MP2_GPU_SCANOUT_HOLD_S; s++) {
		k_sleep(K_SECONDS(1));
		if (up) {
			uint32_t isr = 0;

			if (stm32_ltdc_wait_frame(dev, &isr) == 0) {
				good++;
			} else {
				bad++;
				printk(NAME ": hold: frame at %d s failed, isr 0x%x\n", s, isr);
			}
		}
	}
	if (up) {
		printk(NAME ": hold: %d frames ok, %d failed\n", good, bad);
	}
	return 0;
}

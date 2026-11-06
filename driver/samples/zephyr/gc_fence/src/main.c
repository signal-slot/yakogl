/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_fence -- the second step of a Vivante bring-up (docs/notes/
 * imx8mp-gc-bringup.md, step 4: gc_identify, gc_fence, gc_clear, gc_triangle):
 * one command buffer whose fence retires through the GPU's interrupt, and
 * nothing drawn.
 *
 * The GPU device's init has identified the GPU and reset it to idle
 * (gpu_vivante_gc_rust.c: yd_gc_identify, yd_gc_reset); gc_render_init() does
 * the hardware init, the MMU v2 tables, the interrupt and the front end's idle
 * loop (yd_gc_hw_init, yd_gc_mmu_*, yd_gc_ring_init). The sample then submits
 * a buffer of NOPs, which the render service wraps with the prologue and the
 * EVENT, and waits for its fence with a bound. A retired fence proves the
 * front end fetched through the MMU, the EVENT reached the pixel engine, the
 * interrupt arrived on the right line and the ring re-armed.
 *
 * Only the board layer (yakogl_board.h) and the GPU driver's own interface
 * (gc.h, gc_render.h) are used, so the sample builds for every board of the
 * board layer with a vivante,gc node.
 *
 * The result line:
 *   gc_fence: fence <seqno> irqs <n> elapsed <us> us irq-latency <us> us
 * The record (kind APP): w1 = stage << 8 | status (1 pass, 2 fail; the stage
 * is the step that failed, 0 on a pass); w2 = the fence seqno (0 when none
 * was handed out); w3 = the interrupts that retired a fence << 24 (saturating
 * at 255) | the elapsed time from the submit to the wait's return in us
 * (24 bits, saturating). A failed step's errno is on the console.
 */
#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/sys/printk.h>
#include <errno.h>

#include <yakogl_board.h>
#include <yakogl_record.h>

#include <gc.h>
#include <gc_render.h>
#include <hw/cmdstream.xml.h>

#define NAME "gc_fence"

/* A few NOPs; the size is irrelevant. An all-zero buffer would not do: opcode
 * 0 is not a command. */
#define CMD_BYTES 64u

/* The step a run is in: the PROGRESS breadcrumb's stage and, on a failure,
 * the APP record's stage. */
enum step {
	STEP_DEVICE = 1, /* the GPU device: power, identify, reset */
	STEP_RENDER,     /* gc_render_init: hw init, MMU, interrupt, ring */
	STEP_ALLOC,      /* the command buffer */
	STEP_SUBMIT,
	STEP_WAIT,       /* the fence, with the bound */
	STEP_IRQ,        /* the fence retired without an interrupt counted */
};

static uint32_t sat(uint64_t v, uint32_t max)
{
	return v > max ? max : (uint32_t)v;
}

static void stage(enum step s)
{
	board_progress(YK_RECORD_WHERE_STAGE | (uint32_t)s);
}

static int fail(enum step s, int rc, uint32_t fence, uint32_t w3)
{
	printk(NAME ": FAIL at step %d: %d\n", (int)s, rc);
	board_result(YK_RECORD_KIND_APP, YK_RECORD_STATUS_FAIL | (uint32_t)s << 8, fence, w3);
	return rc != 0 ? rc : -EIO;
}

static int run(void)
{
	const struct device *gpu = DEVICE_DT_GET_ONE(vivante_gc);
	const struct gc_identity *id;
	struct gc_render_perf p0, p1;
	struct gc_bo cmd;
	uint32_t fence = 0, irqs, w3;
	uint64_t t0, t1, elapsed_us, irq_us;
	int rc;

	stage(STEP_DEVICE);
	if (!device_is_ready(gpu) || !gc_alive(gpu)) {
		return fail(STEP_DEVICE, -ENODEV, 0, 0);
	}
	id = gc_identity(gpu);
	printk(NAME ": GPU model 0x%x revision 0x%x product 0x%x customer 0x%x eco %u%s\n",
	       id->model, id->revision, id->product_id, id->customer_id, id->eco_id,
	       id->from_table ? " (features from the table)" : "");

	stage(STEP_RENDER);
	rc = gc_render_init(gpu);
	if (rc) {
		return fail(STEP_RENDER, rc, 0, 0);
	}

	stage(STEP_ALLOC);
	/* The service writes a LINK back at cmd_bytes: GC_CMDBUF_TAIL of slack. */
	rc = gc_render_alloc(CMD_BYTES + GC_CMDBUF_TAIL, GC_ALIGN_CMDBUF, &cmd);
	if (rc) {
		return fail(STEP_ALLOC, rc, 0, 0);
	}
	for (uint32_t i = 0; i < CMD_BYTES / 4u; i++) {
		((volatile uint32_t *)cmd.cpu)[i] = VIV_FE_NOP_HEADER_OP_NOP;
	}

	stage(STEP_SUBMIT);
	struct gc_submit s = {
		.cmd_gpu = cmd.gpu,
		.cmd_bytes = CMD_BYTES,
		.flags = GC_SUBMIT_FLUSH_ALL,
	};

	gc_render_perf(&p0);
	t0 = k_cycle_get_64();
	rc = gc_render_submit(&s, &fence);
	if (rc) {
		return fail(STEP_SUBMIT, rc, 0, 0);
	}

	stage(STEP_WAIT);
	rc = gc_render_wait(fence, CONFIG_GC_FENCE_WAIT_MS);
	t1 = k_cycle_get_64();
	gc_render_perf(&p1);
	elapsed_us = k_cyc_to_us_floor64(t1 - t0);
	/* kicks_done moves only in the interrupt handler (gpu::vivante::irq),
	 * once per retired event; gpu_busy_ns is the submit-to-interrupt span. */
	irqs = p1.kicks_done - p0.kicks_done;
	irq_us = (p1.gpu_busy_ns - p0.gpu_busy_ns) / 1000u;
	w3 = sat(irqs, 0xff) << 24 | sat(elapsed_us, 0xffffff);
	printk(NAME ": fence %u irqs %u elapsed %llu us irq-latency %llu us (completed %u, wait %d)\n",
	       fence, irqs, (unsigned long long)elapsed_us, (unsigned long long)irq_us,
	       gc_render_completed(), rc);
	if (rc) {
		gc_render_stall_dump(NAME " failed");
		return fail(STEP_WAIT, rc, fence, w3);
	}
	if (irqs == 0u) {
		return fail(STEP_IRQ, -EIO, fence, w3);
	}
	board_result(YK_RECORD_KIND_APP, YK_RECORD_STATUS_PASS, fence, w3);
	return 0;
}

int main(void)
{
	board_finish(NAME, run());
}

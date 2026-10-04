/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_fence - milestones 2 and 3 on the i.MX8MP GPU, without drawing anything.
 *
 * It brings up the render service (hardware init, MMU v2, the interrupt, the
 * front end on its idle loop), then submits an empty command buffer several
 * times and waits for each fence. That is the smallest thing that proves the
 * whole path works: the front end fetches from a GPU virtual address, so the
 * MMU translates; the EVENT command reaches the pixel engine, so the ring's
 * prologue and epilogue are well formed; the interrupt arrives, so the event
 * bit and the GIC line are right; and the ring re-arms, so the trick of
 * overwriting the WAIT word with a LINK holds.
 *
 * Boot it from Torizon with tools/kexec-zephyr:
 *   kexec_zephyr zephyr.bin 0x42000000
 */
#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/logging/log.h>
#include <errno.h>

#include <gc.h>
#include <gc_render.h>
#include <hw/cmdstream.xml.h>

#include "gc_sample.h"

LOG_MODULE_REGISTER(gc_fence, LOG_LEVEL_INF);

#define CMD_BYTES  64u   /* a couple of NOPs; the size is irrelevant */

static const char *run(void)
{
	const struct device *gpu = DEVICE_DT_GET_ONE(vivante_gc);
	struct gc_bo cmd;
	int rc;

	if (!device_is_ready(gpu)) {
		return "the GPU device did not initialise";
	}
	rc = gc_render_init(gpu);
	if (rc) {
		LOG_ERR("gc_render_init: %d", rc);
		return "the render service did not come up";
	}

	/* The service writes a LINK back into the ring at cmd_bytes, so the
	 * allocation carries GC_CMDBUF_TAIL of slack past the commands. */
	rc = gc_render_alloc(CMD_BYTES + GC_CMDBUF_TAIL, GC_ALIGN_CMDBUF, &cmd);
	if (rc) {
		return "no carveout memory for a command buffer";
	}
	LOG_INF("command buffer: GPU 0x%08x, CPU %p", cmd.gpu, cmd.cpu);

	/* Nothing but NOPs: the sample writes no register, so a failure here
	 * is in the ring, the MMU or the interrupt, never in a stream the
	 * sample got wrong. An all-zero buffer would NOT do -- opcode 0 is not
	 * a command. */
	for (uint32_t i = 0; i < CMD_BYTES / 4; i++) {
		((uint32_t *)cmd.cpu)[i] = VIV_FE_NOP_HEADER_OP_NOP;
	}

	for (int i = 0; i < CONFIG_GC_FENCE_ROUNDS; i++) {
		struct gc_submit s = {
			.cmd_gpu = cmd.gpu,
			.cmd_bytes = CMD_BYTES,
			.flags = GC_SUBMIT_FLUSH_ALL,
		};
		uint32_t fence = 0;

		gc_sample_service_wdog();
		rc = gc_render_submit(&s, &fence);
		if (rc) {
			LOG_ERR("round %d: submit %d", i, rc);
			return "submit failed";
		}
		rc = gc_render_wait(fence, 500);
		if (rc) {
			LOG_ERR("round %d: fence %u wait %d", i, fence, rc);
			return rc == -ETIMEDOUT ? "the fence never completed"
						: "the GPU faulted";
		}
		LOG_INF("round %d: fence %u done (completed %u)", i, fence,
			gc_render_completed());
	}
	return NULL;
}

int main(void)
{
	const char *reason;

	gc_sample_service_wdog();
	reason = run();
	gc_sample_verdict("gc_fence", reason == NULL, reason);
	if (reason != NULL) {
		gc_render_stall_dump("gc_fence failed");
	}
	gc_sample_finish();
	return 0;
}

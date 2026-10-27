/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * Writers for the GC7000 front-end command stream: the small layer the draw
 * builders (gc_draw.c) are written in terms of. The ring is Rust
 * (gpu::vivante::cmdbuf, with its own command words in gpu::vivante::cmd).
 *
 * The encoding is hw/cmdstream.xml.h. Every command is one or two 32-bit
 * words; the front end fetches 64-bit words, which is why the odd-length ones
 * are padded and why sizes are given to the hardware in units of eight bytes.
 *
 * LOAD_STATE is the only way to write a GPU register from the stream: a header
 * with the first register's offset (divided by four) and a count, then that
 * many values. It is also the only way to reach the registers that have no
 * MMIO meaning, such as the event and pipe-select ones.
 */

#include <zephyr/kernel.h>
#include <zephyr/logging/log.h>

#include "gc_internal.h"
#include "hw/cmdstream.xml.h"
#include "hw/state.xml.h"
#include "hw/common.xml.h"

LOG_MODULE_DECLARE(gpu_gc, CONFIG_YAKOGL_DRV_GPU_VIVANTE_LOG_LEVEL);

bool gc_cmd_room(const struct gc_cmd *c, uint32_t dwords)
{
	return c->n + dwords <= c->max;
}

static void put(struct gc_cmd *c, uint32_t w)
{
	if (c->n < c->max) {
		c->buf[c->n] = w;
	}
	c->n++;
}

void gc_cmd_state_multi(struct gc_cmd *c, uint32_t reg, const uint32_t *vals, uint32_t count)
{
	put(c, VIV_FE_LOAD_STATE_HEADER_OP_LOAD_STATE |
	       VIV_FE_LOAD_STATE_HEADER_COUNT(count) |
	       VIV_FE_LOAD_STATE_HEADER_OFFSET(reg >> VIV_FE_LOAD_STATE_HEADER_OFFSET__SHR));
	for (uint32_t i = 0; i < count; i++) {
		put(c, vals[i]);
	}
	/* header + count values; pad the pair out to 64 bits. */
	if ((count + 1) & 1) {
		put(c, VIV_FE_NOP_HEADER_OP_NOP);
	}
}

void gc_cmd_state(struct gc_cmd *c, uint32_t reg, uint32_t value)
{
	gc_cmd_state_multi(c, reg, &value, 1);
}

/* The same, with the header's FIXP bit: the values are 16.16 fixed point.
 * The viewport and scissor registers want it (Mesa's EMIT_STATE_FIXP). */
void gc_cmd_state_fixp(struct gc_cmd *c, uint32_t reg, uint32_t value)
{
	put(c, VIV_FE_LOAD_STATE_HEADER_OP_LOAD_STATE | VIV_FE_LOAD_STATE_HEADER_FIXP |
	       VIV_FE_LOAD_STATE_HEADER_COUNT(1) |
	       VIV_FE_LOAD_STATE_HEADER_OFFSET(reg >> VIV_FE_LOAD_STATE_HEADER_OFFSET__SHR));
	put(c, value);
}

void gc_cmd_nop(struct gc_cmd *c)
{
	put(c, VIV_FE_NOP_HEADER_OP_NOP);
	put(c, 0);
}

void gc_cmd_end(struct gc_cmd *c)
{
	put(c, VIV_FE_END_HEADER_OP_END);
	put(c, 0);
}

/*
 * The fence. A LOAD_STATE of the event register: the id, and which unit
 * raises it. FROM_PE means "when the pixel engine has retired everything",
 * which is what a render needs -- FROM_FE would fire as soon as the front end
 * read the command, with the pixels still in flight.
 */
void gc_cmd_event(struct gc_cmd *c, uint32_t event_id, bool from_pe)
{
	gc_cmd_state(c, VIVS_GL_EVENT, VIVS_GL_EVENT_EVENT_ID(event_id) |
				       (from_pe ? VIVS_GL_EVENT_FROM_PE : VIVS_GL_EVENT_FROM_FE));
}

/* The semaphore/stall pair: the semaphore arms it, the stall makes the front
 * end wait until `to` has caught up with `from`. Always emitted together. */
void gc_cmd_semaphore(struct gc_cmd *c, uint32_t from, uint32_t to)
{
	gc_cmd_state(c, VIVS_GL_SEMAPHORE_TOKEN,
		     VIVS_GL_SEMAPHORE_TOKEN_FROM(from) | VIVS_GL_SEMAPHORE_TOKEN_TO(to));
}

void gc_cmd_stall(struct gc_cmd *c, uint32_t from, uint32_t to)
{
	put(c, VIV_FE_STALL_HEADER_OP_STALL);
	put(c, VIV_FE_STALL_TOKEN_FROM(from) | VIV_FE_STALL_TOKEN_TO(to));
}

/* `prefetch` is in 64-bit words: how much the front end may read ahead at the
 * target. Everything that produces it divides a byte count by eight. */
void gc_cmd_link(struct gc_cmd *c, uint32_t address, uint32_t prefetch)
{
	put(c, VIV_FE_LINK_HEADER_OP_LINK | VIV_FE_LINK_HEADER_PREFETCH(prefetch));
	put(c, address);
}

void gc_cmd_wait(struct gc_cmd *c, uint32_t delay)
{
	put(c, VIV_FE_WAIT_HEADER_OP_WAIT | VIV_FE_WAIT_HEADER_DELAY(delay));
	put(c, 0);
}

/* DRAW_INSTANCED, which HALTI5 cores use in place of DRAW_PRIMITIVES; one
 * instance. Three words, so it needs a pad. */
void gc_cmd_draw(struct gc_cmd *c, uint32_t prim_type, uint32_t start, uint32_t count)
{
	put(c, VIV_FE_DRAW_INSTANCED_HEADER_OP_DRAW_INSTANCED |
	       VIV_FE_DRAW_INSTANCED_HEADER_TYPE(prim_type) |
	       VIV_FE_DRAW_INSTANCED_HEADER_INSTANCE_COUNT_LO(1));
	put(c, VIV_FE_DRAW_INSTANCED_COUNT_INSTANCE_COUNT_HI(0) |
	       VIV_FE_DRAW_INSTANCED_COUNT_VERTEX_COUNT(count));
	put(c, VIV_FE_DRAW_INSTANCED_START_INDEX(start));
	put(c, VIV_FE_NOP_HEADER_OP_NOP);
}

void gc_cmd_align8(struct gc_cmd *c)
{
	while (c->n & 1) {
		put(c, VIV_FE_NOP_HEADER_OP_NOP);
	}
}

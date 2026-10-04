/*
 * SPDX-License-Identifier: Apache-2.0 AND MIT
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * SPDX-FileCopyrightText: 2012-2019 Etnaviv Project
 * Ported from Mesa 26.2.1 src/gallium/drivers/etnaviv/ (etnaviv_context.c,
 * etnaviv_emit.c, etnaviv_state.c, etnaviv_rs.c, etnaviv_screen.c) (MIT).
 * The upstream part stays under its licence (LICENSES/MIT.txt); our
 * changes are Apache-2.0. See NOTICE.
 *
 * Milestones 4 and 5: clear a linear XRGB8888 surface, and draw one Gouraud
 * triangle into it. See gc_draw.h for the contract.
 *
 * The reference throughout is Mesa 26.2.1's etnaviv Gallium driver, which is
 * the only complete description of this GPU's 3D state that exists:
 *   etnaviv_context.c   etna_reset_gpu_state(), the per-buffer init state
 *   etnaviv_emit.c      etna_emit_state(), emit_halti5_only_state()
 *   etnaviv_state.c     where each register's value comes from
 *   etnaviv_rs.c        the resolve-engine clear
 *   etnaviv_screen.c    etna_get_specs(), which HALTI5 path this core takes
 * with the register layouts in the hw/ headers. Every value below names the
 * register and, where it is not obvious, why it has that value.
 *
 * Three facts about this particular core shape everything here:
 *
 *  1. It has NO BLT engine (the hardware database's BltEngine bit is 0), so
 *     the clear is the resolve engine, the path Mesa picks for it.
 *  2. It is HALTI5, so shader code is fetched from memory through the
 *     instruction cache -- there is no instruction memory to write, and Mesa
 *     asserts on it. The whole pre-HALTI5 register set (VS_OUTPUT(n),
 *     VS_INPUT(n), VS_START_PC/END_PC, PA_SHADER_ATTRIBUTES, the
 *     GL_VARYING_* block) must NOT be programmed; HALTI5 has its own.
 *  3. A linear render target needs PE_LOGIC_OP's SINGLE_BUFFER field set to
 *     1. Without it the pixel engine writes a tiled layout and the read-back
 *     is a scrambled picture rather than a wrong one.
 *
 * Nothing here has run on silicon.
 */

#include <zephyr/kernel.h>
#include <zephyr/logging/log.h>
#include <zephyr/sys/util.h>
#include <errno.h>
#include <string.h>

#include "gc_internal.h"
#include "gc_draw.h"
#include "gc_shaders.h"
#include "hw/cmdstream.xml.h"
#include "hw/state.xml.h"
#include "hw/state_3d.xml.h"
#include "hw/common.xml.h"
#include "hw/common_3d.xml.h"

LOG_MODULE_DECLARE(gpu_gc, CONFIG_SAFEUI_GPU_VIVANTE_LOG_LEVEL);

/* Room for the longest stream (the triangle: about 90 commands) plus the
 * LINK the render service writes past the end. */
#define GC_DRAW_CMD_BYTES  4096u

/* The float's bit pattern, which is how the hardware wants a float register. */
static inline uint32_t fui(float f)
{
	union { float f; uint32_t u; } u = { .f = f };

	return u.u;
}

/* Mesa's S_FIXED(f, 16): a truncating 16.16 conversion. The viewport and
 * scissor registers take this, flagged by the FIXP bit in the LOAD_STATE. */
static inline uint32_t fixp16(float f)
{
	return (uint32_t)(int32_t)(f * 65536.0f);
}

/* ------------------------------------------------------------------ *
 * Per-command-buffer state
 *
 * Mesa re-emits this at the head of EVERY command buffer, not once at boot
 * (etna_reset_gpu_state, called whenever ctx->needs_gpu_state_reset is set,
 * which a flush sets). So does this driver.
 * ------------------------------------------------------------------ */
static void emit_reset_state(struct gc_cmd *c)
{
	uint32_t zeros[VIVS_NFE_GENERIC_ATTRIB__LEN] = { 0 };

	gc_cmd_state(c, VIVS_GL_API_MODE, VIVS_GL_API_MODE_OPENGL);
	gc_cmd_state(c, VIVS_PA_W_CLIP_LIMIT, 0x34000001);
	gc_cmd_state(c, VIVS_PA_FLAGS, 0);
	gc_cmd_state(c, VIVS_PA_VIEWPORT_UNK00A80, 0x38a01404);
	gc_cmd_state(c, VIVS_PA_VIEWPORT_UNK00A84, fui(8192.0f));
	gc_cmd_state(c, VIVS_PA_ZFARCLIPPING, 0);
	gc_cmd_state(c, VIVS_RA_HDEPTH_CONTROL, 0x00007000);
	gc_cmd_state(c, VIVS_PS_CONTROL_EXT, 0);
	gc_cmd_state(c, VIVS_VS_HALTI1_UNK00884, 0x00000808);
	gc_cmd_state(c, VIVS_RA_UNK00E0C, 0);
	gc_cmd_state(c, VIVS_PS_HALTI3_UNK0103C, 0x76543210);
	gc_cmd_state(c, VIVS_PE_HALTI4_UNK014C0, 0);
	gc_cmd_state(c, VIVS_NTE_DESCRIPTOR_CONTROL, VIVS_NTE_DESCRIPTOR_CONTROL_ENABLE);
	gc_cmd_state(c, VIVS_FE_HALTI5_UNK007D8, 0x00000002);
	gc_cmd_state(c, VIVS_PS_SAMPLER_BASE, 0);
	/* VS_SAMPLER_BASE is skipped: this core has unified samplers. */
	gc_cmd_state(c, VIVS_SH_CONFIG, VIVS_SH_CONFIG_RTNE_ROUNDING);
	/* PS_MSAA_CONFIG's reset value, with the five fields Mesa clears for a
	 * core that has MSAA_FRAGMENT_OPERATION. */
	gc_cmd_state(c, VIVS_PS_MSAA_CONFIG, 0x6706f67f);
	gc_cmd_state(c, VIVS_GL_BUG_FIXES, 0x00000006);   /* BUG_FIXES18 is set */
	gc_cmd_state(c, VIVS_NTE_DESCRIPTOR_FLUSH, 0);
	gc_cmd_state(c, VIVS_GL_FLUSH_CACHE,
		     VIVS_GL_FLUSH_CACHE_DESCRIPTOR_UNK12 |
		     VIVS_GL_FLUSH_CACHE_DESCRIPTOR_UNK13);
	gc_cmd_state(c, VIVS_VS_ICACHE_INVALIDATE, 0x0000001f);
	/* Some GPUs come out of reset with random vertex attributes enabled
	 * (Mesa's comment), so all 32 generic attribute slots are cleared
	 * before anything real is written. */
	gc_cmd_state_multi(c, VIVS_NFE_GENERIC_ATTRIB_CONFIG0(0), zeros,
			   VIVS_NFE_GENERIC_ATTRIB__LEN);
}

/* The FE/PE semaphore and stall pair Mesa calls etna_stall(). When the
 * source is the front end the stall is a command; otherwise it is a write of
 * the stall-token register. */
static void emit_stall(struct gc_cmd *c, uint32_t from, uint32_t to)
{
	gc_cmd_semaphore(c, from, to);
	if (from == SYNC_RECIPIENT_FE) {
		gc_cmd_stall(c, from, to);
	} else {
		gc_cmd_state(c, VIVS_GL_STALL_TOKEN,
			     VIVS_GL_STALL_TOKEN_FROM(from) | VIVS_GL_STALL_TOKEN_TO(to));
	}
}

static int check_target(const struct gc_target *t, bool for_rs)
{
	if ((t->gpu & 63u) || (t->stride & 63u)) {
		LOG_ERR("target 0x%08x stride %u: both must be 64-byte aligned", t->gpu,
			t->stride);
		return -EINVAL;
	}
	if (t->width == 0 || t->height == 0 || t->stride < t->width * 4u) {
		LOG_ERR("target %ux%u stride %u is not consistent", t->width, t->height,
			t->stride);
		return -EINVAL;
	}
	/* The resolve engine aborts on a width that is not a multiple of 16
	 * (Mesa's ETNA_RS_WIDTH_MASK). The 3D pipe has no such limit. */
	if (for_rs && (t->width & 15u)) {
		LOG_ERR("clear: width %u must be a multiple of 16 for the resolve engine",
			t->width);
		return -EINVAL;
	}
	return 0;
}

/* One command buffer, allocated once and rebuilt per call. */
static struct gc_bo s_cmd;

static int cmd_begin(struct gc_cmd *c)
{
	int rc;

	if (s_cmd.cpu == NULL) {
		rc = gc_render_alloc(GC_DRAW_CMD_BYTES + GC_CMDBUF_TAIL, GC_ALIGN_CMDBUF,
				     &s_cmd);
		if (rc) {
			return rc;
		}
	}
	*c = (struct gc_cmd){
		.buf = s_cmd.cpu,
		.gpu = s_cmd.gpu,
		.n = 0,
		.max = GC_DRAW_CMD_BYTES / 4,
	};
	return 0;
}

static int cmd_submit(struct gc_cmd *c, uint32_t flags, uint32_t *fence_out)
{
	struct gc_submit s;

	gc_cmd_align8(c);
	if (!gc_cmd_room(c, 0)) {
		LOG_ERR("draw: the stream needs %u dwords, the buffer holds %u", c->n, c->max);
		return -ENOSPC;
	}
	s = (struct gc_submit){ .cmd_gpu = c->gpu, .cmd_bytes = c->n * 4, .flags = flags };
	return gc_render_submit(&s, fence_out);
}

/* ------------------------------------------------------------------ *
 * M4: the clear
 *
 * The resolve engine, because this core has no BLT engine. It is a
 * fixed-function blitter: point it at the destination, give it a window size
 * and a fill value, and kick it. It touches none of the 3D state, which is
 * what makes it the right first picture -- if the clear works and the
 * triangle does not, the fault is in the 3D pipe and not in the memory,
 * the MMU or the submit path.
 * ------------------------------------------------------------------ */
int gc_draw_clear(const struct gc_target *t, uint32_t argb, uint32_t *fence_out)
{
	struct gc_cmd c;
	int rc = check_target(t, true);

	if (rc) {
		return rc;
	}
	rc = cmd_begin(&c);
	if (rc) {
		return rc;
	}

	/* Mesa brackets the resolve with a colour and depth flush and a stall
	 * from the rasteriser to the pixel engine, so that anything the 3D
	 * pipe still owes the surface lands before the blitter overwrites it. */
	gc_cmd_state(&c, VIVS_GL_FLUSH_CACHE,
		     VIVS_GL_FLUSH_CACHE_COLOR | VIVS_GL_FLUSH_CACHE_DEPTH);
	emit_stall(&c, SYNC_RECIPIENT_RA, SYNC_RECIPIENT_PE);

	/* Source and destination are the same surface and the same format: a
	 * clear is a resolve whose pixels come from the fill value rather than
	 * from the source. 32 bits per pixel is A8R8G8B8 as far as the resolve
	 * engine is concerned; the X byte of the fill value is simply written. */
	gc_cmd_state(&c, VIVS_RS_CONFIG,
		     VIVS_RS_CONFIG_SOURCE_FORMAT(RS_FORMAT_A8R8G8B8) |
		     VIVS_RS_CONFIG_DEST_FORMAT(RS_FORMAT_A8R8G8B8));
	/* Strides are in bytes and are not shifted, because both layouts are
	 * linear (Mesa shifts only for a tiled surface). */
	gc_cmd_state(&c, VIVS_RS_SOURCE_STRIDE, t->stride);
	gc_cmd_state(&c, VIVS_RS_DEST_STRIDE, t->stride);
	gc_cmd_state(&c, VIVS_RS_PIPE_SOURCE_ADDR(0), t->gpu);
	gc_cmd_state(&c, VIVS_RS_PIPE_DEST_ADDR(0), t->gpu);
	gc_cmd_state(&c, VIVS_RS_PIPE_OFFSET(0), 0);
	gc_cmd_state(&c, VIVS_RS_PIPE_OFFSET(1), 0);
	gc_cmd_state(&c, VIVS_RS_WINDOW_SIZE,
		     VIVS_RS_WINDOW_SIZE_WIDTH(t->width) |
		     VIVS_RS_WINDOW_SIZE_HEIGHT(t->height));
	gc_cmd_state(&c, VIVS_RS_DITHER(0), 0xffffffff);
	gc_cmd_state(&c, VIVS_RS_DITHER(1), 0xffffffff);
	/* BITS is a per-bit write mask over the pixel; all ones replaces the
	 * whole pixel. MODE_ENABLED1 means "one fill value, not four". */
	gc_cmd_state(&c, VIVS_RS_CLEAR_CONTROL,
		     VIVS_RS_CLEAR_CONTROL_BITS(0xffff) |
		     VIVS_RS_CLEAR_CONTROL_MODE_ENABLED1);
	for (uint32_t i = 0; i < VIVS_RS_FILL_VALUE__LEN; i++) {
		gc_cmd_state(&c, VIVS_RS_FILL_VALUE(i), argb);
	}
	gc_cmd_state(&c, VIVS_RS_EXTRA_CONFIG, VIVS_RS_EXTRA_CONFIG_AA(0) |
					       VIVS_RS_EXTRA_CONFIG_ENDIAN(ENDIAN_MODE_NO_SWAP));
	/* This core has the PE enhancement that needs the single-buffer bit
	 * around the kick. */
	gc_cmd_state(&c, VIVS_RS_SINGLE_BUFFER, VIVS_RS_SINGLE_BUFFER_ENABLE);
	gc_cmd_state(&c, VIVS_RS_KICKER, 0xbeebbeeb);   /* the kick value Mesa uses */
	gc_cmd_state(&c, VIVS_RS_SINGLE_BUFFER, 0);

	emit_stall(&c, SYNC_RECIPIENT_RA, SYNC_RECIPIENT_PE);

	LOG_DBG("clear: 0x%08x into 0x%08x %ux%u stride %u (%u dwords)", argb, t->gpu,
		t->width, t->height, t->stride, c.n);
	return cmd_submit(&c, GC_SUBMIT_FLUSH_COLOR, fence_out);
}

/* ------------------------------------------------------------------ *
 * M5: one triangle
 * ------------------------------------------------------------------ */

static struct gc_bo s_vs_code, s_ps_code, s_vbo;

static int ensure_triangle_objects(void)
{
	int rc;

	if (s_vs_code.cpu == NULL) {
		rc = gc_render_alloc(sizeof(gc_tri_vs_code), GC_ALIGN_SHADER, &s_vs_code);
		if (rc) {
			return rc;
		}
		memcpy(s_vs_code.cpu, gc_tri_vs_code, sizeof(gc_tri_vs_code));
	}
	if (s_ps_code.cpu == NULL) {
		rc = gc_render_alloc(sizeof(gc_tri_ps_code), GC_ALIGN_SHADER, &s_ps_code);
		if (rc) {
			return rc;
		}
		memcpy(s_ps_code.cpu, gc_tri_ps_code, sizeof(gc_tri_ps_code));
	}
	if (s_vbo.cpu == NULL) {
		rc = gc_render_alloc(3 * sizeof(struct gc_vertex), GC_ALIGN_DEFAULT, &s_vbo);
		if (rc) {
			return rc;
		}
	}
	return 0;
}

/* The two vec4 attributes of one interleaved stream: position at offset 0,
 * colour at offset 16, stride 32.
 *
 * NUM is a two-bit field, so four components encode as 0. That is the
 * hardware's encoding, not an oversight: Mesa writes the component count
 * masked to two bits.
 */
static void emit_vertex_elements(struct gc_cmd *c)
{
	const uint32_t stride = sizeof(struct gc_vertex);
	uint32_t cfg0[GC_TRI_NUM_ATTRIBUTES];
	uint32_t cfg1[GC_TRI_NUM_ATTRIBUTES];
	uint32_t scale[GC_TRI_NUM_ATTRIBUTES];

	cfg0[0] = VIVS_NFE_GENERIC_ATTRIB_CONFIG0_TYPE(FE_DATA_TYPE_FLOAT) |
		  VIVS_NFE_GENERIC_ATTRIB_CONFIG0_ENDIAN(ENDIAN_MODE_NO_SWAP) |
		  VIVS_NFE_GENERIC_ATTRIB_CONFIG0_STREAM(0) |
		  VIVS_NFE_GENERIC_ATTRIB_CONFIG0_NUM(4) |
		  VIVS_NFE_GENERIC_ATTRIB_CONFIG0_NORMALIZE_OFF |
		  VIVS_NFE_GENERIC_ATTRIB_CONFIG0_START(offsetof(struct gc_vertex, pos));
	/* END is where this run of byte-contiguous attributes stops. */
	cfg1[0] = VIVS_NFE_GENERIC_ATTRIB_CONFIG1_END(offsetof(struct gc_vertex, color));

	cfg0[1] = VIVS_NFE_GENERIC_ATTRIB_CONFIG0_TYPE(FE_DATA_TYPE_FLOAT) |
		  VIVS_NFE_GENERIC_ATTRIB_CONFIG0_ENDIAN(ENDIAN_MODE_NO_SWAP) |
		  VIVS_NFE_GENERIC_ATTRIB_CONFIG0_STREAM(0) |
		  VIVS_NFE_GENERIC_ATTRIB_CONFIG0_NUM(4) |
		  VIVS_NFE_GENERIC_ATTRIB_CONFIG0_NORMALIZE_OFF |
		  VIVS_NFE_GENERIC_ATTRIB_CONFIG0_START(offsetof(struct gc_vertex, color));
	/* The last attribute of a stream ends the run. */
	cfg1[1] = VIVS_NFE_GENERIC_ATTRIB_CONFIG1_NONCONSECUTIVE |
		  VIVS_NFE_GENERIC_ATTRIB_CONFIG1_END(stride);

	/* 1.0 for float and normalised attributes. */
	scale[0] = fui(1.0f);
	scale[1] = fui(1.0f);

	gc_cmd_state_multi(c, VIVS_NFE_GENERIC_ATTRIB_CONFIG0(0), cfg0, GC_TRI_NUM_ATTRIBUTES);
	gc_cmd_state_multi(c, VIVS_NFE_GENERIC_ATTRIB_SCALE(0), scale, GC_TRI_NUM_ATTRIBUTES);
	gc_cmd_state_multi(c, VIVS_NFE_GENERIC_ATTRIB_CONFIG1(0), cfg1, GC_TRI_NUM_ATTRIBUTES);
}

/*
 * The viewport, the scissor and the clip box.
 *
 * Y points down in memory: the vertical scale is negative so that clip-space
 * y = +1 lands on row 0, the first row of the buffer. That is the convention
 * a scan-out buffer wants (row 0 at the top of the screen) and it makes a
 * read-back check read the way the picture looks.
 *
 * The four constants added to the scissor and clip registers are Mesa's
 * empirical margins (ETNA_SE_SCISSOR_MARGIN_* and ETNA_SE_CLIP_MARGIN_*),
 * copied verbatim: nothing documents them.
 */
static void emit_viewport(struct gc_cmd *c, const struct gc_target *t)
{
	const float half_w = (float)t->width / 2.0f;
	const float half_h = (float)t->height / 2.0f;

	gc_cmd_state_fixp(c, VIVS_PA_VIEWPORT_SCALE_X, fixp16(half_w));
	gc_cmd_state_fixp(c, VIVS_PA_VIEWPORT_SCALE_Y, fixp16(-half_h));
	/* Z is a plain float, and doubled: this GPU maps clip z 0..1 to 0..1
	 * while GL gives it -1..1, so Mesa rewrites the depth range here. */
	gc_cmd_state(c, VIVS_PA_VIEWPORT_SCALE_Z, fui(0.5f * 2.0f));
	gc_cmd_state_fixp(c, VIVS_PA_VIEWPORT_OFFSET_X, fixp16(half_w));
	gc_cmd_state_fixp(c, VIVS_PA_VIEWPORT_OFFSET_Y, fixp16(half_h));
	gc_cmd_state(c, VIVS_PA_VIEWPORT_OFFSET_Z, fui(0.5f - 0.5f));

	gc_cmd_state_fixp(c, VIVS_SE_SCISSOR_LEFT, 0);
	gc_cmd_state_fixp(c, VIVS_SE_SCISSOR_TOP, 0);
	gc_cmd_state_fixp(c, VIVS_SE_SCISSOR_RIGHT, (t->width << 16) + 0x1119);
	gc_cmd_state_fixp(c, VIVS_SE_SCISSOR_BOTTOM, (t->height << 16) + 0x1111);
	gc_cmd_state(c, VIVS_SE_DEPTH_SCALE, fui(0.0f));
	gc_cmd_state(c, VIVS_SE_DEPTH_BIAS, fui(0.0f));
	gc_cmd_state(c, VIVS_SE_CONFIG, 0);
	gc_cmd_state_fixp(c, VIVS_SE_CLIP_RIGHT, (t->width << 16) + 0xffff);
	gc_cmd_state_fixp(c, VIVS_SE_CLIP_BOTTOM, (t->height << 16) + 0xffff);
}

/* The render target: no depth, no stencil, no blending, no tile status. */
static void emit_framebuffer(struct gc_cmd *c, const struct gc_target *t)
{
	/* DEPTH_MODE_NONE means there is no depth buffer, DISABLE_ZS switches
	 * the depth and stencil unit off. The DEPTH_FUNC field takes the same
	 * numbering as GL's comparison functions, so 7 is ALWAYS (Mesa writes
	 * the gallium enum into it unchanged); with no depth buffer it is
	 * inert, but leaving it 0 would mean NEVER. */
	gc_cmd_state(c, VIVS_PE_DEPTH_CONFIG,
		     VIVS_PE_DEPTH_CONFIG_DEPTH_MODE_NONE |
		     VIVS_PE_DEPTH_CONFIG_DEPTH_FUNC(7) |
		     VIVS_PE_DEPTH_CONFIG_DISABLE_ZS);
	gc_cmd_state(c, VIVS_PE_DEPTH_NEAR, fui(0.0f));
	gc_cmd_state(c, VIVS_PE_DEPTH_FAR, fui(1.0f));
	gc_cmd_state(c, VIVS_PE_DEPTH_NORMALIZE, 0);
	gc_cmd_state(c, VIVS_PE_DEPTH_STRIDE, 0);
	gc_cmd_state(c, VIVS_PE_STENCIL_OP, 0);
	gc_cmd_state(c, VIVS_PE_STENCIL_CONFIG, VIVS_PE_STENCIL_CONFIG_MODE_DISABLED);
	gc_cmd_state(c, VIVS_PE_ALPHA_OP, 0);
	gc_cmd_state(c, VIVS_PE_ALPHA_BLEND_COLOR, 0);
	gc_cmd_state(c, VIVS_PE_ALPHA_CONFIG, 0);
	/* OVERWRITE says the draw replaces the target rather than reading it
	 * back; COMPONENTS is the colour mask, all four enabled. No tiling
	 * bits: the surface is linear. */
	gc_cmd_state(c, VIVS_PE_COLOR_FORMAT,
		     VIVS_PE_COLOR_FORMAT_FORMAT(PE_FORMAT_X8R8G8B8) |
		     VIVS_PE_COLOR_FORMAT_COMPONENTS(0xf) |
		     VIVS_PE_COLOR_FORMAT_OVERWRITE);
	gc_cmd_state(c, VIVS_PE_COLOR_STRIDE, t->stride);
	gc_cmd_state(c, VIVS_PE_HDEPTH_CONTROL, 0);
	/* One pixel pipe, so only address 0. */
	gc_cmd_state(c, VIVS_PE_PIPE_COLOR_ADDR(0), t->gpu);
	gc_cmd_state(c, VIVS_PE_STENCIL_CONFIG_EXT, 0);
	/* SINGLE_BUFFER(1) is what makes the pixel engine write a LINEAR
	 * layout. Without it the picture is tiled, which reads back as noise.
	 * The 0x000e4000 is Mesa's constant remainder of the register. */
	gc_cmd_state(c, VIVS_PE_LOGIC_OP,
		     VIVS_PE_LOGIC_OP_OP(LOGIC_OP_COPY) | 0x000e4000 |
		     VIVS_PE_LOGIC_OP_SINGLE_BUFFER(1));
	gc_cmd_state(c, VIVS_PE_DITHER(0), 0xffffffff);
	gc_cmd_state(c, VIVS_PE_DITHER(1), 0xffffffff);
	gc_cmd_state(c, VIVS_PE_ALPHA_COLOR_EXT0, 0);
	gc_cmd_state(c, VIVS_PE_ALPHA_COLOR_EXT1, 0);
	gc_cmd_state(c, VIVS_PE_STENCIL_CONFIG_EXT2, 0);
	gc_cmd_state(c, VIVS_PE_MEM_CONFIG, 0);
	/* Tile status off. Nothing else in this driver writes a TS register,
	 * so this one zero is the whole "no fast clear" decision. */
	gc_cmd_state(c, VIVS_TS_MEM_CONFIG, 0);
	gc_cmd_state(c, VIVS_TS_COLOR_CLEAR_VALUE, 0);
	gc_cmd_state(c, VIVS_TS_DEPTH_CLEAR_VALUE, 0);
	gc_cmd_state(c, VIVS_TS_COLOR_CLEAR_VALUE_EXT, 0);
}

/* The shader pipeline: counts, the varying map, and the instruction cache.
 * The HALTI5 registers only; the pre-HALTI5 set must stay untouched. */
static void emit_shaders(struct gc_cmd *c)
{
	const uint32_t vs_out_count = 1 + GC_TRI_NUM_VARYINGS;
	const uint32_t vs_insns = ARRAY_SIZE(gc_tri_vs_code) / 4;
	const uint32_t ps_insns = ARRAY_SIZE(gc_tri_ps_code) / 4;
	uint32_t out_regs[VIVS_VS_HALTI5_OUTPUT__LEN] = { 0 };
	uint32_t in_regs[4] = { 0 };

	gc_cmd_state(c, VIVS_VS_OUTPUT_COUNT, vs_out_count);
	gc_cmd_state(c, VIVS_VS_INPUT_COUNT,
		     VIVS_VS_INPUT_COUNT_COUNT(GC_TRI_NUM_ATTRIBUTES) |
		     VIVS_VS_INPUT_COUNT_UNK8(GC_TRI_VS_INPUT_COUNT_UNK8));
	gc_cmd_state(c, VIVS_VS_TEMP_REGISTER_CONTROL,
		     VIVS_VS_TEMP_REGISTER_CONTROL_NUM_TEMPS(GC_TRI_VS_NUM_TEMPS));
	gc_cmd_state(c, VIVS_VS_LOAD_BALANCING, GC_TRI_VS_LOAD_BALANCING);

	gc_cmd_state(c, VIVS_PA_ATTRIBUTE_ELEMENT_COUNT,
		     VIVS_PA_ATTRIBUTE_ELEMENT_COUNT_COUNT(GC_TRI_NUM_VARYINGS));
	/* Smooth shading, no culling, solid fill. Cull winding is folded into
	 * the cull mode on this hardware; with culling off it does not exist. */
	gc_cmd_state(c, VIVS_PA_CONFIG,
		     VIVS_PA_CONFIG_SHADE_MODEL_SMOOTH | VIVS_PA_CONFIG_CULL_FACE_MODE_OFF |
		     VIVS_PA_CONFIG_FILL_MODE_SOLID | VIVS_PA_CONFIG_WIDE_LINE);
	gc_cmd_state(c, VIVS_PA_LINE_WIDTH, fui(0.5f));
	gc_cmd_state(c, VIVS_PA_POINT_SIZE, fui(0.5f));
	gc_cmd_state(c, VIVS_PA_SYSTEM_MODE,
		     VIVS_PA_SYSTEM_MODE_PROVOKING_VERTEX_LAST |
		     VIVS_PA_SYSTEM_MODE_HALF_PIXEL_CENTER);
	gc_cmd_state(c, VIVS_PA_WIDE_LINE_WIDTH0, fui(0.5f));
	gc_cmd_state(c, VIVS_PA_WIDE_LINE_WIDTH1, fui(0.5f));

	gc_cmd_state(c, VIVS_RA_CONTROL, VIVS_RA_CONTROL_UNK0);
	/* No early depth test and no early depth write: there is no depth
	 * buffer, and the rasteriser and the pixel engine disagree about a
	 * linear layout anyway (Mesa disables early Z for linear targets). */
	gc_cmd_state(c, VIVS_RA_EARLY_DEPTH,
		     0x00000030 | VIVS_RA_EARLY_DEPTH_FORWARD_W |
		     VIVS_RA_EARLY_DEPTH_FORWARD_Z | VIVS_RA_EARLY_DEPTH_WRITE_DISABLE);

	gc_cmd_state(c, VIVS_PS_OUTPUT_REG,
		     VIVS_PS_OUTPUT_REG_0(GC_TRI_PS_COLOR_OUT_REG));
	gc_cmd_state(c, VIVS_PS_INPUT_COUNT,
		     VIVS_PS_INPUT_COUNT_COUNT(GC_TRI_NUM_VARYINGS + 1) |
		     VIVS_PS_INPUT_COUNT_UNK8(GC_TRI_PS_INPUT_COUNT_UNK8));
	gc_cmd_state(c, VIVS_PS_TEMP_REGISTER_CONTROL,
		     VIVS_PS_TEMP_REGISTER_CONTROL_NUM_TEMPS(GC_TRI_PS_NUM_TEMPS));
	/* One render target, and its output is clamped because the format is
	 * unsigned normalised. */
	gc_cmd_state(c, VIVS_PS_CONTROL,
		     VIVS_PS_CONTROL_SATURATE_RT0 | VIVS_PS_CONTROL_RT_COUNT(0));
	gc_cmd_state(c, VIVS_PS_OUTPUT_REG2, 0);
	gc_cmd_state(c, VIVS_PS_CONTROL_EXT,
		     VIVS_PS_CONTROL_EXT_OUTPUT_MODE0(COLOR_OUTPUT_MODE_NORMAL));
	/* Alpha to coverage off. */
	gc_cmd_state(c, VIVS_PS_MSAA_CONFIG, 0xffff6fff);

	gc_cmd_state(c, VIVS_GL_VARYING_TOTAL_COMPONENTS,
		     VIVS_GL_VARYING_TOTAL_COMPONENTS_NUM(
			     ROUND_UP(GC_TRI_VARYING0_COMPONENTS, 2)));

	/* --- the HALTI5-only block --- */
	gc_cmd_state(c, VIVS_FE_HALTI5_ID_CONFIG, 0);   /* no gl_VertexID */
	gc_cmd_state(c, VIVS_VS_HALTI5_OUTPUT_COUNT,
		     VIVS_VS_HALTI5_OUTPUT_COUNT_COUNT(vs_out_count) |
		     VIVS_VS_HALTI5_OUTPUT_COUNT_B(vs_out_count * 0x10));
	gc_cmd_state(c, VIVS_VS_HALTI5_UNK008A0,
		     0x0001000e | ((0x110u / vs_out_count) << 20));
	/* Both maps are byte arrays, four entries per register. The vertex
	 * shader's position comes out of its slot 0 and the colour varying out
	 * of slot 1; the inputs land in the same two registers, which is why
	 * the shader itself is a single NOP. */
	out_regs[0] = GC_TRI_VS_POS_OUT_REG | (GC_TRI_VS_VARYING0_REG << 8);
	in_regs[0] = GC_TRI_VS_INPUT0_REG | (GC_TRI_VS_INPUT1_REG << 8);
	gc_cmd_state_multi(c, VIVS_VS_HALTI5_OUTPUT(0), out_regs, ARRAY_SIZE(out_regs));
	gc_cmd_state_multi(c, VIVS_VS_HALTI5_INPUT(0), in_regs, ARRAY_SIZE(in_regs));

	/* Four bits per varying: how many components it has. */
	gc_cmd_state(c, VIVS_PA_VARYING_NUM_COMPONENTS(0), GC_TRI_VARYING0_COMPONENTS);
	gc_cmd_state(c, VIVS_PA_VARYING_NUM_COMPONENTS(1), 0);
	gc_cmd_state(c, VIVS_PA_VS_OUTPUT_COUNT, vs_out_count);
	gc_cmd_state(c, VIVS_PS_VARYING_NUM_COMPONENTS(0), GC_TRI_VARYING0_COMPONENTS);
	gc_cmd_state(c, VIVS_PS_VARYING_NUM_COMPONENTS(1), 0);
	/* No point size out and no point coordinate in: both fields all ones. */
	gc_cmd_state(c, VIVS_GL_HALTI5_SH_SPECIALS, 0x7f7f0000 | (0x7fu << 8));
	/* Four bits of interpolation mode per varying COMPONENT; smooth is 0,
	 * so one register of zeroes covers the four components. */
	gc_cmd_state(c, VIVS_GL_HALTI5_SHADER_ATTRIBUTES(0), 0);

	/* --- the instruction cache --- *
	 * HALTI5 has no instruction memory: the shader is fetched from RAM.
	 * RANGE_HIGH takes an instruction count, ICACHE_COUNT one less. */
	gc_cmd_state(c, VIVS_VS_NEWRANGE_LOW, 0);
	gc_cmd_state(c, VIVS_VS_HALTI5_RANGE_HIGH, vs_insns);
	gc_cmd_state(c, VIVS_VS_INST_ADDR, s_vs_code.gpu);
	gc_cmd_state(c, VIVS_SH_CONFIG, VIVS_SH_CONFIG_RTNE_ROUNDING);
	gc_cmd_state(c, VIVS_SH_ICACHE_CONTROL, VIVS_SH_ICACHE_CONTROL_ENABLE);
	gc_cmd_state(c, VIVS_VS_ICACHE_COUNT, vs_insns - 1);

	gc_cmd_state(c, VIVS_PS_NEWRANGE_LOW, 0);
	gc_cmd_state(c, VIVS_PS_HALTI5_RANGE_HIGH, ps_insns);
	gc_cmd_state(c, VIVS_PS_INST_ADDR, s_ps_code.gpu);
	gc_cmd_state(c, VIVS_SH_CONFIG, VIVS_SH_CONFIG_RTNE_ROUNDING);
	gc_cmd_state(c, VIVS_SH_ICACHE_CONTROL, VIVS_SH_ICACHE_CONTROL_ENABLE);
	gc_cmd_state(c, VIVS_PS_ICACHE_COUNT, ps_insns - 1);

	/* No uniforms, but the bases still have to be defined. */
	gc_cmd_state(c, VIVS_VS_UNIFORM_BASE, 0);
	gc_cmd_state(c, VIVS_PS_UNIFORM_BASE, 0);

	/* HALTI5 has to be prompted to pre-fetch the shaders it was just
	 * pointed at; Mesa calls this mandatory. */
	gc_cmd_state(c, VIVS_VS_ICACHE_PREFETCH, 0);
	gc_cmd_state(c, VIVS_PS_ICACHE_PREFETCH, 0);
}

int gc_draw_triangle(const struct gc_target *t, const struct gc_vertex v[3],
		     uint32_t *fence_out)
{
	struct gc_cmd c;
	int rc = check_target(t, false);

	if (rc) {
		return rc;
	}
	rc = ensure_triangle_objects();
	if (rc) {
		return rc;
	}
	memcpy(s_vbo.cpu, v, 3 * sizeof(*v));

	rc = cmd_begin(&c);
	if (rc) {
		return rc;
	}

	emit_reset_state(&c);

	/* Flush what the previous submit left in the caches before the new
	 * state lands. Nothing here binds a texture, so the texture caches are
	 * left out of the cheap path. */
	gc_cmd_state(&c, VIVS_GL_FLUSH_CACHE,
		     VIVS_GL_FLUSH_CACHE_COLOR | VIVS_GL_FLUSH_CACHE_DEPTH |
		     VIVS_GL_FLUSH_CACHE_SHADER_L1);
	emit_stall(&c, SYNC_RECIPIENT_RA, SYNC_RECIPIENT_PE);

	/* One sample per pixel. The enable field is the sample mask. */
	gc_cmd_state(&c, VIVS_GL_MULTI_SAMPLE_CONFIG,
		     VIVS_GL_MULTI_SAMPLE_CONFIG_MSAA_SAMPLES_NONE |
		     VIVS_GL_MULTI_SAMPLE_CONFIG_MSAA_ENABLES(0xf));

	emit_vertex_elements(&c);
	gc_cmd_state(&c, VIVS_NFE_VERTEX_STREAMS_BASE_ADDR(0), s_vbo.gpu);
	gc_cmd_state(&c, VIVS_NFE_VERTEX_STREAMS_CONTROL(0),
		     FE_VERTEX_STREAM_CONTROL_VERTEX_STRIDE(sizeof(struct gc_vertex)));
	gc_cmd_state(&c, VIVS_NFE_VERTEX_STREAMS_VERTEX_DIVISOR(0), 0);

	emit_viewport(&c, t);
	emit_framebuffer(&c, t);

	/* The shader load wants the front end to have caught up with the pixel
	 * engine first. */
	emit_stall(&c, SYNC_RECIPIENT_FE, SYNC_RECIPIENT_PE);
	emit_shaders(&c);
	emit_stall(&c, SYNC_RECIPIENT_RA, SYNC_RECIPIENT_PE);

	gc_cmd_draw(&c, PRIMITIVE_TYPE_TRIANGLES, 0, 3);

	/* Make the pixels visible: flush the colour cache, drain the pixel
	 * engine twice over (the rasteriser first, then the front end) so the
	 * EVENT the render service appends cannot overtake them. */
	gc_cmd_state(&c, VIVS_GL_FLUSH_CACHE, VIVS_GL_FLUSH_CACHE_COLOR);
	emit_stall(&c, SYNC_RECIPIENT_RA, SYNC_RECIPIENT_PE);
	emit_stall(&c, SYNC_RECIPIENT_FE, SYNC_RECIPIENT_PE);

	LOG_DBG("triangle: into 0x%08x %ux%u (%u dwords, VS 0x%08x PS 0x%08x VBO 0x%08x)",
		t->gpu, t->width, t->height, c.n, s_vs_code.gpu, s_ps_code.gpu, s_vbo.gpu);
	return cmd_submit(&c, GC_SUBMIT_FLUSH_COLOR, fence_out);
}

uint32_t gc_target_pixel(const struct gc_target *t, uint32_t x, uint32_t y)
{
	uint8_t *row;

	if (t->cpu == NULL || x >= t->width || y >= t->height) {
		return 0;
	}
	row = (uint8_t *)t->cpu + (size_t)y * t->stride;
	/* A no-op on the carveout's non-cacheable mapping; needed when the
	 * target is a cached buffer the caller owns. */
	gc_render_cache_invd(row + x * 4u, 4);
	return *(uint32_t *)(row + x * 4u) & 0x00ffffffu;
}

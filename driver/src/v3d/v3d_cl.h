/*
 * SPDX-License-Identifier: Apache-2.0 AND MIT
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * SPDX-FileCopyrightText: 2014-2017 Broadcom
 * SPDX-FileCopyrightText: 2016 Intel Corporation
 * Ported from Mesa 26.2.1 src/broadcom/cle/v3d_packet.xml (the packet layouts)
 * and cle/v3d_packet_helpers.h (MIT).
 * The upstream part stays under its licence (LICENSES/MIT.txt); our
 * changes are Apache-2.0. See NOTICE.
 */
/*
 * V3D 4.2 control lists: YakoGL's adapter over Mesa's generated packers.
 *
 * The packet layouts are not written here. They are Mesa 26.2.1's
 * v3d_packet_v42_pack.h (generated/), which Mesa's own generator makes
 * from Mesa's v3d_packet.xml, both vendored unmodified under
 * third_party/mesa-26.2.1/src/broadcom/cle/:
 *
 *   python3 third_party/mesa-26.2.1/src/broadcom/cle/gen_pack_header.py \
 *       third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml 42 \
 *       > driver/src/v3d/generated/v3d_packet_v42_pack.h
 *
 * tools/check.sh regenerates it and requires the committed file byte for
 * byte. An update of Mesa is: replace third_party/mesa-<ver>/, regenerate,
 * fix this adapter.
 *
 * What this file adds is ours:
 *
 *  - struct v3d_cl, a list in CPU-mapped GPU memory that refuses to
 *    overflow (Mesa's gallium v3d_cl grows a BO instead);
 *  - the __gen_* hooks the generated code asks its user for: addresses are
 *    plain 32-bit GPU addresses, there are no relocations;
 *  - "correct or refused": Mesa's packers do not range-check once NDEBUG is
 *    set (and assert, which would stop the board, when it is not), so every
 *    packet is packed, unpacked with the generated unpacker and refused
 *    unless the unpacked values are the values given. A value too wide for
 *    its field, a misaligned address, a minus-one field given 0, a fixed
 *    point value that is not on the field's grid: each marks the list
 *    bad_field and writes nothing, never a truncated field;
 *  - v3d_cl_emit(), Mesa's cl_emit() idiom over struct v3d_cl:
 *
 *        v3d_cl_emit(cl, CLIP_WINDOW, clip) {
 *                clip.clip_window_left_pixel_coordinate = x;
 *                ...
 *        }
 *
 *    The packet is packed on the stack and copied into the list in one
 *    memcpy: the list is uncached GPU memory (the V3D OSAL's carve-out is
 *    Normal-NC), where a packer's byte stores and reads would each be a bus
 *    access (record 9c39f4c3: ~50 us a draw before the lists were staged).
 *
 * The composition (which packets, in which order, with which values) is in
 * v3d_frame.c and v3d_job.c, after Mesa's v3dx_draw.c, v3dx_emit.c and
 * v3dx_rcl.c, which are Gallium code and cannot be vendored.
 */
#ifndef V3D_CL_H
#define V3D_CL_H

#include <stdbool.h>
#include <stdint.h>
#include <string.h>

/* A control list being written: `buf` is the CPU view, `gpu` the address the
 * GPU sees for buf[0]. A write that does not fit sets `overflow` and writes
 * nothing, so a list that overflowed is never submitted by accident. The
 * layout is pinned for Rust by src/v3d/ffi_layout_check.c. */
struct v3d_cl {
	uint8_t *buf;
	uint32_t gpu;
	uint32_t size;
	uint32_t used;
	bool overflow;
	bool bad_field; /* a value did not fit its field, or an address was misaligned */
};

/* ---- Mesa's packers ------------------------------------------------------ */

/* The hooks gen_pack_header.py's output expects from its user. The same
 * definitions as tools/v3d/cl_golden.c and Mesa's clif (clif_private.h). */
#ifndef __gen_user_data
#define __gen_user_data void
#endif
#ifndef __gen_address_type
#define __gen_address_type uint32_t
#endif
#ifndef __gen_address_offset
#define __gen_address_offset(reloc) (*(reloc))
#endif
#ifndef __gen_emit_reloc
#define __gen_emit_reloc(cl, reloc) ((void)0)
#endif
#ifndef __gen_unpack_address
#define __gen_unpack_address(cl, s, e) ((uint32_t)(__gen_unpack_uint(cl, s, e) << (31 - ((e) - (s)))))
#endif

/* Mesa's util/u_endian.h learns the byte order from <endian.h> when the
 * build defines HAVE_ENDIAN_H (Mesa's meson does, on glibc); a Zephyr or
 * bare-metal build has no such header, so the compiler's own macro says. */
#if !defined(UTIL_ARCH_LITTLE_ENDIAN) && !defined(HAVE_ENDIAN_H)
#if defined(__BYTE_ORDER__) && __BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__
#define UTIL_ARCH_LITTLE_ENDIAN 1
#define UTIL_ARCH_BIG_ENDIAN 0
#elif defined(__BYTE_ORDER__) && __BYTE_ORDER__ == __ORDER_BIG_ENDIAN__
#define UTIL_ARCH_LITTLE_ENDIAN 0
#define UTIL_ARCH_BIG_ENDIAN 1
#endif
#endif

/* The packers are compiled with NDEBUG whatever the build says: a value that
 * does not fit is refused below, not asserted (an assert would stop the
 * board). <assert.h> is included again afterwards so that the includer's
 * own assert() is what it was. Mesa's headers are not ours to keep free of
 * the warnings our builds turn on. */
#ifndef NDEBUG
#define V3D_CL_RESTORE_ASSERT
#define NDEBUG
#endif
#include <assert.h>
/* Mesa's util/macros.h and Zephyr's sys/util.h both define CLAMP and
 * DIV_ROUND_UP. When macros.h comes in here and the includer already has its
 * own definitions (Zephyr's, say), those are set aside while Mesa's headers
 * are read and put back afterwards; otherwise Mesa's stay, for any Mesa
 * header the includer reads later. */
#if !defined(UTIL_MACROS_H) && defined(CLAMP)
#define V3D_CL_RESTORE_CLAMP
#pragma push_macro("CLAMP")
#undef CLAMP
#endif
#if !defined(UTIL_MACROS_H) && defined(DIV_ROUND_UP)
#define V3D_CL_RESTORE_DIV_ROUND_UP
#pragma push_macro("DIV_ROUND_UP")
#undef DIV_ROUND_UP
#endif
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunused-parameter"
#pragma GCC diagnostic ignored "-Wsign-compare"
#pragma GCC diagnostic ignored "-Wshift-negative-value"
#include "generated/v3d_packet_v42_pack.h"
#include "v3d_cl_packets.h"
#pragma GCC diagnostic pop
#ifdef V3D_CL_RESTORE_CLAMP
#undef V3D_CL_RESTORE_CLAMP
#undef CLAMP
#pragma pop_macro("CLAMP")
#endif
#ifdef V3D_CL_RESTORE_DIV_ROUND_UP
#undef V3D_CL_RESTORE_DIV_ROUND_UP
#undef DIV_ROUND_UP
#pragma pop_macro("DIV_ROUND_UP")
#endif
#ifdef V3D_CL_RESTORE_ASSERT
#undef V3D_CL_RESTORE_ASSERT
#undef NDEBUG
#include <assert.h>
#endif

/* The generated header names a field's values as plain macros inside its
 * structs (NONE, Z, STENCIL, RENDER_TARGET_0, INDEX_TYPE_8_BIT, ...), which
 * would take those words from every file that includes this one, Zephyr's
 * and the samples' included. The values YakoGL uses are kept under V3D_
 * names, from the macros themselves; then the macros go. */
enum {
	V3D_STORE_BUFFER_RT0 = RENDER_TARGET_0, /* render target n is n (0..3) */
	V3D_STORE_BUFFER_NONE = NONE,
	V3D_STORE_BUFFER_Z = Z,
	V3D_STORE_BUFFER_STENCIL = STENCIL,
	V3D_STORE_BUFFER_ZSTENCIL = ZSTENCIL,
	V3D_LOAD_BUFFER_RT0 = RENDER_TARGET_0,
	V3D_LOAD_BUFFER_NONE = NONE,
	V3D_LOAD_BUFFER_Z = Z,
	V3D_LOAD_BUFFER_STENCIL = STENCIL,
	V3D_LOAD_BUFFER_ZSTENCIL = ZSTENCIL,
	V3D_PRIM_LIST_TRIANGLES = LIST_TRIANGLES,
	V3D_INDEX_TYPE_8 = INDEX_TYPE_8_BIT,
	V3D_INDEX_TYPE_16 = INDEX_TYPE_16_BIT,
	V3D_INDEX_TYPE_32 = INDEX_TYPE_32_BIT,
	V3D_ATTRIBUTE_FLOAT = ATTRIBUTE_FLOAT,
	V3D_TILE_ALLOC_BLOCK_64B = TILE_ALLOCATION_BLOCK_SIZE_64B,
	V3D_TILE_ALLOC_BLOCK_128B = TILE_ALLOCATION_BLOCK_SIZE_128B,
};
_Static_assert(TILE_ALLOCATION_INITIAL_BLOCK_SIZE_128B == TILE_ALLOCATION_BLOCK_SIZE_128B &&
		       TILE_ALLOCATION_INITIAL_BLOCK_SIZE_64B == TILE_ALLOCATION_BLOCK_SIZE_64B,
	       "one numbering for both tile allocation block sizes");
#undef ATTRIBUTE_BYTE
#undef ATTRIBUTE_FIXED
#undef ATTRIBUTE_FLOAT
#undef ATTRIBUTE_HALF_FLOAT
#undef ATTRIBUTE_INT
#undef ATTRIBUTE_INT2_10_10_10
#undef ATTRIBUTE_SHORT
#undef BLOCK_SIZE_128B
#undef BLOCK_SIZE_256B
#undef BLOCK_SIZE_64B
#undef EARLY_Z_DIRECTION_GT_GE
#undef EARLY_Z_DIRECTION_LT_LE
#undef GEOMETRY_SHADER_LINE_STRIP
#undef GEOMETRY_SHADER_POINTS
#undef GEOMETRY_SHADER_TRI_STRIP
#undef INDEX_TYPE_16_BIT
#undef INDEX_TYPE_32_BIT
#undef INDEX_TYPE_8_BIT
#undef LIST_LINES
#undef LIST_POINTS
#undef LIST_TRIANGLES
#undef LOAD_BUFFER_STATE
#undef NONE
#undef RENDER_TARGET_0
#undef RENDER_TARGET_1
#undef RENDER_TARGET_2
#undef RENDER_TARGET_3
#undef RENDER_TARGET_MAXIMUM_128BPP
#undef RENDER_TARGET_MAXIMUM_32BPP
#undef RENDER_TARGET_MAXIMUM_64BPP
#undef STENCIL
#undef STORE_BUFFER_STATE
#undef STORE_BUFFER_STATE_CL
#undef STORE_PRIMITIVE_COUNTS
#undef STORE_PRIMITIVE_COUNTS_AND_ZERO
#undef SWIZZLE_ALPHA
#undef SWIZZLE_BLUE
#undef SWIZZLE_GREEN
#undef SWIZZLE_ONE
#undef SWIZZLE_RED
#undef SWIZZLE_ZERO
#undef TESSELLATION_EDGE_SPACING_EVEN
#undef TESSELLATION_EDGE_SPACING_FRACTIONAL_EVEN
#undef TESSELLATION_EDGE_SPACING_FRACTIONAL_ODD
#undef TESSELLATION_TYPE_ISOLINES
#undef TESSELLATION_TYPE_QUADS
#undef TESSELLATION_TYPE_TRIANGLE
#undef TILE_ALLOCATION_BLOCK_SIZE_128B
#undef TILE_ALLOCATION_BLOCK_SIZE_256B
#undef TILE_ALLOCATION_BLOCK_SIZE_64B
#undef TILE_ALLOCATION_INITIAL_BLOCK_SIZE_128B
#undef TILE_ALLOCATION_INITIAL_BLOCK_SIZE_256B
#undef TILE_ALLOCATION_INITIAL_BLOCK_SIZE_64B
#undef VPM_SETUP_SIZE_16_BIT
#undef VPM_SETUP_SIZE_32_BIT
#undef VPM_SETUP_SIZE_8_BIT
#undef Z
#undef ZSTENCIL

/* ---- the list ------------------------------------------------------------ */

static inline void v3d_cl_init(struct v3d_cl *cl, void *buf, uint32_t gpu, uint32_t size)
{
	cl->buf = (uint8_t *)buf;
	cl->gpu = gpu;
	cl->size = size;
	cl->used = 0;
	cl->overflow = false;
	cl->bad_field = false;
}

/* True when every packet was written whole with every field in range. */
static inline bool v3d_cl_ok(const struct v3d_cl *cl)
{
	return !cl->overflow && !cl->bad_field;
}

/* The GPU address of the next byte to be written. */
static inline uint32_t v3d_cl_gpu_next(const struct v3d_cl *cl)
{
	return cl->gpu + cl->used;
}

/*
 * Reserve `len` zeroed bytes at the next multiple of `align` (a power of two),
 * for what has no opcode (the uniform streams, the default attribute values)
 * or must be aligned (the shader state record). The padding is zeroed too.
 * Returns NULL and marks the list overflowed when it does not fit.
 */
static inline uint8_t *v3d_cl_reserve(struct v3d_cl *cl, uint32_t len, uint32_t align)
{
	uint32_t at = (cl->gpu + cl->used + align - 1) & ~(align - 1);
	uint32_t pad = at - (cl->gpu + cl->used);
	if (cl->overflow || align == 0 || (align & (align - 1)) || pad > cl->size - cl->used ||
	    len > cl->size - cl->used - pad) {
		cl->overflow = true;
		return NULL;
	}
	uint8_t *p = cl->buf + cl->used;
	memset(p, 0, pad + len);
	cl->used += pad + len;
	return p + pad;
}

/* Store `len` packed bytes at the list's current position: all of them, or,
 * when they do not fit, none (overflow). `ok` false (the packer refused a
 * value) stores none and marks the list bad. */
static inline void v3d_cl_put(struct v3d_cl *cl, const uint8_t *b, uint32_t len, bool ok)
{
	if (cl->overflow || len > cl->size - cl->used) {
		cl->overflow = true;
		return;
	}
	if (!ok) {
		cl->bad_field = true;
		return;
	}
	memcpy(cl->buf + cl->used, b, len);
	cl->used += len;
}

/* ---- the checked packers ------------------------------------------------- */

/*
 * v3d_pack_<NAME>(out, values): Mesa's V3D42_<NAME>_pack into `out`
 * (V3D42_<NAME>_length bytes), then V3D42_<NAME>_unpack of those bytes; true
 * only when that gives back `values` exactly. The copy the unpacker writes
 * into starts as `values` itself, so struct padding compares equal.
 *
 * v3d_cl_put_<NAME>(cl, values): the same, stored into the list.
 */
#define V3D_CL_PACKER(name)                                                                        \
	static inline bool v3d_pack_##name(uint8_t *out, const struct V3D42_##name *v)             \
	{                                                                                          \
		struct V3D42_##name back;                                                          \
		memcpy(&back, v, sizeof(back));                                                    \
		V3D42_##name##_pack(NULL, out, v);                                                 \
		V3D42_##name##_unpack(out, &back);                                                 \
		return memcmp(&back, v, sizeof(back)) == 0;                                        \
	}                                                                                          \
	static inline void v3d_cl_put_##name(struct v3d_cl *cl, const struct V3D42_##name *v)      \
	{                                                                                          \
		uint8_t b[V3D42_##name##_length];                                                  \
		bool ok = v3d_pack_##name(b, v);                                                   \
		v3d_cl_put(cl, b, V3D42_##name##_length, ok);                                      \
	}

/* Mesa's cl_emit(): the body fills `var`, a struct V3D42_<NAME> whose opcode
 * (and sub-id) the generated header sets; the packet is stored when the body
 * ends. A record (no opcode) is written the same way at the list's current
 * position; the caller aligns it first (v3d_cl_reserve). */
#define v3d_cl_emit(cl, name, var)                                                                 \
	for (struct V3D42_##name var = { V3D42_##name##_header }, *var##_emit_once = &var;         \
	     var##_emit_once != NULL; v3d_cl_put_##name((cl), &var), var##_emit_once = NULL)

/* The packets and records YakoGL writes (v3d_cl_packets.h). */
V3D_CL_PACKETS(V3D_CL_PACKER)

/* ---- values on a field's grid -------------------------------------------- */

static inline uint32_t v3d_cl_float_bits(float f)
{
	uint32_t u;
	memcpy(&u, &f, sizeof(u));
	return u;
}

/* An "f187" field (SAMPLE_STATE's coverage, DEPTH_OFFSET's factor and units)
 * holds the top 16 bits of a float; Mesa's packer truncates to them
 * (fui(x) >> 16). The value the field will hold, so that the checked packer
 * sees the value it gives back. */
static inline float v3d_cl_f187(float f)
{
	uint32_t u = v3d_cl_float_bits(f) & 0xffff0000u;
	float out;
	memcpy(&out, &u, sizeof(out));
	return out;
}

/* A float into an unsigned 14.8 fixed-point value ("u14.8", VIEWPORT_OFFSET's
 * fine x/y), rounded as Mesa's util_bitpack_ufixed does (llroundf: half away
 * from zero), as the raw 22-bit number. A value below 0, above the field or
 * NaN is refused (false), never clamped. The product is exact in double (a
 * float times 2^8), so the result equals llroundf for every accepted value
 * without libm. The packet takes raw / 256.0f, which is exact. */
static inline bool v3d_float_to_u14_8(float v, uint32_t *out)
{
	if (!(v >= 0.0f))
		return false; /* negative or NaN */
	double x = (double)v * 256.0 + 0.5;
	if (x >= (double)(1u << 22))
		return false;
	*out = (uint32_t)x;
	return true;
}

#endif

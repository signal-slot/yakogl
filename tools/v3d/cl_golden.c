/* SPDX-License-Identifier: MIT
 *
 * cl_golden: checks YakoGL's V3D 4.2 control-list adapter
 * (driver/src/v3d/v3d_cl.h over the committed, generated
 * v3d_packet_v42_pack.h) against the packers of Mesa's own build
 * (build-v3d/src/broadcom/cle/v3d_packet_v42_pack.h, compiled apart in
 * cl_golden_mesa.c), for every packet and record the driver writes
 * (v3d_cl_packets.h):
 *
 *  1. the struct is the same size in both;
 *  2. valid values are never refused and pack to Mesa's bytes: 20,000 random
 *     byte strings per packet (plus all-zero and all-ones), each unpacked by
 *     Mesa's unpacker into the values it holds, then written through
 *     v3d_cl_put_<name>(): accepted, V3D42_<name>_length bytes, the bytes
 *     Mesa's packer gives for the same values;
 *  3. values that do not fit are refused, with nothing written: a field too
 *     wide, a signed field out of range, a misaligned address, a minus-one
 *     field given 0, a fixed-point value off its grid, an f187 value with
 *     low bits (and the same after v3d_cl_f187, accepted); an overflowing
 *     list writes nothing either, and a refusal does not abort (NDEBUG);
 *  4. v3d_cl_emit() writes what v3d_cl_put_<name>() writes, opcode included.
 *
 * That the committed header is what Mesa's generator makes from the vendored
 * v3d_packet.xml is tools/check.sh's regeneration row, which needs no Mesa
 * tree; this check needs a Mesa 26.2.1 tree with build-v3d
 * (tools/v3d/README.md):
 *
 *   M=<mesa>; B=$M/build-v3d; cc -O1 -std=c11 -D_GNU_SOURCE -DHAVE_ENDIAN_H \
 *      -I$M/src -I$M/include -I$M/src/broadcom -I$B/src/broadcom -I$B/src \
 *      -Idriver/src/v3d -Idriver/src/v3d/generated -Ithird_party/mesa-26.2.1/src/broadcom \
 *      -Ithird_party/mesa-26.2.1/src -Ithird_party/mesa-26.2.1/include \
 *      tools/v3d/cl_golden.c tools/v3d/cl_golden_mesa.c -lm -o cl_golden
 *
 * v3d_cl.h finds the committed header next to itself (a quoted include);
 * cl_golden_mesa.c asks for "cle/v3d_packet_v42_pack.h", which only the Mesa
 * build directory has.
 *
 * Exit status 0 only when every case passed AND the number of cases run is
 * the number planned.
 */
#include <stdio.h>
#include <stdint.h>
#include <stdbool.h>
#include <string.h>

#include "v3d_cl.h"

#define CASES 20000
#define CONTROLS 15

#define MESA_DECL(name)                                                                            \
	void mesa_pack_##name(uint8_t *out, const void *v);                                        \
	void mesa_unpack_##name(const uint8_t *in, void *v);                                       \
	extern const size_t mesa_size_##name, mesa_len_##name;
V3D_CL_PACKETS(MESA_DECL)

static uint64_t rng = 0x9e3779b97f4a7c15ull;
static uint8_t rnd8(void)
{
	rng ^= rng << 13;
	rng ^= rng >> 7;
	rng ^= rng << 17;
	return (uint8_t)(rng >> 24);
}

static int failures, cases_run, packets;

static void check(bool ok, const char *name, int i, const char *what)
{
	cases_run++;
	if (!ok && failures++ < 20)
		printf("cl_golden: FAIL %s case %d: %s\n", name, i, what);
}

/* A list over `mem` at GPU address 0x10000. */
static struct v3d_cl list(uint8_t *mem, uint32_t size)
{
	struct v3d_cl cl;
	memset(mem, 0xa5, size);
	v3d_cl_init(&cl, mem, 0x10000u, size);
	return cl;
}

/* 1, 2 and 4 for one packet. */
#define CHECK_PACKET(name)                                                                         \
	do {                                                                                       \
		packets++;                                                                         \
		check(sizeof(struct V3D42_##name) == mesa_size_##name &&                           \
			      V3D42_##name##_length == mesa_len_##name,                             \
		      #name, -1, "struct size or length differs from Mesa's build");               \
		for (int i = 0; i < CASES + 2; i++) {                                              \
			uint8_t in[V3D42_##name##_length], ref[V3D42_##name##_length];              \
			uint8_t mem[V3D42_##name##_length + 8];                                    \
			for (unsigned b = 0; b < sizeof(in); b++)                                  \
				in[b] = i == 0 ? 0 : i == 1 ? 0xff : rnd8();                       \
			struct V3D42_##name v;                                                     \
			memset(&v, 0, sizeof(v));                                                  \
			mesa_unpack_##name(in, &v);                                                \
			mesa_pack_##name(ref, &v);                                                 \
			struct v3d_cl cl = list(mem, sizeof(mem));                                 \
			v3d_cl_put_##name(&cl, &v);                                                \
			check(v3d_cl_ok(&cl) && cl.used == sizeof(ref) &&                          \
				      memcmp(mem, ref, sizeof(ref)) == 0 &&                         \
				      mem[sizeof(ref)] == 0xa5,                                     \
			      #name, i, "a valid value was refused or packed differently");        \
		}                                                                                  \
	} while (0)

/* 3: `fill` sets a field of `v` out of range; the list must be refused and
 * untouched. */
#define REFUSED(name, what, fill)                                                                  \
	do {                                                                                       \
		uint8_t mem[64];                                                                   \
		struct v3d_cl cl = list(mem, sizeof(mem));                                         \
		struct V3D42_##name v = { V3D42_##name##_header };                                 \
		fill;                                                                              \
		v3d_cl_put_##name(&cl, &v);                                                        \
		bool untouched = true;                                                             \
		for (unsigned b = 0; b < sizeof(mem); b++)                                         \
			untouched &= mem[b] == 0xa5;                                               \
		check(cl.bad_field && !cl.overflow && cl.used == 0 && untouched, #name, -1, what); \
	} while (0)

#define ACCEPTED(name, what, fill)                                                                 \
	do {                                                                                       \
		uint8_t mem[64];                                                                   \
		struct v3d_cl cl = list(mem, sizeof(mem));                                         \
		struct V3D42_##name v = { V3D42_##name##_header };                                 \
		fill;                                                                              \
		v3d_cl_put_##name(&cl, &v);                                                        \
		check(v3d_cl_ok(&cl) && cl.used == V3D42_##name##_length, #name, -1, what);        \
	} while (0)

static void controls(void)
{
	REFUSED(CLIP_WINDOW, "a 16-bit field given 0x10000", v.clip_window_width_in_pixels = 0x10000);
	REFUSED(STORE_TILE_BUFFER_GENERAL, "a 6-bit enum field given 64",
		v.output_image_format = (enum V3D42_Output_Image_Format)64);
	REFUSED(VIEWPORT_OFFSET, "a signed 10-bit field given 512", v.coarse_x = 512);
	REFUSED(VIEWPORT_OFFSET, "a signed 10-bit field given -513", v.coarse_y = -513);
	REFUSED(GL_SHADER_STATE, "an address that is not 32-byte aligned", v.address = 0x1010u);
	REFUSED(MULTICORE_RENDERING_TILE_LIST_SET_BASE, "an address that is not 64-byte aligned",
		v.address = 0x2020u);
	REFUSED(TILE_BINNING_MODE_CFG, "a minus-one field given 0",
		(v.width_in_pixels = 0, v.height_in_pixels = 1, v.number_of_render_targets = 1));
	REFUSED(VIEWPORT_OFFSET, "a u14.8 value off the 1/256 grid", v.fine_x = 0.3f);
	REFUSED(SAMPLE_STATE, "an f187 value with low mantissa bits", v.coverage = 1.1f);
	ACCEPTED(SAMPLE_STATE, "the same value as the f187 field holds it", v.coverage = v3d_cl_f187(1.1f));
	REFUSED(SAMPLER_STATE, "an s8.8 value off its grid", v.fixed_bias = 0.001f);
	ACCEPTED(TILE_BINNING_MODE_CFG, "the minus-one fields at 1",
		 (v.width_in_pixels = 1, v.height_in_pixels = 1, v.number_of_render_targets = 1));

	/* a list with no room: overflow, nothing written, not bad_field */
	{
		uint8_t mem[16];
		struct v3d_cl cl = list(mem, 8); /* CLIP_WINDOW is 9 bytes */
		struct V3D42_CLIP_WINDOW v = { V3D42_CLIP_WINDOW_header };
		v3d_cl_put_CLIP_WINDOW(&cl, &v);
		check(cl.overflow && !cl.bad_field && cl.used == 0 && mem[0] == 0xa5, "CLIP_WINDOW", -1,
		      "a packet that does not fit is written anyway");
	}
	/* v3d_cl_emit is v3d_cl_put with the header: the opcode is Mesa's */
	{
		uint8_t a[16], b[16];
		struct v3d_cl ca = list(a, sizeof(a)), cb = list(b, sizeof(b));
		v3d_cl_emit(&ca, CLIP_WINDOW, clip) {
			clip.clip_window_left_pixel_coordinate = 3;
			clip.clip_window_height_in_pixels = 7;
		}
		struct V3D42_CLIP_WINDOW v = { V3D42_CLIP_WINDOW_header };
		v.clip_window_left_pixel_coordinate = 3;
		v.clip_window_height_in_pixels = 7;
		uint8_t ref[V3D42_CLIP_WINDOW_length];
		mesa_pack_CLIP_WINDOW(ref, &v);
		v3d_cl_put_CLIP_WINDOW(&cb, &v);
		check(ca.used == 9 && a[0] == 107 && memcmp(a, ref, 9) == 0 && memcmp(a, b, 16) == 0,
		      "CLIP_WINDOW", -1, "v3d_cl_emit differs from the packer");
	}
	/* a sub-id packet's header carries its sub-id */
	{
		uint8_t a[16];
		struct v3d_cl ca = list(a, sizeof(a));
		v3d_cl_emit(&ca, TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES, zs) {
			zs.stencil_clear_value = 0x5a;
		}
		check(ca.used == 9 && a[0] == 121 && (a[1] & 0xf) == 2 && a[2] == 0x5a,
		      "TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES", -1, "the sub-id or a field is wrong");
	}
}

int main(void)
{
#define RUN(name) CHECK_PACKET(name);
	V3D_CL_PACKETS(RUN)
#undef RUN
	controls();
	const int planned = packets * (1 + CASES + 2) + CONTROLS;
	printf("cl_golden: %d packets and records\n", packets);
	printf("cl_golden: %d cases run (planned %d), %d failed\n", cases_run, planned, failures);
	return failures == 0 && cases_run == planned ? 0 : 1;
}

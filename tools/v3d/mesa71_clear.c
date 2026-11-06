/* SPDX-License-Identifier: MIT
 *
 * mesa71_clear: the V3D 7.1 clear-only frame as Mesa's v3d driver emits it,
 * packed here with Mesa's own 7.1 packers (v3d_packet_v71_pack.h, cl_emit at
 * V3D_VERSION 71), read back in full by Mesa's clif_dump for a v3d_device_info
 * of version 71, and compared, packet by packet, with the CLIF Mesa's driver
 * dumped on the drm-shim for the same clear: tools/v3d/shim71/clear.clif.
 *
 * The frame is the one in that dump: a 64x64 RGBA8888 TLB clear to
 * 0xffff0000 (low clear colour word), one 64x64 tile, no layers, no
 * multisampling; the BCL, the RCL with its two dummy tiles and the generic
 * tile list. The BO addresses are fixed fakes (the ones of the dump, so the
 * decoder names its buffers the same way); nothing is submitted anywhere.
 * This is the byte reference for frame71 (driver/src/v3d/v3d_frame.c at 7.1),
 * which builds the same frame with our packers; the generated 7.1 pack header
 * is Mesa's, taken from the Mesa build, not from the tree.
 *
 * The comparison is by packet name and by field label and value. Fields that
 * depend on where the buffers sit are ignored, and only these (IGNORED below):
 *   "address"  OCCLUSION_QUERY_COUNTER_ENABLE, MULTICORE_RENDERING_TILE_LIST_BASE
 *              and STORE: BO placement (the dump is [null] or a BO + offset)
 *   "start", "end"  GENERIC_TILE_LIST: the tile list's address in its BO
 * The text after the value (the enum name in a trailing comment, the
 * "[BO+offset]" notation) is not compared. Anything else, including a missing, extra or reordered
 * packet or field, is a mismatch.
 *
 * Built against a Mesa 26.2.1 tree with build-v3d, or the vendored reference
 * build (tools/v3d/mesa_ref.sh); tools/v3d/check71.d/mesa71_clear.sh does
 * either. By hand, with M=<mesa>, B=$M/build-v3d:
 *
 *   cc -std=gnu11 -Wall -Werror -D_GNU_SOURCE -DHAVE_ENDIAN_H \
 *      -I$M/src -I$M/include -I$M/src/broadcom -I$B/src/broadcom -I$B/src \
 *      tools/v3d/mesa71_clear.c \
 *      $B/src/broadcom/libbroadcom_v3d.a $B/src/broadcom/cle/libbroadcom_cle.a \
 *      $B/src/broadcom/libv3d_neon.a $B/src/util/libmesa_util.a \
 *      -lexpat -lz -lzstd -lm -lpthread -o mesa71_clear
 *   ./mesa71_clear tools/v3d/shim71/clear.clif    (MESA71_CLEAR_DUMP=1 prints the decode)
 */
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "common/v3d_device_info.h"
#include "clif/clif_dump.h"
#include "drm-uapi/v3d_drm.h"
#include "util/log.h"

#define __gen_user_data void
#define __gen_address_type uint32_t
#define __gen_address_offset(reloc) (*(reloc))
#define __gen_emit_reloc(cl, reloc)
#define __gen_unpack_address(cl, s, e) ((uint32_t)(__gen_unpack_uint(cl, s, e) << (31 - ((e) - (s)))))
#include "cle/v3d_packet_v71_pack.h"

/* the fake BOs, at the addresses of the dump */
#define BCL_GPU   0x8000u
#define TILE_GPU  0x9000u
#define RES_GPU   0x3000u
#define RCL_GPU   0xd000u
#define LIST_GPU  0xe000u

static uint8_t bcl_bo[4096], rcl_bo[4096], list_bo[4096], tile_bo[12288], res_bo[20480];

struct cl {
	uint8_t *base, *p;
};

/* cl_emit as Mesa's v3dx_cl.h has it, for 7.1 only */
#define cl_emit(cl, T, nm)                                                                         \
	for (struct V3D71_##T nm = { V3D71_##T##_header }, *_once = (void *)1; _once;              \
	     _once = NULL, V3D71_##T##_pack(NULL, (cl)->p, &nm), (cl)->p += V3D71_##T##_length)

static void pack_bcl(struct cl *cl)
{
	cl_emit(cl, NUMBER_OF_LAYERS, n) { n.number_of_layers = 1; }
	cl_emit(cl, TILE_BINNING_MODE_CFG, c) {
		c.tile_allocation_initial_block_size = 1; /* 128b */
		c.tile_allocation_block_size = 0;         /* 64b */
		c.log2_tile_width = 3;
		c.log2_tile_height = 3;
		c.width_in_pixels = 64;
		c.height_in_pixels = 64;
	}
	cl_emit(cl, FLUSH_VCD_CACHE, c) { (void)c; }
	cl_emit(cl, OCCLUSION_QUERY_COUNTER, c) { c.address = 0; }
	cl_emit(cl, START_TILE_BINNING, s) { (void)s; }
	cl_emit(cl, FLUSH, f) { (void)f; }
}

static void pack_store(struct cl *cl, unsigned buffer, unsigned format, unsigned out_fmt,
		       unsigned height_ub, uint32_t addr)
{
	cl_emit(cl, STORE_TILE_BUFFER_GENERAL, s) {
		s.buffer_to_store = buffer;
		s.memory_format = format;
		s.flip_y = false;
		s.dither_mode = 0;
		s.decimate_mode = 0;
		s.output_image_format = out_fmt;
		s.clear_buffer_being_stored = false;
		s.channel_reverse = false;
		s.r_b_swap = false;
		s.height_in_ub_or_stride = height_ub;
		s.height = 0;
		s.address = addr;
	}
}

static void pack_rcl(struct cl *cl, uint32_t list_start, uint32_t list_end)
{
	cl_emit(cl, TILE_RENDERING_MODE_CFG_COMMON, c) {
		c.number_of_render_targets = 1;
		c.image_width_pixels = 64;
		c.image_height_pixels = 64;
		c.multisample_mode_4x = false;
		c.double_buffer_in_non_ms_mode = false;
		c.depth_buffer_disable = false;
		c.early_z_test_and_update_direction = 0;
		c.early_z_disable = true;
		c.internal_depth_type = 0; /* depth_32f */
		c.early_depth_stencil_clear = false;
		c.log2_tile_width = 3;
		c.log2_tile_height = 3;
	}
	cl_emit(cl, TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1, r) {
		r.render_target_number = 0;
		r.base_address = 0;
		r.stride = 32;
		r.internal_bpp = 0;              /* 32 */
		r.internal_type_and_clamping = 8; /* 8 */
		r.clear_color_low_bits = 0xffff0000u;
	}
	cl_emit(cl, TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES, z) {
		z.stencil_clear_value = 0;
		z.z_clear_value = 0.0f;
	}
	cl_emit(cl, TILE_LIST_INITIAL_BLOCK_SIZE, b) {
		b.size_of_first_block_in_chained_tile_lists = 1;
		b.use_auto_chained_tile_lists = true;
	}
	cl_emit(cl, MULTICORE_RENDERING_TILE_LIST_SET_BASE, b) {
		b.tile_list_set_number = 0;
		b.address = TILE_GPU;
	}
	cl_emit(cl, MULTICORE_RENDERING_SUPERTILE_CFG, s) {
		s.supertile_width_in_tiles = 1;
		s.supertile_height_in_tiles = 1;
		s.total_frame_width_in_supertiles = 1;
		s.total_frame_height_in_supertiles = 1;
		s.total_frame_width_in_tiles = 1;
		s.total_frame_height_in_tiles = 1;
		s.multicore_enable = false;
		s.supertile_raster_order = false;
		s.number_of_bin_tile_lists = 1;
	}
	/* the two dummy tiles; the first clears the render target */
	for (int i = 0; i < 2; i++) {
		cl_emit(cl, TILE_COORDINATES, t) {
			t.tile_column_number = 0;
			t.tile_row_number = 0;
		}
		cl_emit(cl, END_OF_LOADS, e) { (void)e; }
		pack_store(cl, 8 /* none */, 0 /* raster */, 0, 0, 0);
		if (i == 0)
			cl_emit(cl, CLEAR_RENDER_TARGETS, c) { (void)c; }
		cl_emit(cl, END_OF_TILE_MARKER, e) { (void)e; }
	}
	cl_emit(cl, FLUSH_VCD_CACHE, c) { (void)c; }
	cl_emit(cl, START_ADDRESS_OF_GENERIC_TILE_LIST, g) {
		g.start = list_start;
		g.end = list_end;
	}
	cl_emit(cl, SUPERTILE_COORDINATES, s) {
		s.column_number_in_supertiles = 0;
		s.row_number_in_supertiles = 0;
	}
	cl_emit(cl, END_OF_RENDERING, e) { (void)e; }
}

static void pack_list(struct cl *cl)
{
	cl_emit(cl, TILE_COORDINATES_IMPLICIT, t) { (void)t; }
	cl_emit(cl, END_OF_LOADS, e) { (void)e; }
	cl_emit(cl, PRIM_LIST_FORMAT, p) {
		p.primitive_type = 2; /* list triangles */
		p.tri_strip_or_fan = false;
	}
	cl_emit(cl, SET_INSTANCEID, i) { i.instance_id = 0; }
	cl_emit(cl, BRANCH_TO_IMPLICIT_TILE_LIST, b) { b.tile_list_set_number = 0; }
	pack_store(cl, 0 /* RT0 */, 4 /* UIF no XOR */, 27 /* rgba8 */, 8, RES_GPU);
	cl_emit(cl, CLEAR_RENDER_TARGETS, c) { (void)c; }
	cl_emit(cl, END_OF_TILE_MARKER, e) { (void)e; }
	cl_emit(cl, RETURN_FROM_SUB_LIST, r) { (void)r; }
}

/* ---- the packets of a dump: name and "label: value" fields ---- */

#define MAXPKT 128
#define MAXFLD 16

struct field {
	char label[64];
	char value[64];
};

struct packet {
	char name[64];
	int nf;
	struct field f[MAXFLD];
};

struct dump {
	struct packet p[MAXPKT];
	int n;
	int errors; /* lines the decoder wrote for a failure */
};

/* Fields whose value depends on BO placement; see the header comment. */
static const char *const IGNORED[] = { "address", "start", "end" };

static bool ignored(const char *label)
{
	for (unsigned i = 0; i < sizeof(IGNORED) / sizeof(IGNORED[0]); i++)
		if (!strcmp(label, IGNORED[i]))
			return true;
	return false;
}

/* Parse the control lists of a CLIF text: the lines after "@format ctrllist"
 * up to the next "@" line; a line starting in column 0 is a packet name, the
 * indented lines are its fields. A "MESA: info: " prefix is dropped. */
static void parse(const char *text, struct dump *d)
{
	bool inlist = false;
	char *copy = strdup(text), *save = NULL;

	for (char *line = strtok_r(copy, "\n", &save); line; line = strtok_r(NULL, "\n", &save)) {
		char *s = strstr(line, "MESA: info: ");
		s = s ? s + 12 : line;
		if (strstr(s, "Failed") || strstr(s, "nknown"))
			d->errors++;
		if (s[0] == '@') {
			inlist = !strncmp(s, "@format ctrllist", 16);
			continue;
		}
		if (!inlist || !*s)
			continue;
		if (s[0] != ' ') {
			if (d->n >= MAXPKT)
				continue;
			struct packet *p = &d->p[d->n++];
			memset(p, 0, sizeof(*p));
			snprintf(p->name, sizeof(p->name), "%s", s);
			continue;
		}
		if (!d->n)
			continue;
		struct packet *p = &d->p[d->n - 1];
		/* a field is "    Label: value" in clif_dump's text and
		 * "  <comment>Label:</comment> value" in the driver's dump */
		char *l = s, *v;
		size_t ll;
		while (*l == ' ')
			l++;
		if (!strncmp(l, "/*", 2)) {
			char *c1 = strstr(l, "*/");
			if (!c1)
				continue;
			l += 2;
			while (*l == ' ')
				l++;
			ll = (size_t)(c1 - l);
			v = c1 + 2;
		} else {
			char *colon = strstr(l, ": ");
			if (!colon)
				continue;
			ll = (size_t)(colon - l);
			v = colon + 1;
		}
		if (p->nf >= MAXFLD)
			continue;
		struct field *f = &p->f[p->nf++];
		while (ll && (l[ll - 1] == ' ' || l[ll - 1] == ':'))
			ll--;
		snprintf(f->label, sizeof(f->label), "%.*s", (int)ll, l);
		for (char *q = f->label; *q; q++)
			if (*q >= 'A' && *q <= 'Z')
				*q += 'a' - 'A';
		while (*v == ' ')
			v++;
		snprintf(f->value, sizeof(f->value), "%.*s", (int)strcspn(v, " "), v);
	}
	free(copy);
}

static char *slurp(const char *path)
{
	FILE *in = fopen(path, "r");
	if (!in)
		return NULL;
	fseek(in, 0, SEEK_END);
	long n = ftell(in);
	fseek(in, 0, SEEK_SET);
	char *t = malloc((size_t)n + 1);
	size_t got = fread(t, 1, (size_t)n, in);
	t[got] = 0;
	fclose(in);
	return t;
}

int main(int argc, char **argv)
{
	if (argc != 2) {
		fprintf(stderr, "usage: mesa71_clear <shim71/clear.clif>\n");
		return 2;
	}
	char *ref_text = slurp(argv[1]);
	if (!ref_text) {
		printf("mesa71_clear: FAIL cannot read %s\n", argv[1]);
		return 1;
	}

	struct cl bcl = { bcl_bo, bcl_bo }, rcl = { rcl_bo, rcl_bo }, list = { list_bo, list_bo };
	pack_list(&list);
	pack_bcl(&bcl);
	pack_rcl(&rcl, LIST_GPU, LIST_GPU + (uint32_t)(list.p - list.base));

	const char *dir = getenv("TMPDIR") ? getenv("TMPDIR") : "/tmp";
	char path[512];
	snprintf(path, sizeof(path), "%s/mesa71_clear_%d.log", dir, (int)getpid());
	setenv("MESA_LOG_FILE", path, 1);
	setenv("MESA_LOG_LEVEL", "info", 1);
	setenv("MESA_LOG_PREFIX", "", 1);

	struct v3d_device_info devinfo = { .ver = 71, .has_accumulators = false,
					   .cle_readahead = 256 };
	struct log_stream *stream = mesa_log_streami();
	struct clif_dump *clif = clif_dump_init(&devinfo, stream, true, false);
	clif_dump_add_bo(clif, "resource_0x3000", RES_GPU, sizeof(res_bo), res_bo);
	clif_dump_add_bo(clif, "CL_0x8000", BCL_GPU, sizeof(bcl_bo), bcl_bo);
	clif_dump_add_bo(clif, "tile_alloc_0x9000", TILE_GPU, sizeof(tile_bo), tile_bo);
	clif_dump_add_bo(clif, "CL_0xd000", RCL_GPU, sizeof(rcl_bo), rcl_bo);
	clif_dump_add_bo(clif, "CL_0xe000", LIST_GPU, sizeof(list_bo), list_bo);
	struct drm_v3d_submit_cl submit = {
		.bcl_start = BCL_GPU, .bcl_end = BCL_GPU + (uint32_t)(bcl.p - bcl.base),
		.rcl_start = RCL_GPU, .rcl_end = RCL_GPU + (uint32_t)(rcl.p - rcl.base),
	};
	clif_dump(clif, &submit);
	clif_dump_destroy(clif);
	mesa_log_stream_destroy(stream);

	char *text = slurp(path);
	unlink(path);
	if (!text) {
		printf("mesa71_clear: FAIL no decoder output\n");
		return 1;
	}
	if (getenv("MESA71_CLEAR_DUMP"))
		fputs(text, stdout);

	static struct dump got, want;
	parse(text, &got);
	parse(ref_text, &want);

	int mism = 0;
	if (!got.n || !want.n) {
		printf("mesa71_clear: FAIL no packets (decoded %d, reference %d)\n", got.n, want.n);
		return 1;
	}
	if (got.n != want.n) {
		printf("mesa71_clear: packet count %d, reference has %d\n", got.n, want.n);
		mism++;
	}
	for (int i = 0; i < got.n && i < want.n; i++) {
		struct packet *g = &got.p[i], *w = &want.p[i];
		if (strcmp(g->name, w->name)) {
			printf("mesa71_clear: packet %d is %s, reference has %s\n", i, g->name, w->name);
			mism++;
			continue;
		}
		if (g->nf != w->nf) {
			printf("mesa71_clear: %s (packet %d) has %d fields, reference has %d\n", g->name, i,
			       g->nf, w->nf);
			mism++;
			continue;
		}
		for (int k = 0; k < g->nf; k++) {
			if (strcmp(g->f[k].label, w->f[k].label)) {
				printf("mesa71_clear: %s field %d is '%s', reference has '%s'\n", g->name, k,
				       g->f[k].label, w->f[k].label);
				mism++;
			} else if (!ignored(g->f[k].label) && strcmp(g->f[k].value, w->f[k].value)) {
				printf("mesa71_clear: %s '%s' is %s, reference has %s\n", g->name,
				       g->f[k].label, g->f[k].value, w->f[k].value);
				mism++;
			}
		}
	}
	printf("mesa71_clear: %d packets, decoder errors %d, %d mismatches against shim71/clear.clif\n",
	       got.n, got.errors, mism);
	return got.errors || mism;
}

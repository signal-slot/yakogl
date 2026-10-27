/* SPDX-License-Identifier: MIT
 *
 * frame_check: decodes the control lists YakoGL's V3D frame builder writes
 * (driver/src/v3d/v3d_frame.c) with Mesa's own CLE decoder (clif_dump, which
 * follows START_ADDRESS_OF_GENERIC_TILE_LIST into the per-tile sub-list), and
 * checks that the packets decoded are the ones Mesa's gallium driver emits for
 * a TLB clear of one RGBA8888 target (v3dx_draw.c start_binning, v3dx_job.c
 * bcl_epilogue, v3dx_rcl.c emit_rcl), each the expected number of times.
 *
 * This is a structural check: every byte must decode as a packet, the decoder
 * must report no error, and the packet census must match. The field values are
 * covered by cl_golden.c; the byte reference for the whole frame is Mesa's
 * V3D_DEBUG=clif dump of the same clear on the board.
 *
 * --triangle (M5) builds v3d_frame_build_triangle()'s clear-then-triangle
 * frame for the program in tri_prog.h (generated from v3d_shc's JSON by
 * tools/v3d/v3d_prog_header.py) and checks, from the decoder's text: the
 * packet census of the BCL, RCL and tile list; every field of the decoded
 * GL_SHADER_STATE_RECORD and attribute records against the value Mesa's
 * emit_shader_state_record derives from the JSON; that the three code
 * buffers the record points at hold the JSON's QPU words, as the decoder dumps
 * them; and the uniform streams the record points at, word for word, against
 * v3d_write_uniforms' rule for each quniform_contents. Nothing here runs a
 * shader.
 *
 * Built against a Mesa 26.2.1 tree with build-v3d (tools/v3d/README.md):
 *
 *   M=<mesa>; B=$M/build-v3d; cc -std=c11 -D_GNU_SOURCE -DHAVE_ENDIAN_H \
 *      -I$M/src -I$M/include -I$M/src/broadcom -Idriver/src/v3d -Idriver/src/v3d/generated -Ithird_party/mesa-26.2.1/src/broadcom -Ithird_party/mesa-26.2.1/src -Ithird_party/mesa-26.2.1/include \
 *      tools/v3d/frame_check.c driver/src/v3d/v3d_frame.c \
 *      $B/src/broadcom/libbroadcom_v3d.a $B/src/broadcom/cle/libbroadcom_cle.a \
 *      $B/src/broadcom/libv3d_neon.a $B/src/util/libmesa_util.a \
 *      -lexpat -lz -lzstd -lm -lpthread -o frame_check
 *   ./frame_check [W H]      (FRAME_CHECK_DUMP=1 prints the decoded lists)
 *
 * The triangle mode needs the generated program header on the include path:
 *
 *   python3 tools/v3d/v3d_prog_header.py \
 *      tools/v3d/tri-shaders/passthrough_interp_color.json \
 *      $OUT/tri_prog.h --name tri
 *   (the cc line above, plus -DFRAME_CHECK_TRIANGLE -I$OUT)
 *   ./frame_check --triangle [W H]
 */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "common/v3d_device_info.h"
#include "clif/clif_dump.h"
#include "drm-uapi/v3d_drm.h"
#include "util/log.h"

#include "v3d_frame.h"

#define BO_GPU   0x00100000u
#define BO_SIZE  0x00010000u
#define BCL_OFF  0x0000u
#define RCL_OFF  0x1000u
#define LIST_OFF 0x3000u

/* Lines whose first word is exactly `name`. */
static int census(const char *text, const char *name)
{
	int n = 0;
	size_t len = strlen(name);
	for (const char *line = text; *line;) {
		const char *p = line;
		while (*p == ' ')
			p++;
		if (strncmp(p, name, len) == 0 && (p[len] == ' ' || p[len] == '\n' || p[len] == 0))
			n++;
		const char *nl = strchr(line, '\n');
		if (!nl)
			break;
		line = nl + 1;
	}
	return n;
}

static int run_clear(uint32_t w, uint32_t h)
{

	static uint8_t bo[BO_SIZE];
	struct v3d_cl bcl, rcl, list;
	v3d_cl_init(&bcl, bo + BCL_OFF, BO_GPU + BCL_OFF, RCL_OFF - BCL_OFF);
	v3d_cl_init(&rcl, bo + RCL_OFF, BO_GPU + RCL_OFF, LIST_OFF - RCL_OFF);
	v3d_cl_init(&list, bo + LIST_OFF, BO_GPU + LIST_OFF, BO_SIZE - LIST_OFF);

	struct v3d_clear_frame f = {
		.width = w, .height = h, .clear_rgba8 = 0xff00ff00u,
		.target = 0x00400000u, .stride = w * 4, .tile_alloc = 0x00800000u,
	};
	if (!v3d_frame_build_clear(&f, &bcl, &rcl, &list)) {
		printf("frame_check: FAIL the builder refused %ux%u\n", w, h);
		return 1;
	}
	struct v3d_tiling t;
	v3d_frame_tiling(w, h, &t);

	/* Mesa's logger writes the decoded lists to this file, unprefixed. */
	const char *dir = getenv("TMPDIR") ? getenv("TMPDIR") : "/tmp";
	char path[512];
	snprintf(path, sizeof(path), "%s/frame_check_%d.log", dir, (int)getpid());
	setenv("MESA_LOG_FILE", path, 1);
	setenv("MESA_LOG_LEVEL", "info", 1);
	setenv("MESA_LOG_PREFIX", "", 1);

	/* clif_dump reads ver and cle_readahead */
	struct v3d_device_info devinfo = { .ver = 42, .has_accumulators = true,
					   .cle_readahead = 256 };
	struct log_stream *stream = mesa_log_streami();
	struct clif_dump *clif = clif_dump_init(&devinfo, stream, true, false);
	clif_dump_add_bo(clif, "bo", BO_GPU, BO_SIZE, bo);
	/* the target and the tile allocation are referenced, not followed; they
	 * are registered so that the decoder can name them */
	static uint8_t target[64], tile_alloc[64];
	clif_dump_add_bo(clif, "target", f.target, sizeof(target), target);
	clif_dump_add_bo(clif, "tile_alloc", f.tile_alloc, sizeof(tile_alloc), tile_alloc);
	struct drm_v3d_submit_cl submit = {
		.bcl_start = bcl.gpu, .bcl_end = bcl.gpu + bcl.used,
		.rcl_start = rcl.gpu, .rcl_end = rcl.gpu + rcl.used,
	};
	clif_dump(clif, &submit);
	clif_dump_destroy(clif);
	mesa_log_stream_destroy(stream);

	FILE *in = fopen(path, "r");
	if (!in) {
		printf("frame_check: FAIL no decoder output at %s\n", path);
		return 1;
	}
	fseek(in, 0, SEEK_END);
	long n = ftell(in);
	fseek(in, 0, SEEK_SET);
	char *text = malloc(n + 1);
	size_t got_bytes = fread(text, 1, n, in);
	text[got_bytes] = 0;
	fclose(in);
	unlink(path);

	if (getenv("FRAME_CHECK_DUMP"))
		fputs(text, stdout);

	/* What Mesa emits for this frame, and how often, under the name the
	 * decoder prints: the XML shortname upper-cased when there is one
	 * (v3d_decoder.c), else the packet name. */
	uint32_t supertiles = ((h - 1) / (t.tile_h * t.supertile_h) + 1) *
			      ((w - 1) / (t.tile_w * t.supertile_w) + 1);
	const struct { const char *name; int n; } want[] = {
		{ "TILE_BINNING_MODE_CFG", 1 },
		{ "CLEAR_VCD_CACHE", 2 },           /* Flush VCD cache: BCL once, RCL once */
		{ "OCCLUSION_QUERY_COUNTER_ENABLE", 1 },
		{ "START_TILE_BINNING", 1 },
		{ "FLUSH", 1 },
		{ "TILE_RENDERING_MODE_CFG_COMMON", 1 },
		{ "TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART1", 1 },
		{ "TILE_RENDERING_MODE_CFG_COLOR", 1 },
		{ "TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES", 1 },
		{ "TILE_LIST_INITIAL_BLOCK_SIZE", 1 },
		{ "MULTICORE_RENDERING_TILE_LIST_BASE", 1 },
		{ "MULTICORE_RENDERING_SUPERTILE_CFG", 1 },
		{ "TILE_COORDS", 2 },               /* the two dummy tiles */
		{ "END_LOADS", 3 },
		{ "STORE", 3 },                     /* two dummy tiles + the list */
		{ "CLEAR", 2 },
		{ "END_TILE", 3 },
		{ "GENERIC_TILE_LIST", 1 },
		{ "IMPLICIT_TILE_COORDS", 1 },
		{ "PRIM_LIST_FORMAT", 1 },
		{ "SET_INSTANCEID", 1 },
		{ "BRANCH_IMPLICIT_TILE", 1 },
		{ "RETURN", 1 },
		{ "SUPERTILE_COORDS", (int)supertiles },
		{ "END_RENDER", 1 },
	};
	int failures = 0, total = 0;
	for (unsigned i = 0; i < sizeof(want) / sizeof(want[0]); i++) {
		int got = census(text, want[i].name);
		total += got;
		if (got != want[i].n) {
			printf("frame_check: FAIL %s: decoded %d, expected %d\n", want[i].name, got,
			       want[i].n);
			failures++;
		}
	}
	if (strstr(text, "Failed") || strstr(text, "nknown")) {
		printf("frame_check: FAIL the decoder reported an error\n");
		failures++;
	}
	/* the population: the census must have found packets at all */
	if (total == 0) {
		printf("frame_check: FAIL nothing was decoded\n");
		failures++;
	}
	printf("frame_check: %ux%u, %ux%u tiles, %u supertile(s), BCL %u RCL %u list %u bytes, "
	       "%d packets decoded: %s\n",
	       w, h, t.tiles_x, t.tiles_y, supertiles, bcl.used, rcl.used, list.used, total,
	       failures ? "FAIL" : "ok");
	free(text);
	return failures ? 1 : 0;
}

/* ======================================================================== */
/* --triangle: M5 */

#ifdef FRAME_CHECK_TRIANGLE
#include "tri_prog.h"

struct named_bo {
	const char *name;
	uint32_t gpu, size;
	void *cpu;
};

/* Decode the job with Mesa's clif_dump and return its text (malloc'd). */
static char *decode(const struct named_bo *bos, unsigned n, const struct drm_v3d_submit_cl *submit)
{
	const char *dir = getenv("TMPDIR") ? getenv("TMPDIR") : "/tmp";
	char path[512];
	snprintf(path, sizeof(path), "%s/frame_check_%d.log", dir, (int)getpid());
	setenv("MESA_LOG_FILE", path, 1);
	setenv("MESA_LOG_LEVEL", "info", 1);
	setenv("MESA_LOG_PREFIX", "", 1);

	struct v3d_device_info devinfo = { .ver = 42, .has_accumulators = true,
					   .cle_readahead = 256 };
	struct log_stream *stream = mesa_log_streami();
	struct clif_dump *clif = clif_dump_init(&devinfo, stream, true, false);
	for (unsigned i = 0; i < n; i++)
		clif_dump_add_bo(clif, bos[i].name, bos[i].gpu, bos[i].size, bos[i].cpu);
	struct drm_v3d_submit_cl s = *submit;
	clif_dump(clif, &s);
	clif_dump_destroy(clif);
	mesa_log_stream_destroy(stream);

	FILE *in = fopen(path, "r");
	if (!in)
		return NULL;
	fseek(in, 0, SEEK_END);
	long len = ftell(in);
	fseek(in, 0, SEEK_SET);
	char *text = malloc(len + 1);
	size_t got = fread(text, 1, len, in);
	text[got] = 0;
	fclose(in);
	unlink(path);
	return text;
}

/* The text from `from` up to the next line that starts with '@' (the next
 * @format or @buffer), as a NUL-terminated copy; NULL if `from` is absent.
 * `nth` picks the nth occurrence of `from`, counting from 0. */
static char *section(const char *text, const char *from, int nth)
{
	const char *p = text;
	for (int i = 0; i <= nth; i++) {
		p = strstr(p, from);
		if (!p)
			return NULL;
		if (i < nth)
			p += strlen(from);
	}
	const char *body = strchr(p, '\n');
	if (!body)
		return NULL;
	body++;
	const char *end = body;
	while (*end && *end != '@') {
		const char *nl = strchr(end, '\n');
		end = nl ? nl + 1 : end + strlen(end);
	}
	char *out = malloc(end - body + 1);
	memcpy(out, body, end - body);
	out[end - body] = 0;
	return out;
}

/* The value the decoder printed for field `name` in a pretty-printed group
 * ("    Name: value" lines), or NULL when the field is not there. */
static const char *field(const char *group, const char *name, char *buf, size_t n)
{
	size_t len = strlen(name);
	for (const char *line = group; *line;) {
		const char *p = line;
		while (*p == ' ')
			p++;
		if (strncmp(p, name, len) == 0 && p[len] == ':' && p[len + 1] == ' ') {
			const char *v = p + len + 2;
			const char *nl = strchr(v, '\n');
			size_t vl = nl ? (size_t)(nl - v) : strlen(v);
			if (vl >= n)
				vl = n - 1;
			memcpy(buf, v, vl);
			buf[vl] = 0;
			return buf;
		}
		const char *nl = strchr(line, '\n');
		if (!nl)
			break;
		line = nl + 1;
	}
	return NULL;
}

/* The number of "    Name: value" lines of a group: the population the
 * field checks below must account for. */
static int field_lines(const char *group)
{
	int n = 0;
	for (const char *line = group; *line;) {
		if (strncmp(line, "    ", 4) == 0 && strstr(line, ": "))
			n++;
		const char *nl = strchr(line, '\n');
		if (!nl)
			break;
		line = nl + 1;
	}
	return n;
}

static int tri_failures;

/* --- the few JSON reads the check needs, from v3d_shc's output itself, so
 * that the expected values do not come through tri_prog.h (the builder's
 * input). v3d_shc prints one flat object per stage. --- */
static char *json_text;

/* The span of the object that follows `"stage": {`, or NULL. */
static const char *json_stage(const char *stage, const char **end)
{
	char key[32];
	snprintf(key, sizeof(key), "\"%s\": {", stage);
	const char *p = strstr(json_text, key);
	if (!p)
		return NULL;
	p += strlen(key);
	int depth = 1;
	const char *q = p;
	for (; *q && depth; q++) {
		if (*q == '{' || *q == '[')
			depth++;
		else if (*q == '}' || *q == ']')
			depth--;
	}
	*end = q;
	return p;
}

/* The raw token of `"key": ` inside a stage (true, 4, "0x..", or the '['
 * of an array), or NULL when the stage has no such key: an absent value. */
static const char *json_get(const char *stage, const char *key)
{
	const char *end, *p = json_stage(stage, &end);
	if (!p)
		return NULL;
	char k[96];
	snprintf(k, sizeof(k), "\"%s\": ", key);
	const char *f = strstr(p, k);
	if (!f || f >= end)
		return NULL;
	return f + strlen(k);
}

static int json_absent; /* keys the check had to take from the header */

static bool jb(const char *stage, const char *key, bool header_value)
{
	const char *v = json_get(stage, key);
	if (!v) {
		json_absent++;
		return header_value;
	}
	return strncmp(v, "true", 4) == 0;
}

static uint32_t ju(const char *stage, const char *key)
{
	const char *v = json_get(stage, key);
	if (!v) {
		printf("frame_check: FAIL %s.%s is absent from the JSON\n", stage, key);
		tri_failures++;
		return 0;
	}
	if (*v == '"')
		v++;
	return (uint32_t)strtoul(v, NULL, 0);
}

/* The n-th element of the array `key` of a stage (numbers or "0x.." strings);
 * false past its end. */
static bool json_elem(const char *stage, const char *key, uint32_t n, uint64_t *out)
{
	const char *v = json_get(stage, key);
	if (!v || *v != '[')
		return false;
	v++;
	for (uint32_t i = 0;; i++) {
		while (*v == ' ' || *v == '"')
			v++;
		if (*v == ']' || !*v)
			return false;
		char *e;
		uint64_t x = strtoull(v, &e, 0);
		if (e == v)
			return false; /* not a number: malformed */
		if (i == n) {
			*out = x;
			return true;
		}
		while (*e == '"' || *e == ' ')
			e++;
		if (*e != ',')
			return false; /* ']' : the array ended */
		v = e + 1;
	}
}

/* The n-th {contents, data} of a stage's uniform list (its objects hold no
 * ']', so the first one ends the list). */
static bool json_uniform(const char *stage, uint32_t n, uint32_t *contents, uint32_t *data)
{
	const char *v = json_get(stage, "uniforms");
	if (!v || *v != '[')
		return false;
	const char *list_end = strchr(v, ']');
	for (uint32_t i = 0;; i++) {
		v = strstr(v, "\"contents\": ");
		if (!v || v > list_end)
			return false;
		if (i == n) {
			*contents = strtoul(v + 12, NULL, 0);
			const char *d = strstr(v, "\"data\": \"");
			if (!d || d > list_end)
				return false;
			*data = strtoul(d + 9, NULL, 0);
			return true;
		}
		v += 12;
	}
}


static void want_field(const char *what, const char *group, const char *name, const char *want)
{
	char buf[256];
	const char *got = field(group, name, buf, sizeof(buf));
	if (!got || strcmp(got, want) != 0) {
		printf("frame_check: FAIL %s \"%s\": decoded %s%s%s, expected \"%s\"\n", what, name,
		       got ? "\"" : "", got ? got : "(absent)", got ? "\"" : "", want);
		tri_failures++;
	}
}

static const char *b(bool v)
{
	return v ? "1 /* true */" : "0 /* false */";
}

static const char *u(uint32_t v)
{
	static char ring[16][32];
	static int k;
	char *s = ring[k++ % 16];
	snprintf(s, 32, "%u", v);
	return s;
}

static const char *addr(const char *bo, uint32_t base, uint32_t a)
{
	static char ring[16][96];
	static int k;
	char *s = ring[k++ % 16];
	snprintf(s, 96, "[%s+0x%08x] /* 0x%08x */", bo, a - base, a);
	return s;
}

/* The QPU words of a code buffer as clif_dump prints a buffer with no
 * relocations ("@buffer NAME", then "@format binary" lines of 0x%08x words,
 * or "@format blank" for a zero tail). Returns how many 32-bit words were
 * compared, or -1 on the first difference. */
static int check_code(const char *text, const char *name, const char *stage)
{
	uint32_t n = 0;
	uint64_t word64;
	while (json_elem(stage, "code", n, &word64))
		n++;
	if (n == 0) {
		printf("frame_check: FAIL %s.code is absent or empty in the JSON\n", stage);
		return -1;
	}
	char key[64];
	snprintf(key, sizeof(key), "@buffer %s\n", name);
	const char *p = strstr(text, key);
	if (!p) {
		printf("frame_check: FAIL the decoder did not dump buffer %s\n", name);
		return -1;
	}
	p += strlen(key);
	uint32_t i = 0;
	while (i < 2 * n) {
		while (*p == ' ' || *p == '\n')
			p++;
		if (strncmp(p, "@format binary", 14) == 0 || strncmp(p, "@format blank", 13) == 0) {
			if (p[8] == 'l') { /* blank: the rest of the buffer is zero */
				break;
			}
			p = strchr(p, '\n');
			continue;
		}
		if (strncmp(p, "0x", 2) != 0)
			break;
		uint32_t word = strtoul(p, (char **)&p, 16);
		json_elem(stage, "code", i / 2, &word64);
		uint32_t want = (uint32_t)(word64 >> (32 * (i % 2)));
		if (word != want) {
			printf("frame_check: FAIL %s word %u: decoded 0x%08x, JSON 0x%08x\n", name, i,
			       word, want);
			return -1;
		}
		i++;
	}
	if (i != 2 * n) {
		printf("frame_check: FAIL %s: %u of %u words decoded\n", name, i, 2 * n);
		return -1;
	}
	return (int)i;
}

static uint32_t f32(float f)
{
	uint32_t v;
	memcpy(&v, &f, 4);
	return v;
}

/* v3d_write_uniforms' rule, re-stated here from v3d_uniforms.c:244-266 so the
 * check does not share the builder's code. */
static int check_uniforms(const char *stage, const uint8_t *stream, const struct v3d_viewport *vp,
			  const struct v3d_uniform_storage *st, uint32_t height)
{
	uint32_t contents, data, i;
	for (i = 0; json_uniform(stage, i, &contents, &data); i++) {
		uint32_t want;
		switch (contents) {
		case 0: want = data; break;
		case 1: /* gallium_uniforms[data], v3d_uniforms.c:251-252 */
			if (!st->words || data >= st->count) {
				printf("frame_check: FAIL %s uniform %u: word %u is outside the "
				       "storage (%u words)\n", stage, i, data, st->count);
				return -1;
			}
			want = st->words[data];
			break;
		case 2: want = f32(vp->scale[0] * 256.0f); break;
		case 3: want = f32(vp->scale[1] * 256.0f); break;
		case 4: want = f32(vp->translate[2]); break;
		case 5: want = f32(vp->scale[2]); break;
		/* gl_FragCoord.y's window transform, stated here for the frame
		 * this check builds: a window target (y0 at the top), so
		 * y' = FYCD * -1 + height (v3d-backend.md, "gl_FragCoord"); the
		 * data of both codes is 0. */
		case 0x10000:
		case 0x10001:
			if (data != 0) {
				printf("frame_check: FAIL %s uniform %u: contents 0x%x with data %u\n",
				       stage, i, contents, data);
				return -1;
			}
			want = contents == 0x10000 ? f32(-1.0f) : f32((float)height);
			break;
		default:
			printf("frame_check: FAIL %s uniform %u: contents %u has no rule here\n",
			       stage, i, contents);
			return -1;
		}
		uint32_t got;
		memcpy(&got, stream + 4 * i, 4);
		if (got != want) {
			printf("frame_check: FAIL %s uniform %u (contents %u): 0x%08x, expected "
			       "0x%08x\n", stage, i, contents, got, want);
			return -1;
		}
	}
	return (int)i;
}


static int run_triangle(uint32_t w, uint32_t h, const char *json_path)
{
	FILE *jf = fopen(json_path, "r");
	if (!jf) {
		printf("frame_check: FAIL cannot read %s\n", json_path);
		return 1;
	}
	fseek(jf, 0, SEEK_END);
	long jl = ftell(jf);
	fseek(jf, 0, SEEK_SET);
	json_text = malloc(jl + 1);
	json_text[fread(json_text, 1, jl, jf)] = 0;
	fclose(jf);

	/* the GPU address space of the job: one buffer per role, as Mesa has */
	enum { B_CL, B_FS, B_VS, B_CS, B_VTX, B_TARGET, B_TA, N_BO };
	static uint8_t cl_mem[0x10000], fs_mem[sizeof(tri_fs_code)], vs_mem[sizeof(tri_vs_code)],
		cs_mem[sizeof(tri_cs_code)], vtx_mem[256], target[64], ta[64];
	struct named_bo bos[N_BO] = {
		{ "bo", 0x00100000u, sizeof(cl_mem), cl_mem },
		{ "fs_code", 0x00200000u, sizeof(fs_mem), fs_mem },
		{ "vs_code", 0x00210000u, sizeof(vs_mem), vs_mem },
		{ "cs_code", 0x00220000u, sizeof(cs_mem), cs_mem },
		{ "vertices", 0x00230000u, sizeof(vtx_mem), vtx_mem },
		{ "target", 0x00400000u, sizeof(target), target },
		{ "tile_alloc", 0x00800000u, sizeof(ta), ta },
	};
	memcpy(fs_mem, tri_fs_code, sizeof(tri_fs_code));
	memcpy(vs_mem, tri_vs_code, sizeof(tri_vs_code));
	memcpy(cs_mem, tri_cs_code, sizeof(tri_cs_code));

	struct v3d_fs_prog fs;
	struct v3d_vs_prog vs, cs;
	tri_fs_prog(&fs, bos[B_FS].gpu);
	tri_vs_prog(&vs, bos[B_VS].gpu);
	tri_cs_prog(&cs, bos[B_CS].gpu);

	/* interleaved vec4 position + vec4 colour, 32 bytes a vertex */
	struct v3d_vertex_attr attrs[TRI_NUM_ATTRS];
	attrs[TRI_ATTR_A_POS_RECORD] = (struct v3d_vertex_attr){ bos[B_VTX].gpu, 32, 4 };
	attrs[TRI_ATTR_A_COLOR_RECORD] = (struct v3d_vertex_attr){ bos[B_VTX].gpu + 16, 32, 4 };

	struct v3d_triangle_frame f = {
		.clear = { .width = w, .height = h, .clear_rgba8 = 0xff000000u,
			   .target = bos[B_TARGET].gpu, .stride = w * 4,
			   .tile_alloc = bos[B_TA].gpu },
		.front_ccw = true,
		.fs = &fs, .vs = &vs, .cs = &cs,
		.attrs = attrs, .num_attrs = TRI_NUM_ATTRS,
		.first_vertex = 0, .vertex_count = 3,
	};
	/* GL uniform storage with a distinct value in every word, so a word taken
	 * from the wrong index or stage is seen */
	static uint32_t st_vs[64], st_fs[64];
	for (uint32_t i = 0; i < 64; i++) {
		st_vs[i] = 0x51000000u | i;
		st_fs[i] = 0x5f000000u | i;
	}
	f.vs_storage = (struct v3d_uniform_storage){ st_vs, 64 };
	f.fs_storage = (struct v3d_uniform_storage){ st_fs, 64 };
	v3d_viewport_gl(0, 0, (float)w, (float)h, 0.0f, 1.0f, true, h, &f.vp);
	v3d_wpos_gl(true, h, &f.wpos);

	struct v3d_cl bcl, rcl, ind;
	v3d_cl_init(&bcl, cl_mem, bos[B_CL].gpu, 0x1000);
	v3d_cl_init(&rcl, cl_mem + 0x1000, bos[B_CL].gpu + 0x1000, 0x2000);
	v3d_cl_init(&ind, cl_mem + 0x3000, bos[B_CL].gpu + 0x3000, sizeof(cl_mem) - 0x3000);
	struct v3d_triangle_layout lo;
	if (!v3d_frame_build_triangle(&f, &bcl, &rcl, &ind, &lo)) {
		printf("frame_check: FAIL the builder refused the triangle frame %ux%u\n", w, h);
		return 1;
	}
	struct drm_v3d_submit_cl submit = {
		.bcl_start = bcl.gpu, .bcl_end = bcl.gpu + bcl.used,
		.rcl_start = rcl.gpu, .rcl_end = rcl.gpu + rcl.used,
	};
	char *text = decode(bos, N_BO, &submit);
	if (!text) {
		printf("frame_check: FAIL no decoder output\n");
		return 1;
	}
	if (getenv("FRAME_CHECK_DUMP"))
		fputs(text, stdout);

	struct v3d_tiling t;
	v3d_frame_tiling(w, h, &t);
	uint32_t supertiles = ((h - 1) / (t.tile_h * t.supertile_h) + 1) *
			      ((w - 1) / (t.tile_w * t.supertile_w) + 1);

	/* 1. the census: what Mesa emits for a clear and one triangle, in the
	 * decoder's names (the XML shortname upper-cased, else the name) */
	const struct { const char *name; int n; } want[] = {
		/* start_binning */
		{ "TILE_BINNING_MODE_CFG", 1 },
		{ "CLEAR_VCD_CACHE", 2 },           /* BCL once, RCL once */
		{ "OCCLUSION_QUERY_COUNTER_ENABLE", 2 }, /* start_binning + emit_state */
		{ "START_TILE_BINNING", 1 },
		/* emit_state */
		{ "CLIP", 1 },
		{ "CFG_BITS", 1 },
		{ "POINT_SIZE", 1 },
		{ "LINE_WIDTH", 1 },
		{ "CLIPPER_XY", 1 },
		{ "CLIPPER_Z", 1 },
		{ "CLIPZ", 1 },
		{ "VIEWPORT_OFFSET", 1 },
		{ "BLEND_ENABLES", 1 },
		{ "COLOR_WMASKS", 1 },
		{ "BLEND_CCOLOR", 1 },
		{ "ZERO_ALL_FLATSHADE_FLAGS", 1 },
		{ "ZERO_ALL_NOPERSPECTIVE_FLAGS", 1 },
		{ "ZERO_ALL_CENTROID_FLAGS", 1 },
		{ "TRANSFORM_FEEDBACK_SPECS", 1 },
		{ "SAMPLE_STATE", 1 },
		/* v3d_emit_gl_shader_state, the draw, bcl_epilogue */
		{ "VCM_CACHE_SIZE", 1 },
		{ "GL_SHADER", 1 },
		{ "VERTEX_ARRAY_PRIMS", 1 },
		{ "FLUSH", 1 },
		/* the RCL and the tile list, as for the clear */
		{ "TILE_RENDERING_MODE_CFG_COMMON", 1 },
		{ "TILE_RENDERING_MODE_CFG_CLEAR_COLORS_PART1", 1 },
		{ "TILE_RENDERING_MODE_CFG_COLOR", 1 },
		{ "TILE_RENDERING_MODE_CFG_ZS_CLEAR_VALUES", 1 },
		{ "TILE_LIST_INITIAL_BLOCK_SIZE", 1 },
		{ "MULTICORE_RENDERING_TILE_LIST_BASE", 1 },
		{ "MULTICORE_RENDERING_SUPERTILE_CFG", 1 },
		{ "TILE_COORDS", 2 },
		{ "END_LOADS", 3 },
		{ "STORE", 3 },
		{ "CLEAR", 2 },
		{ "END_TILE", 3 },
		{ "GENERIC_TILE_LIST", 1 },
		{ "IMPLICIT_TILE_COORDS", 1 },
		{ "PRIM_LIST_FORMAT", 1 },
		{ "SET_INSTANCEID", 1 },
		{ "BRANCH_IMPLICIT_TILE", 1 },
		{ "RETURN", 1 },
		{ "SUPERTILE_COORDS", (int)supertiles },
		{ "END_RENDER", 1 },
	};
	int census_total = 0, census_want = 0;
	for (unsigned i = 0; i < sizeof(want) / sizeof(want[0]); i++) {
		int got = census(text, want[i].name);
		census_total += got;
		census_want += want[i].n;
		if (got != want[i].n) {
			printf("frame_check: FAIL %s: decoded %d, expected %d\n", want[i].name, got,
			       want[i].n);
			tri_failures++;
		}
	}
	/* every packet line the decoder printed is one of the above: count the
	 * packet-name lines (upper-case identifiers alone on a line) */
	int packet_lines = 0;
	for (const char *line = text; *line;) {
		const char *nl = strchr(line, '\n');
		size_t len = nl ? (size_t)(nl - line) : strlen(line);
		size_t k = 0;
		while (k < len && ((line[k] >= 'A' && line[k] <= 'Z') || line[k] == '_' ||
				   (line[k] >= '0' && line[k] <= '9')))
			k++;
		if (len > 0 && k == len && line[0] >= 'A' && line[0] <= 'Z')
			packet_lines++;
		if (!nl)
			break;
		line = nl + 1;
	}
	if (packet_lines != census_want || census_total != census_want) {
		printf("frame_check: FAIL %d packet lines decoded, %d counted by name, %d expected\n",
		       packet_lines, census_total, census_want);
		tri_failures++;
	}
	if (strstr(text, "Failed") || strstr(text, "nknown")) {
		printf("frame_check: FAIL the decoder reported an error\n");
		tri_failures++;
	}

	/* 2. the shader state record, field by field, against the JSON values
	 * through Mesa's emit_shader_state_record rules (v3dx_draw.c:553-682)
	 * and v3d_compute_vpm_config without a GS (vir.c:3071-3079) */
	int fields_checked = 0;
	char *rec = section(text, "@format shadrec_gl_main", 0);
	if (!rec) {
		printf("frame_check: FAIL no shader state record was decoded\n");
		tri_failures++;
	} else {
		const uint32_t base = bos[B_CL].gpu;
		const struct { const char *name; const char *want; } rf[] = {
			{ "Point size in shaded vertex data", b(false) },
			{ "Enable clipping", b(true) },
			{ "Vertex ID read by coordinate shader", b(jb("cs", "uses_vid", cs.uses_vid)) },
			{ "Instance ID read by coordinate shader", b(jb("cs", "uses_iid", cs.uses_iid)) },
			{ "Base Instance ID read by coordinate shader", b(false) },
			{ "Vertex ID read by vertex shader", b(jb("vs", "uses_vid", vs.uses_vid)) },
			{ "Instance ID read by vertex shader", b(jb("vs", "uses_iid", vs.uses_iid)) },
			{ "Base Instance ID read by vertex shader", b(false) },
			{ "Fragment shader does Z writes", b(jb("fs", "writes_z", fs.writes_z)) },
			{ "Turn off early-z test", b(jb("fs", "disable_ez", fs.disable_ez)) },
			{ "Coordinate shader has separate input and output VPM blocks",
			  b(jb("cs", "separate_segments", cs.separate_segments)) },
			{ "Vertex shader has separate input and output VPM blocks",
			  b(jb("vs", "separate_segments", vs.separate_segments)) },
			{ "Fragment shader uses real pixel centre W in addition to centroid W2",
			  b(jb("fs", "uses_center_w", fs.uses_center_w)) },
			{ "Enable Sample Rate Shading", b(false) },
			{ "Any shader reads hardware-written Primitive ID", b(jb("fs", "uses_pid", fs.uses_pid)) },
			{ "Insert Primitive ID as first varying to fragment shader",
			  b(jb("fs", "uses_pid", fs.uses_pid)) },
			{ "Turn off scoreboard", b(false) },
			{ "Do scoreboard wait on first thread switch",
			  b(jb("fs", "lock_scoreboard_on_first_thrsw", fs.lock_scoreboard_on_first_thrsw)) },
			{ "Disable implicit point/line varyings",
			  b(!jb("fs", "uses_implicit_point_line_varyings", fs.uses_implicit_point_line_varyings)) },
			{ "No prim pack", b(false) },
			{ "Number of varyings in Fragment Shader", u(ju("fs", "num_inputs")) },
			{ "Coordinate Shader output VPM segment size", u(ju("cs", "vpm_output_size")) },
			{ "Min Coord Shader output segments required in play in addition to VCM "
			  "cache size", u(0) },
			{ "Coordinate Shader input VPM segment size",
			  u(jb("cs", "separate_segments", cs.separate_segments) ? ju("cs", "vpm_input_size") : 1) },
			/* minus_one: the decoder prints the value plus one */
			{ "Min Coord Shader input segments required in play", u(1) },
			{ "Vertex Shader output VPM segment size", u(ju("vs", "vpm_output_size")) },
			{ "Min Vertex Shader output segments required in play in addition to VCM "
			  "cache size", u(0) },
			{ "Vertex Shader input VPM segment size",
			  u(jb("vs", "separate_segments", vs.separate_segments) ? ju("vs", "vpm_input_size") : 1) },
			{ "Min Vertex Shader input segments required in play", u(1) },
			{ "Address of default attribute values", addr("bo", base, lo.defaults) },
			{ "Fragment Shader Code Address", addr("fs_code", bos[B_FS].gpu, fs.code) },
			{ "Fragment Shader 4-way threadable", b(ju("fs", "threads") == 4) },
			{ "Fragment Shader start in final thread section", b(jb("fs", "single_seg", fs.single_seg)) },
			{ "Fragment Shader Propagate NaNs", b(true) },
			{ "Fragment Shader Uniforms Address", addr("bo", base, lo.fs_uniforms) },
			{ "Vertex Shader Code Address", addr("vs_code", bos[B_VS].gpu, vs.code) },
			{ "Vertex Shader 4-way threadable", b(ju("vs", "threads") == 4) },
			{ "Vertex Shader start in final thread section", b(jb("vs", "single_seg", vs.single_seg)) },
			{ "Vertex Shader Propagate NaNs", b(true) },
			{ "Vertex Shader Uniforms Address", addr("bo", base, lo.vs_uniforms) },
			{ "Coordinate Shader Code Address", addr("cs_code", bos[B_CS].gpu, cs.code) },
			{ "Coordinate Shader 4-way threadable", b(ju("cs", "threads") == 4) },
			{ "Coordinate Shader start in final thread section", b(jb("cs", "single_seg", cs.single_seg)) },
			{ "Coordinate Shader Propagate NaNs", b(true) },
			{ "Coordinate Shader Uniforms Address", addr("bo", base, lo.cs_uniforms) },
		};
		const int nrf = sizeof(rf) / sizeof(rf[0]);
		for (int i = 0; i < nrf; i++)
			want_field("shader record", rec, rf[i].name, rf[i].want);
		fields_checked += nrf;
		/* the population: every field the decoder printed was checked */
		if (field_lines(rec) != nrf) {
			printf("frame_check: FAIL the record has %d decoded fields, %d checked\n",
			       field_lines(rec), nrf);
			tri_failures++;
		}
		free(rec);
	}

	/* 3. the attribute records (v3dx_draw.c:798-847, v3dx_state.c:392-442) */
	int attrs_checked = 0;
	for (int i = 0; i < TRI_NUM_ATTRS + 1; i++) {
		char *ar = section(text, "@format shadrec_gl_attr", i);
		if (i == TRI_NUM_ATTRS) {
			if (ar) {
				printf("frame_check: FAIL more attribute records than %d\n",
				       TRI_NUM_ATTRS);
				tri_failures++;
				free(ar);
			}
			break;
		}
		if (!ar) {
			printf("frame_check: FAIL attribute record %d was not decoded\n", i);
			tri_failures++;
			continue;
		}
		/* GFXH-930: if no attribute is read by the CS, the last one is */
		bool any_cs = false;
		uint64_t sz, vs_values = 0, cs_values = 0;
		for (int k = 0; k < TRI_NUM_ATTRS; k++)
			any_cs |= json_elem("cs", "vattr_sizes", k, &sz) && sz != 0;
		if (!json_elem("cs", "vattr_sizes", i, &cs_values) ||
		    !json_elem("vs", "vattr_sizes", i, &vs_values)) {
			printf("frame_check: FAIL vattr_sizes[%d] absent from the JSON\n", i);
			tri_failures++;
		}
		if (!any_cs && i == TRI_NUM_ATTRS - 1)
			cs_values = 1;
		const struct { const char *name; const char *want; } af[] = {
			{ "Address", addr("vertices", bos[B_VTX].gpu, attrs[i].address) },
			/* the field holds components & 3; the decoder prints a
			 * raw 0 as 4 (v3d_decoder.c:875-876) */
			{ "Vec size", u(attrs[i].components) },
			{ "Type", "2 /* Attribute float */" },
			{ "Signed int type", b(false) },
			{ "Normalized int type", b(false) },
			{ "Read as int/uint", b(false) },
			{ "Number of values read by Coordinate shader", u((uint32_t)cs_values) },
			{ "Number of values read by Vertex shader", u((uint32_t)vs_values) },
			{ "Instance Divisor", u(0) },
			{ "Stride", u(attrs[i].stride) },
			{ "Maximum Index", u(0xffffff) },
		};
		const int naf = sizeof(af) / sizeof(af[0]);
		char what[32];
		snprintf(what, sizeof(what), "attribute record %d", i);
		for (int k = 0; k < naf; k++)
			want_field(what, ar, af[k].name, af[k].want);
		fields_checked += naf;
		if (field_lines(ar) != naf) {
			printf("frame_check: FAIL %s has %d decoded fields, %d checked\n", what,
			       field_lines(ar), naf);
			tri_failures++;
		}
		attrs_checked++;
		free(ar);
	}

	/* 4. the code the record points at, as the decoder dumps it */
	int words = 0, r;
	const struct { const char *name, *stage; uint32_t n; } code[] = {
		{ "fs_code", "fs", sizeof(tri_fs_code) / 8 },
		{ "vs_code", "vs", sizeof(tri_vs_code) / 8 },
		{ "cs_code", "cs", sizeof(tri_cs_code) / 8 },
	};
	for (int i = 0; i < 3; i++) {
		r = check_code(text, code[i].name, code[i].stage);
		if (r < 0)
			tri_failures++;
		else
			words += r;
	}

	/* 5. the uniform streams at the addresses the record names */
	int uniforms = 0;
	const struct { const char *stage; uint32_t at; const struct v3d_uniform_storage *st; } us[] = {
		{ "fs", lo.fs_uniforms, &f.fs_storage },
		{ "vs", lo.vs_uniforms, &f.vs_storage },
		{ "cs", lo.cs_uniforms, &f.vs_storage }, /* the CS reads the VS's storage */
	};
	for (int i = 0; i < 3; i++) {
		r = check_uniforms(us[i].stage, cl_mem + (us[i].at - bos[B_CL].gpu), &f.vp, us[i].st, h);
		if (r < 0)
			tri_failures++;
		else
			uniforms += r;
	}
	/* 6. the default attribute values the record points at: (0, 0, 0, 1.0f)
	 * for each of the 16 vec4 slots of a float attribute (v3dx_state.c:445-464) */
	int defaults = 0;
	for (int i = 0; i < 64; i++) {
		uint32_t got, want = (i % 4 == 3) ? f32(1.0f) : 0;
		memcpy(&got, cl_mem + (lo.defaults - bos[B_CL].gpu) + 4 * i, 4);
		if (got != want) {
			printf("frame_check: FAIL default attribute word %d: 0x%08x, expected 0x%08x\n",
			       i, got, want);
			tri_failures++;
			break;
		}
		defaults++;
	}
	if (defaults != 64)
		tri_failures++;

	/* 7. QUNIFORM_UNIFORM, whether or not this JSON uses it: the FS's stream
	 * with one {UNIFORM, 5} appended must carry st_fs[5] there, and the same
	 * frame with a 5-word storage must be refused, not read past the end */
	int storage_checks = 0;
	{
		static struct v3d_uniform u2[256];
		static uint8_t cl2[sizeof(cl_mem)];
		if (fs.num_uniforms + 1 > 256) {
			printf("frame_check: FAIL the FS has too many uniforms for the storage check\n");
			tri_failures++;
		} else {
			memcpy(u2, fs.uniforms, fs.num_uniforms * sizeof(u2[0]));
			u2[fs.num_uniforms] = (struct v3d_uniform){ V3D_QUNIFORM_UNIFORM, 5 };
			struct v3d_fs_prog fs2 = fs;
			fs2.uniforms = u2;
			fs2.num_uniforms = fs.num_uniforms + 1;
			struct v3d_triangle_frame f2 = f;
			f2.fs = &fs2;
			struct v3d_cl b2, r2, i2;
			struct v3d_triangle_layout lo2;
			v3d_cl_init(&b2, cl2, bos[B_CL].gpu, 0x1000);
			v3d_cl_init(&r2, cl2 + 0x1000, bos[B_CL].gpu + 0x1000, 0x2000);
			v3d_cl_init(&i2, cl2 + 0x3000, bos[B_CL].gpu + 0x3000, sizeof(cl2) - 0x3000);
			uint32_t got = 0;
			bool built = v3d_frame_build_triangle(&f2, &b2, &r2, &i2, &lo2);
			if (built)
				memcpy(&got, cl2 + (lo2.fs_uniforms - bos[B_CL].gpu) +
					     4 * fs.num_uniforms, 4);
			if (!built || got != st_fs[5]) {
				printf("frame_check: FAIL QUNIFORM_UNIFORM 5: built %d, 0x%08x, expected "
				       "0x%08x\n", built, got, st_fs[5]);
				tri_failures++;
			} else {
				storage_checks++;
			}
			f2.fs_storage.count = 5;
			v3d_cl_init(&b2, cl2, bos[B_CL].gpu, 0x1000);
			v3d_cl_init(&r2, cl2 + 0x1000, bos[B_CL].gpu + 0x1000, 0x2000);
			v3d_cl_init(&i2, cl2 + 0x3000, bos[B_CL].gpu + 0x3000, sizeof(cl2) - 0x3000);
			if (v3d_frame_build_triangle(&f2, &b2, &r2, &i2, &lo2)) {
				printf("frame_check: FAIL QUNIFORM_UNIFORM 5 of a 5-word storage was "
				       "accepted\n");
				tri_failures++;
			} else {
				storage_checks++;
			}
		}
	}
	if (storage_checks != 2)
		tri_failures++;

	int uniforms_want = (int)(fs.num_uniforms + vs.num_uniforms + cs.num_uniforms);
	int words_want = 2 * (int)(code[0].n + code[1].n + code[2].n);
	if (words != words_want || uniforms != uniforms_want || attrs_checked != TRI_NUM_ATTRS) {
		printf("frame_check: FAIL checked %d/%d code words, %d/%d uniforms, %d/%d attribute "
		       "records\n", words, words_want, uniforms, uniforms_want, attrs_checked,
		       TRI_NUM_ATTRS);
		tri_failures++;
	}

	/* the keys the JSON lacks were each taken from the header's --assume
	 * list, once per use above; that list must name every one of them */
	int assumed = 0;
	for (const char *q = TRI_ASSUMED; *q; q++)
		assumed += *q == '=';
	if (json_absent && !assumed) {
		printf("frame_check: FAIL %d JSON reads were absent and nothing was assumed\n",
		       json_absent);
		tri_failures++;
	}
	printf("frame_check: triangle %ux%u, %u supertile(s), BCL %u RCL %u indirect %u bytes, "
	       "%d packets, %d record fields, %d code words, %d uniforms checked against the "
	       "JSON, %d uniform-storage checks, %d reads absent from it (assumed: %s): %s\n",
	       w, h, supertiles, bcl.used, rcl.used, ind.used, census_total, fields_checked, words,
	       uniforms, storage_checks, json_absent, TRI_ASSUMED[0] ? TRI_ASSUMED : "none",
	       tri_failures ? "FAIL" : "ok");
	free(text);
	return tri_failures ? 1 : 0;
}
#endif

int main(int argc, char **argv)
{
	int a = 1;
	bool triangle = argc > 1 && strcmp(argv[1], "--triangle") == 0;
	if (triangle)
		a++;
	uint32_t w = argc > a + 1 ? strtoul(argv[a], NULL, 0) : 200;
	uint32_t h = argc > a + 1 ? strtoul(argv[a + 1], NULL, 0) : 100;
	if (triangle) {
#ifdef FRAME_CHECK_TRIANGLE
		const char *json = getenv("FRAME_CHECK_JSON");
		if (!json) {
			printf("frame_check: FAIL --triangle needs FRAME_CHECK_JSON=<v3d_shc JSON>\n");
			return 1;
		}
		return run_triangle(w, h, json);
#else
		printf("frame_check: FAIL built without -DFRAME_CHECK_TRIANGLE\n");
		return 1;
#endif
	}
	return run_clear(w, h);
}

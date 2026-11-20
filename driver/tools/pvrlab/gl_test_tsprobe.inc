// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
/* ---- cases 246 tsprobe and 247 dlayout (docs/notes/gc-ts-probe.md) --------
 * Included by gl_test.c after the other case files. The note designed them as
 * cases 243 and 244; those numbers had gone to divmix and w128tex, so they are
 * 246 and 247 here. The page tags are the note's.
 *
 * 246 tsprobe  the tile-status probe with the MMU off, "Part (iii)": one arm
 *              a boot (gl_test_tsprobe_arm, 0..3), run by the library's hook
 *              gc_debug_ts_probe (src/vivante/ts.rs): K0 the control resolve,
 *              K1 the TS fill, K2 the TS-sourced resolve, K3 the pattern
 *              resolve after a YES, each one kick of its own waited on with a
 *              2 s bound. No picture. The hook's readback (struct
 *              tsp_readback) is classified and packed here by
 *              gl_test_probe_decode.inc; the pages are tags 0xd0 + arm (the
 *              outcome), 0xd4 + arm (K3, only after YES) and 0xd8 + arm (the
 *              first difference).
 * 247 dlayout  the D24S8 surface's layout, "Part (ii)": one mode a boot
 *              (gl_test_tsprobe_arm, 0..3). A 64x64 depth ramp
 *              z(x, y) = (x + 64 y) / 8192 is drawn with GL_ALWAYS into a
 *              framebuffer with a 64x64 RGBA8 texture and a D24S8
 *              renderbuffer, cleared to depth 1.0 first; the hook
 *              gc_debug_depth_dump (src/vivante/depth_dump.rs) reads the
 *              surface the PE wrote and pd_decode decodes it here. Pages
 *              0xc0 + mode (A), 0xc4 + mode (B), 0xc8 + mode (C). Mode 3 is
 *              today's draw (gc_tiled_rt off), mode 1 a draw into a tiled
 *              shadow (gc_tiled_rt on, set by the case); modes 0 and 2 need a
 *              depth override in backend.rs that does not exist yet, and
 *              SKIP by name.
 *
 * PASS means the arm or mode ran to its end and the C twin read the hook's
 * words alike, whatever the outcome code says; FAIL means a step could not
 * run (the hook's negative code is the check's got) or the two decodes
 * differ. The answer is in the pages and in the console lines
 * "gl_test: tsprobe: ..." / "gl_test: dlayout: ...".
 *
 * Both need the GC7000, the test knobs and the sample option that builds them
 * (CONFIG_GL_TEST_GC_TS_PROBE, or GL_TEST_TS_PROBE on a host build); without
 * them they SKIP by name. The image's main sets gl_test_tsprobe_arm from the
 * selector (GC_RESULT_PERF_SEL | tag) and, on the board, the two hooks below
 * (PROGRESS records before each kick, the render service's fence timeouts).
 */
#include "gl_test_probe_decode.inc"

/* The arm (case 246) or mode (case 247) to run, 0..3. */
unsigned int gl_test_tsprobe_arm;
/* Optional: write a PROGRESS record (`where`) before a kick that may hang. */
void (*gl_test_probe_progress)(uint32_t where);
/* Optional: the fence timeouts the render service has counted so far. */
uint32_t (*gl_test_probe_timeouts)(void);

#define TSPROBE_PAGES 3u /* 0xd0 / 0xd4 / 0xd8 + arm; 0xc0 / 0xc4 / 0xc8 + mode */

static struct {
	uint32_t taken;                  /* bit p: page p of the case below exists */
	uint32_t base;                   /* 0xd0 (case 246) or 0xc0 (case 247) */
	uint32_t arm;                    /* the arm or mode the pages are of */
	uint32_t w[TSPROBE_PAGES][3];
} s_tsp_rec;

/* One page of the last run of case 246 or 247: tag 0xd0..0xdb or 0xc0..0xcb,
 * w[0..2] the record's w1..w3. 0 if the page exists, -1 if not (the case did
 * not run that arm or mode, or K3 did not run). */
int gl_test_tsprobe_page(unsigned int tag, uint32_t *w)
{
	unsigned int p;

	w[0] = w[1] = w[2] = 0;
	if (s_tsp_rec.taken == 0 || tag < s_tsp_rec.base || tag >= s_tsp_rec.base + 4u * TSPROBE_PAGES)
		return -1;
	p = (tag - s_tsp_rec.base) / 4u;
	if ((tag & 3u) != s_tsp_rec.arm || !(s_tsp_rec.taken & (1u << p)))
		return -1;
	memcpy(w, s_tsp_rec.w[p], sizeof(s_tsp_rec.w[p]));
	return 0;
}

#if defined(CONFIG_GL_TEST_GC_TS_PROBE) || defined(GL_TEST_TS_PROBE)
#define GL_TEST_TSPROBE 1
#endif

#if defined(GL_TEST_VIVANTE) && defined(GL_TEST_KNOBS) && defined(GL_TEST_TSPROBE)
/* src/vivante/ts.rs and depth_dump.rs, the `test-knobs` hooks */
extern int gc_debug_ts_probe(uint32_t arm, struct tsp_readback *out, uint32_t out_bytes,
			     void (*progress)(uint32_t step));
extern int gc_debug_depth_dump(uint32_t mode, uint32_t timeouts, uint32_t *surf, uint32_t *pages);
extern void gc_debug_set_tiled_rt(int on);
extern int gc_debug_tiled_rt;

static void tsprobe_keep(uint32_t base, uint32_t arm, unsigned int p, const uint32_t w[3])
{
	if (s_tsp_rec.base != base || s_tsp_rec.arm != arm)
		s_tsp_rec.taken = 0;
	s_tsp_rec.base = base;
	s_tsp_rec.arm = arm;
	memcpy(s_tsp_rec.w[p], w, sizeof(s_tsp_rec.w[p]));
	s_tsp_rec.taken |= 1u << p;
	pr_info("gl_test: %s: page 0x%02x %08x %08x %08x\n", base == TSP_TAG_P ? "tsprobe" : "dlayout",
		(unsigned)(base + 4u * p + arm), (unsigned)w[0], (unsigned)w[1], (unsigned)w[2]);
}

static uint32_t s_tsp_where; /* 0x200 | arm << 4 (246), 0x300 | mode << 4 (247) */

static void tsprobe_progress(uint32_t step)
{
	if (gl_test_probe_progress)
		gl_test_probe_progress(s_tsp_where | step);
}

static uint32_t tsprobe_timeouts(void)
{
	return gl_test_probe_timeouts ? gl_test_probe_timeouts() : 0u;
}

static const char *const tsp_code_name[16] = {
	[TSP_GUARD] = "GUARD", [TSP_NO_CONTROL] = "NO_CONTROL", [TSP_FILL_MISS] = "FILL_MISS",
	[TSP_YES] = "YES", [TSP_IGNORED] = "IGNORED", [TSP_NO_WRITE] = "NO_WRITE",
	[TSP_MIXED] = "MIXED", [TSP_HANG] = "HANG",
};

static const char *const pd_code_name[16] = {
	[PD_NO_WRITE] = "NO_WRITE", [PD_ROW_LINEAR_CODE] = "ROW_LINEAR", [PD_TILED4_CODE] = "TILED4",
	[PD_SUPER_DRM_CODE] = "SUPER_DRM", [PD_SUPER_INTERLEAVED_CODE] = "SUPER_INTERLEAVED",
	[PD_OTHER] = "OTHER", [PD_PARTIAL] = "PARTIAL", [PD_HANG] = "HANG",
};

static const char *code_name(const char *const *t, uint32_t c)
{
	return c < 16u && t[c] ? t[c] : "?";
}
#endif

static int case_tsprobe(void)
{
#if defined(GL_TEST_VIVANTE) && defined(GL_TEST_KNOBS) && defined(GL_TEST_TSPROBE)
	static struct tsp_readback rb;
	const uint32_t arm = gl_test_tsprobe_arm & 3u;
	uint32_t w[3], code;
	int rc;
	bool ok;

	memset(&rb, 0, sizeof(rb));
	s_tsp_rec.taken = 0;
	s_tsp_where = 0x200u | arm << 4;
	/* the hook waits for the back end's kicks too; nothing is drawn here */
	glFinish();
	rc = gc_debug_ts_probe(arm, &rb, (uint32_t)sizeof(rb), tsprobe_progress);
	/* check 1: got = the hook's refusal (minus its code), 0 when it ran */
	if (!expect_u32("tsprobe: the hook ran every step it could", rc < 0 ? (uint32_t)-rc : 0u, 0u)) {
		pr_info("gl_test: tsprobe: arm %u: the hook could not run a step (%d)\n", (unsigned)arm, rc);
		return -1;
	}
	code = tsp_classify(&rb);
	tsp_pack_p(&rb, w);
	tsprobe_keep(TSP_TAG_P, arm, 0, w);
	if (rb.k3_present) {
		tsp_pack_k3(&rb, w);
		tsprobe_keep(TSP_TAG_P, arm, 1, w);
	}
	tsp_pack_x(&rb, w);
	tsprobe_keep(TSP_TAG_P, arm, 2, w);
	pr_info("gl_test: tsprobe: arm %u: %s (code %u), %u steps, %u fence timeouts, %u guard dwords changed, "
		"K2 C %u B %u dead %u of 4096\n", (unsigned)arm, code_name(tsp_code_name, code), (unsigned)code,
		(unsigned)rb.steps_done, (unsigned)rb.timeouts, (unsigned)rb.guard_changed, (unsigned)rb.n_clear,
		(unsigned)rb.n_detile, (unsigned)rb.n_dead);
	if (rb.k3_present)
		pr_info("gl_test: tsprobe: arm %u: K3 row-0 map 0x%02x (expected 0x%02x low nibble first), "
			"nibble order 0x%02x\n", (unsigned)arm, (unsigned)rb.k3_map,
			(unsigned)tsp_k3_expected_map(arm == 3u ? PD_SUPER_DRM : PD_TILED4, arm == 2u ? 256u : 128u, 1),
			(unsigned)rb.k3_nibble_order);
	/* check 2: the C twin classifies the record as the library did */
	ok = expect_u32("tsprobe: the C twin's code is the hook's", code, (uint32_t)rc);
	return ok ? 0 : -1;
#elif defined(GL_TEST_VIVANTE)
	pr_info("gl_test: case 246 tsprobe: SKIP (needs CONFIG_YAKOGL_TEST_KNOBS and CONFIG_GL_TEST_GC_TS_PROBE)\n");
	return -3;
#else
	pr_info("gl_test: case 246 tsprobe: SKIP (the GC7000's tile status)\n");
	return -3;
#endif
}

static int case_dlayout(void)
{
#if defined(GL_TEST_VIVANTE) && defined(GL_TEST_KNOBS) && defined(GL_TEST_TSPROBE)
	/* The plane z = (x + 64 y) / 8192 at the corners of the 64x64 window,
	 * in clip space (z_ndc = 2 z - 1): (0,0) 0, (64,0) 64/8192,
	 * (0,64) 4096/8192, (64,64) 4160/8192. A framebuffer object is not
	 * mirrored, so window row y is surface row y. */
	static const float v[18] = {
		-1, -1, -1.0f,              1, -1, 128.0f / 8192 - 1,   -1, 1, 0.0f,
		-1, 1, 0.0f,                1, -1, 128.0f / 8192 - 1,   1, 1, 8320.0f / 8192 - 1,
	};
	static uint32_t surf[PD_N];
	static struct pd_work work;
	static struct pd_result res;
	const uint32_t mode = gl_test_tsprobe_arm & 3u;
	uint32_t hook[9], a[3], b[3], c[3], t0, timeouts, code;
	GLuint prog, tex = 0, rb = 0, fbo = 0;
	GLenum status, err;
	int was = gc_debug_tiled_rt, rc;
	bool ok = true;

	s_tsp_rec.taken = 0;
	s_tsp_where = 0x300u | mode << 4;
	if (mode == 0u || mode == 2u) {
		pr_info("gl_test: case 247 dlayout: SKIP (mode %u needs the depth override in backend.rs; "
			"modes 1 and 3 run)\n", (unsigned)mode);
		return -3;
	}
	free_all_objects();
	while (glGetError() != GL_NO_ERROR)
		;
	prog = link_program(gl_test_passthrough_vs_src, gl_test_uniform_fs_src);
	/* check 1 */
	if (!expect_u32("dlayout: program linked", prog ? 1u : 0u, 1u))
		return -1;
	glGenTextures(1, &tex);
	glBindTexture(GL_TEXTURE_2D, tex);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
	glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
	glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
	glBindTexture(GL_TEXTURE_2D, 0);
	glGenRenderbuffers(1, &rb);
	glBindRenderbuffer(GL_RENDERBUFFER, rb);
	glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8_OES, TW, TH);
	glBindRenderbuffer(GL_RENDERBUFFER, 0);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
	glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
	glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb);
	status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
	err = glGetError();
	/* check 2: got = the GL error, else the status */
	if (!expect_u32("dlayout: framebuffer complete", err != GL_NO_ERROR ? err : status, GL_FRAMEBUFFER_COMPLETE)) {
		ok = false;
		goto out;
	}

	/* mode 1 is a draw into a tiled shadow, mode 3 today's draw */
	gc_debug_set_tiled_rt(mode == 1u);
	glFinish();
	t0 = tsprobe_timeouts();
	tsprobe_progress(1);
	glViewport(0, 0, TW, TH);
	glDisable(GL_SCISSOR_TEST);
	glDisable(GL_STENCIL_TEST);
	glDisable(GL_CULL_FACE);
	glDisable(GL_BLEND);
	glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
	glDepthMask(GL_TRUE);
	glClearColor(0.0f, 0.0f, 0.0f, 1.0f);
	glClearDepthf(1.0f);
	glClearStencil(0);
	glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
	glEnable(GL_DEPTH_TEST);
	glDepthFunc(GL_ALWAYS);
	glUseProgram(prog);
	glUniform4f(glGetUniformLocation(prog, "u_color"), 0.0f, 1.0f, 0.0f, 1.0f);
	glBindBuffer(GL_ARRAY_BUFFER, 0);
	glVertexAttribPointer(0, 3, GL_FLOAT, GL_FALSE, 12, v);
	glEnableVertexAttribArray(0);
	glDisableVertexAttribArray(1);
	glDrawArrays(GL_TRIANGLES, 0, 6);
	glDisableVertexAttribArray(0);
	tsprobe_progress(2);
	glFinish();
	timeouts = tsprobe_timeouts() - t0;
	err = glGetError();
	tsprobe_progress(6);
	rc = gc_debug_depth_dump(mode, timeouts, surf, hook);
	gc_debug_set_tiled_rt(was);
	/* check 3: got = the hook's refusal (minus its code), 0 when it read */
	if (!expect_u32("dlayout: the hook read the surface", rc < 0 ? (uint32_t)-rc : 0u, 0u)) {
		pr_info("gl_test: dlayout: mode %u: the hook could not read the surface (%d%s)\n", (unsigned)mode, rc,
			rc == -5 ? ": not the mode's layout; mode 1 needs a draw into a tiled shadow" : "");
		ok = false;
		goto out;
	}
	pd_decode(surf, 0xffffff00u, &work, &res);
	pd_pack_a(&res, mode, timeouts, a);
	pd_pack_b(&res, b);
	pd_pack_c(&res, c);
	tsprobe_keep(PD_TAG_A, mode, 0, a);
	tsprobe_keep(PD_TAG_A, mode, 1, b);
	tsprobe_keep(PD_TAG_A, mode, 2, c);
	code = a[0] & 0xfu;
	pr_info("gl_test: dlayout: mode %u: %s (code %u), %u of 4096 ids found once, %u fence timeouts, "
		"first non-clear dword %u\n", (unsigned)mode, code_name(pd_code_name, code), (unsigned)code,
		(unsigned)res.matched, (unsigned)timeouts, (unsigned)res.first_idx);
	/* check 4: no GL error in the draw */
	ok &= expect_u32("dlayout: no GL error", err, GL_NO_ERROR);
	/* checks 5 to 7: the C twin's pages are the library's, word for word */
	ok &= expect_u32("dlayout: page A is the hook's", (a[0] ^ hook[0]) | (a[1] ^ hook[1]) | (a[2] ^ hook[2]), 0u);
	ok &= expect_u32("dlayout: page B is the hook's", (b[0] ^ hook[3]) | (b[1] ^ hook[4]) | (b[2] ^ hook[5]), 0u);
	ok &= expect_u32("dlayout: page C is the hook's", (c[0] ^ hook[6]) | (c[1] ^ hook[7]) | (c[2] ^ hook[8]), 0u);
	/* check 8: the codes agree */
	ok &= expect_u32("dlayout: the C twin's code is the hook's", code, (uint32_t)rc);
out:
	gc_debug_set_tiled_rt(was);
	glDepthFunc(GL_LESS);
	glDisable(GL_DEPTH_TEST);
	glBindFramebuffer(GL_FRAMEBUFFER, 0);
	glDeleteFramebuffers(1, &fbo);
	glDeleteRenderbuffers(1, &rb);
	glDeleteTextures(1, &tex);
	glUseProgram(0);
	glDeleteProgram(prog);
	return ok ? 0 : -1;
#elif defined(GL_TEST_VIVANTE)
	pr_info("gl_test: case 247 dlayout: SKIP (needs CONFIG_YAKOGL_TEST_KNOBS and CONFIG_GL_TEST_GC_TS_PROBE)\n");
	return -3;
#else
	pr_info("gl_test: case 247 dlayout: SKIP (the GC7000's depth surface)\n");
	return -3;
#endif
}

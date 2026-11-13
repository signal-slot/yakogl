<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: drawing on the PE's fourth render target — the board check

Design note of plan #335, item gc-mrt4-note, for items gc-mrt4-gltest and the
board session. RC14 (`docs/es3-cts-failures.md:274-286`): the GL core caps the
draw-buffer count at the back end's `DRAW_TARGETS`, 3 on every GPU
(`src/backend.rs:793-810`), where ES 3.0 asks for 4 (Table 6.34, cited at
`src/backend.rs:781-782`). The GC7000UL's PE has four colour targets, and
`src/vivante` already programs four — what no board has done is *draw* with
four. Plan #335 keeps `DRAW_TARGETS` at 3 on `gpu-vivante` until one does.
This note gives the register words of the fourth target, the gl_test case that
draws four targets, the opt-in knob that lets a board image ask for four
targets while a product library stays at three, the board procedure and its
verdict lines, and what must hold before the flip. **No code change comes with
it, and nothing here was run on a board.**

Sources: Mesa 26.2.1, tag `mesa-26.2.1` (`third_party/mesa-26.2.1/UPSTREAM`);
as in `gc-texture-swizzle-wrap.md` section 1 the etnaviv driver is not in
`third_party/`, and was read from that same release's tree in the deps cache
(`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1/src/gallium/drivers/etnaviv/`,
cited as `etnaviv_*.c` with the 26.2.1 line numbers). The register names are
the repo's own rnndb cut, `driver/src/vivante/hw/state_3d.xml.h`, cited as
`state_3d.xml.h`. Own code is cited at the tip `57d5579c`; items running
alongside edit `src/backend.rs`, `src/vivante/backend.rs` and
`src/gl/context.rs`, so **line numbers in those three files can drift** —
each is given with the function that holds it.

## 1. (a) The PE and shader-state words of the fourth target

### 1.1 Four is the number for this core, and no new register is needed

`etna_determine_num_rts` gives `num_rts = 8` for HALTI2+ (etnaviv_screen.c
`:774-781`), but `caps->max_render_targets` is `num_rts / 2 = 4` for every
HALTI2+ core, half the RTs kept for 128-bit formats (etnaviv_screen.c
`:299-304`; the same reading in `src/vivante/VIVANTE_NOTES.md:1096-1100` — a
different Mesa cut, hence its line numbers differ). 4 is Mesa's number for the
GC7000UL and the ES 3.0 minimum, and `GLES_MAX_TARGETS` is 4
(`src/backend.rs:774`), so the fourth target is *not* new state: it is the
fourth element of the 8-entry banks this backend already programs for targets
1-3, the fourth byte of `PS_OUTPUT_REG`, the third saturate bit of
`PS_CONTROL` and the `RT_COUNT` field counting one more. Mesa's targets 4-7
need second registers (`PS_OUTPUT_REG2` 0x102c, its saturate bits, emitted
only when `num_rts == 8`: etnaviv_shader.c `:217-222`, etnaviv_emit.c
`:476-477`, etnaviv_state.c `:363-366`); four targets never touch them.
Independent blend exists on HALTI5 (etnaviv_screen.c `:305-306`) and ES 3.0
has one blend state for every draw buffer; nothing here needs it.

The GL core already reaches four with a test back end: `GLES_MAX_TARGETS` is
the array bound (`src/backend.rs:774`), `max_draw_buffers()` and
`max_color_attachments()` both read `draw_targets()`
(`src/gl/context.rs:154-156,163-165`, `crate::backend::draw_targets`
`src/backend.rs:822-831`), `gl_MaxDrawBuffers` the same function at compile
time (`src/shaderc/online/lower.rs:12639,12869`,
`src/shaderc/online/reflect.rs:1289`), and `tests/es3_mrt_gl.rs` already
walks a four-target draw through the GL core with `HOST_DRAW_TARGETS = 4`
(`tests/es3_mrt_gl.rs:317,343,378`). The vivante back end takes four as it
stands: `mrt_targets` clamps to 4 (`src/vivante/backend.rs:509-511`, the
function's doc says "up to the four this core drives"), and
`src/vivante/backend_tests.rs:3715-3769`
(`draw_buffers_program_the_8_rt_bank`, "MRT step 2") pins today, at
`num_targets = 4`, every word below — the bank's address/stride/format/mask/
blend for RT1-3, `PS_OUTPUT_REG == 3 | 4<<8 | 5<<16 | 6<<24`, `RT_COUNT(3)`,
all four saturate bits, and `RT_COUNT(0)` with no bank write for the following
one-target draw. **What is unproven is exactly the silicon half: that the PE
writes the fourth surface.**

### 1.2 The per-target words (rt = 3, bank index `rt - 1` = 2)

RT0 keeps `PE_COLOR_FORMAT` 0x142c / `PE_COLOR_STRIDE` 0x1434 /
`PE_PIPE_COLOR_ADDR(0)` (`src/vivante/regs.rs:504,529,531`); RT n >= 1 uses
index n-1 in the 8-banks, as Mesa programs a HALTI2+ core
(`emit_pe_state`, `src/vivante/backend.rs:1459`, the bank loop at
`:1698-1725`):

| word | address for rt = 3 | ours (function) | rnndb | Mesa |
|---|---|---|---|---|
| address, pipe 0 | `0x14800 + 0x20*2` = 0x14840 | `pe_rt_addr_8_pipe` `regs.rs:55-57`; written `backend.rs:1716` | `state_3d.xml.h:1200-1202` (`__LEN` 8) | etnaviv_emit.c `:609-614` |
| stride, format, tiling | `0x14900 + 4*2` = 0x14908 | `pe_rt_config_8`, `rt_config_stride/format` `regs.rs:59-66`; `backend.rs:1718` | `state_3d.xml.h:1204-1206` | etnaviv_state.c `:321-325` |
| sRGB encode (RT 1+) | bit 30 of that word | `GC_RT_CONFIG_SRGB` `regs.rs:68-70`; `backend.rs:1717` | (`RT_CONFIG_SRGB`) | etnaviv_state.c `:326-328` |
| colour mask + OVERWRITE | `0x14920 + 4*2` = 0x14928 | `pe_halti5_rt_colormask(_components)` `regs.rs:72-78`; `backend.rs:1719-1723` | `state_3d.xml.h:1208-1214` (COMPONENTS bits 4-7, OVERWRITE bit 8, `__LEN` 7) | etnaviv_emit.c `:162-166` |
| blend (PE_ALPHA_CONFIG layout) | `0x14960 + 4*2` = 0x14968 | `pe_halti5_rt_alpha_config` `regs.rs:80-82`; `backend.rs:1724` | `state_3d.xml.h:1227-1229` | etnaviv_emit.c `:167-170` |
| blend colour | 0x14980 / 0x149a0 + 4*2 | **not written for RT >= 1** — see "the one gap" below | `state_3d.xml.h:1259-1277` | etnaviv_blend.c `:216,231-234`, etnaviv_emit.c `:172-180` |

* **The one gap.** Mesa writes the constant blend colour into *every* target's
  `PE_HALTI5_RT_ALPHA_COLOR_EXT0/1` (etnaviv_blend.c `:231-234`), the backend
  writes only RT0's `PE_ALPHA_COLOR_EXT0/1` (`backend.rs:1761-1762`). A
  constant-colour blend reaching a target past the first therefore reads an
  unwritten colour word. This predates four targets — it is true of targets 1
  and 2 today — and gl_test case 106 draws with `GL_BLEND` off
  (`driver/tools/pvrlab/gl_test.c:13992`), as the case here does. It is not
  part of the mrt4 flip; it should be its own issue.
* **One pixel pipe.** This part has a single pipe (the hwdb row,
  `VIVANTE_NOTES.md` MRT section); RT0 is written to both pipe registers
  (`backend.rs:1727-1742` "one pixel pipe on this part: both pipe registers
  name the same surface") while RT1-3 are written for pipe 0 only
  (`backend.rs:1716`), where Mesa emits both (etnaviv_emit.c `:610-612`).
  Three-target board runs (gl_test 106, 163: `VIVANTE_NOTES.md:1088-1091`)
  are the evidence that pipe 1 of RT1+ is never read; the four-target run is
  one more data point, not a new risk.
* **No tile status**: `PE_MEM_CONFIG`/`TS_MEM_CONFIG` are written 0
  (`backend.rs:1764-1765`), so Mesa's per-RT TS banks (etnaviv_emit.c
  `:617-624`) do not come up.

### 1.3 The shader-side words

* `PS_OUTPUT_REG` 0x1004 (`regs.rs:350`) maps each target to its fragment
  output register, one 7-bit field each (rt's field at bit 8*rt; rt 3's field
  is bits 24-30, `state_3d.xml.h:622-633`); our byte-wise write is
  `src/vivante/shader.rs:313-337`: target k names the shader row with
  `slot == 4 + out_shift + GL-slot(k)`, and an unwritten output falls back to
  RT0's register (`shader.rs:316-318` — that is how a clear's one colour
  reaches all four). Mesa builds the same word from the framebuffer's
  `ps_output_remap` (etnaviv_shader.c `:207-215`).
* `PS_CONTROL` 0x1010 (`regs.rs:359`): `RT_COUNT(count - 1)` at bits 8-10
  (`ps_control_rt_count` `regs.rs:361-363`, written `shader.rs:346`;
  `state_3d.xml.h:655-657`) — RT_COUNT(3) is four targets, and RT_COUNT(0)
  implicitly disables the targets past the first (etnaviv_state.c
  `:390-397`); per-target unorm saturate `SATURATE_RT0 << rt`
  (`ps_control_saturate_rt` `regs.rs:84-86`, loop `shader.rs:348-357`;
  `state_3d.xml.h:651-654` — `SATURATE_RT3` 0x10). Four targets stay inside
  this one word: Mesa moves the saturate of targets 4-7 to `PS_OUTPUT_REG2`
  (etnaviv_state.c `:363-366`, asserts `:353-361`).
* `PS_CONTROL_EXT` 0x1030 (`regs.rs:367`): the colour-output mode of rt in
  bits `4*rt..4*rt+2` (`ps_output_mode` `regs.rs:47-49`, `shader.rs:358`;
  `state_3d.xml.h:705-707` for MODE3, values 130-136; Mesa shifts the same
  `4 * rt`, etnaviv_state.c `:376-377`). RGBA8 targets take mode NORMAL(0),
  so four plain RGBA8 targets leave this word's top nibble at 0.
* **Null-slot compaction**: a GL slot with `gpu == 0` (a `GL_NONE` draw
  buffer) is skipped and later slots move down one hardware target, each
  keeping its own shader output through `PS_OUTPUT_REG`
  (`hw_targets` `backend.rs:525-535`, pinned with three slots at
  `backend_tests.rs:3780-3830`); Mesa compacts identically (etnaviv_state.c
  `:239-240` skips null `cbufs`, `:382-383` `ps_output_remap[rt] =
  rt_output[i]`, `:388-391` RT_COUNT from the real targets only).
* **Per-draw keying**: the dedup key of the shader group carries the slot
  list, the target count and `out_shift` (`shader_key`,
  `backend.rs:2098-2104`), so a four-target draw never reuses a three-target
  group's partially written words.

## 2. (b) The gl_test case: 222 `mrt4`

Modelled line-for-line on case 106 `mrt3`
(`driver/tools/pvrlab/gl_test.c:13922-14048`), the GC's three-buffer case
that runs on this board. Case numbers in use run to 221 (the name table at
`gl_test.c:8563`); 222 is free.

**What it draws.** Four 64x64 `GL_RGBA8` texture attachments
(`TW`/`TH` are 64, `gl_test.c:992-993`) on one FBO — RGBA8 because it is
unorm, blended-off and needs no output mode, so every failure reads as the
wrong colour and not as an unsupported format. A new four-output program:
the mrt3 vertex shader unchanged (`tools/shaders/es300/mrt3.vert`) and a new
fragment `tools/shaders/es300/mrt4.frag`, the mrt3 text plus
`layout(location = 3) out vec4 o3; ... o3 = vec4(1.0, 1.0, 0.0, 1.0);`
(yellow). Distinct constant colours, no gradients: at 64x64 a centre pixel
read is exact, and a gradient would blur "RT3 unwritten" with "RT3 wrote a
different output".

Four parts, each read back at the centre through `glReadBuffer` (the
`mrt3_read` pattern, `gl_test.c:13941-13948`) **between** the stages, which
is what case 106 learned from its dead checks
(`gl_test.c:13929-13935`):

1. `glDrawBuffers(4, {ATT0..ATT3})`, clear to grey: all four attachments must
   hold grey — the clear quad's single colour through `PS_OUTPUT_REG`'s
   fallback reaches all four targets at `RT_COUNT(3)`.
2. One draw (full-screen quad, the program above): red, green, blue, yellow.
   **Attachment 3 turning yellow is the board's proof for RC14.**
3. `glDrawBuffers(4, {ATT0, ATT1, GL_NONE, ATT3})`, clear to black: 0, 1, 3
   black; attachment 2 still blue (nothing is programmed for a null slot).
   At four slots the compaction moves ATT3 to hardware RT2 — a second
   exercise of `hw_targets`' skip-and-move-down (`backend.rs:513-535`).
4. One draw of the same program: red, green, blue (untouched since part 2),
   yellow again — ATT3's fragment output (o3) still lands in ATT3 through
   `PS_OUTPUT_REG` byte 2 after the compaction.

Expected pixels (`0xAABBGGRR`, `rgba_u32` `gl_test.c:6399-6402`):

| attachment | part 1 clear | part 2 draw | part 3 hole clear | part 4 hole draw |
|---|---|---|---|---|
| 0 | 0xff808080 | 0xff0000ff | 0xff000000 | 0xff0000ff |
| 1 | 0xff808080 | 0xff00ff00 | 0xff000000 | 0xff00ff00 |
| 2 | 0xff808080 | 0xffff0000 | 0xffff0000 | 0xffff0000 |
| 3 | 0xff808080 | 0x0000ffff | 0xff000000 | 0x0000ffff |

The one recorded check is a fail mask, as case 106's: bits 0-3 the clear,
4-7 the draw, 8-11 the NONE-slot draw, 12-15 the NONE-slot clear, bit n of
each group attachment n; `expect_int(... , fail, 0)`
(`gl_test.c:2921`, case 106's use at `:14047`). A `GL_MAX_DRAW_BUFFERS` read
below 4 skips the case (`return -3`, the skip code the runner counts as
neither pass nor fail, `driver/tools/pvrlab/runner/src/lib.rs:64-66`),
exactly case 106's skip at `gl_test.c:13964-13968`.

**C sketch**, against the existing structure (gl_test.c is the suite,
`main_imx8mp.c` only starts the backend and reports):

```c
/* ---- case 222: four draw buffers (RC14, the GC7000UL's PE fourth RT) ----- */
static int case_mrt4(void)
{
	static const GLenum all[4]  = { GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1,
					GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3 };
	static const GLenum hole[4] = { GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1,
					GL_NONE, GL_COLOR_ATTACHMENT3 };
	const uint32_t colour[4] = { rgba_u32(255, 0, 0, 255), rgba_u32(0, 255, 0, 255),
				     rgba_u32(0, 0, 255, 255), rgba_u32(255, 255, 0, 255) };
	const uint32_t grey = rgba_u32(128, 128, 128, 255), black = rgba_u32(0, 0, 0, 255);
	GLuint prog, fbo = 0, tex[4] = { 0, 0, 0, 0 };
	GLint maxdb = 0;
	uint32_t fail = 0;

	glGetIntegerv(GL_MAX_DRAW_BUFFERS, &maxdb);
	if (maxdb < 4) {
		pr_info("gl_test: case 222 mrt4: SKIP (GL_MAX_DRAW_BUFFERS %d < 4)\n", (int)maxdb);
		return -3;
	}
	free_all_objects();
	prog = link_program(gl_test_es300_mrt4_vs_src, gl_test_es300_mrt4_fs_src);
	if (!prog)
		return -1;
	glGenTextures(4, tex);
	glGenFramebuffers(1, &fbo);
	glBindFramebuffer(GL_FRAMEBUFFER, fbo);
	for (int n = 0; n < 4; n++) {
		glBindTexture(GL_TEXTURE_2D, tex[n]);
		glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, TW, TH, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
		glFramebufferTexture2D(GL_FRAMEBUFFER, all[n], GL_TEXTURE_2D, tex[n], 0);
	}
	if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE)
		return -1;
	glDrawBuffers(4, all);
	/* the quad upload and the GL state of case_mrt3 (gl_test.c:13988-13993),
	 * glDisable(GL_BLEND) included; then the four parts of section 2, each:
	 *   glClearColor/glClear or glDrawArrays, glFinish(),
	 *   for n in 0..3 { uint32_t got = mrt3_read(n);        /* glReadBuffer(ATT n) */
	/*     pr_info per attachment as in case 106 (gl_test.c:14001), and
	 *     if (chan_diff(got, want) > 2)
	 *         fail |= 1u << (group * 4 + n); }                                  */
	return expect_int("mrt4 fail mask (0-3 clear, 4-7 draw, 8-11 NONE draw, 12-15 NONE clear)",
			  (long)fail, 0) ? 0 : -1;
}
```

Joins in `gl_test.c`: the case's comment block by the set's contents list
(the 106 line at `:193`), the name table (`[222] = "mrt4"`, `:8549-8563`),
and the dispatch (`case 222: rc = case_mrt4(); break;`, the 106 line at
`:23494`). It does **not** join the regression set `SET_ORDER`
(`driver/tools/pvrlab/runner/src/lib.rs:53-56`) — case 106 is not in it
either; the case runs alone (`CONFIG_GC_GL_TEST_CASE=222`) or inside a sweep
whose 48-case window covers 222. The case body compiles unconditionally, like
every other case, and stays inert without the knob; the capability, not the
case text, is what the opt-in builds.

The new shader text must join `tools/shaders/` (the generated headers are the
gate: `gen_gl_test_shaders.py --check`, `tools/check.sh:2391-2392`, and
`gl_test_shaders.h` is GENERATED from it, its header line says), with
`pair es300/mrt4.vert es300/mrt4.frag` in `tools/shaders/precompiled.list`
beside the two mrt3 lines (`:516`, `:668`, under `app gl_test` `:445` and
`app gl_test_es100` `:597`), regenerated in the same commit.

## 3. (c) The opt-in knob, and where it joins (nothing here is edited)

Two layers, each with an existing precedent:

* **Library test knob** `gc_debug_draw_targets` (an int; 0 = the product
  answer) with its setter `gc_debug_set_draw_targets(int) -> int`, behind the
  crate's `test-knobs` feature (`Cargo.toml:109-119`, `YAKOGL_TEST_KNOBS`
  `CMakeLists.txt:107`) and `gpu-vivante`, in the `gc_debug_*` namespace
  (`gc_debug_msaa` / `gc_debug_set_msaa` and `gc_debug_narrow_la` at
  `src/vivante/backend.rs` `:849`ff / `:440-452` are the models). It feeds
  `draw_targets()` (`src/backend.rs:822-831`) the same way `HOST_DRAW_TARGETS`
  feeds it in host tests (`src/backend.rs:817-818,823-828`): a non-zero value, clamped
  to `GLES_MAX_TARGETS`, is what `GL_MAX_DRAW_BUFFERS`,
  `GL_MAX_COLOR_ATTACHMENTS`, the attachment checks and `gl_MaxDrawBuffers`
  read. It must be set **before context creation**: a program bakes
  `gl_MaxDrawBuffers` at compile time (`src/shaderc/online/lower.rs:12869`),
  so a mid-run change speaks only to programs compiled after it.
* **Image knob** `CONFIG_GC_GL_TEST_MRT4` (bool, `depends on
  YAKOGL_TEST_KNOBS`, default n), a new symbol in the i.MX8M Plus menu of
  `driver/samples/zephyr/gl_test/Kconfig` (the `GC_GL_TEST_MSAA` block at
  `:335-345` is the model: a startup knob whose help names the measurement and
  says products leave it off). `main_imx8mp.c` calls
  `gc_debug_set_draw_targets(4)` in a block beside the `gc_debug_set_msaa`
  one (`driver/samples/zephyr/gl_test/src/main_imx8mp.c:452-461`), before
  `gl_test_run`. The same `main_imx8mp.c` is the Armadillo-X2's (its conf is
  added after the i.MX8MP's, `driver/samples/zephyr/gl_test/CMakeLists.txt:24-36`),
  so the case is one build per board, not two sources.

Where these join once gc-mrt4-gltest lands them (this note edits none of it):

* `Cargo.toml`: the `test-knobs` feature's comment, which names every knob of
  every GPU (`Cargo.toml:109-119`) — `gc_debug_draw_targets` joins the GC7000
  list. No new crate feature is needed, and one is better not added: a feature
  that flipped the `DRAW_TARGETS` const would duplicate the knob machinery and
  put a test branch into the product build's one read of that const.
* `tools/check.sh`: the C symbol names join `KNOB_VIVANTE_SYMBOLS`
  (`tools/check.sh:2730-2732`), which the "no test knob in the artefact" row
  requires absent from a product build (`tools/check.sh:3073`); and the
  knob's unit test joins the `KNOBS_TESTS` list by name
  (`tools/check.sh:1373-1384`), the row that runs `KNOBS_CONFIG =
  host-test,gpu-vivante,shaderc-online,test-knobs` and fails if a listed knob
  test stops compiling. The gltest `cc -fsyntax-only` arms
  (`tools/check.sh:2345-2348`) need no change: the case body compiles in all
  of them and skips at run time without the knob.
* `driver/samples/zephyr/gl_test/Kconfig` + `boards/*.conf`: nothing in the
  confs — the symbol defaults n; an image turns it on with
  `-DCONFIG_GC_GL_TEST_MRT4=y` (like the sweep's `-DCONFIG_...` examples in
  `prj_sweep.conf:6-8`).

For the planner: once the knob is in the tree, the four-target dEQP probe
(gc-mrt4-deqp-probe, currently editing `src/backend.rs` for a scratch build)
can set `gc_debug_set_draw_targets(4)` in the dEQP image instead, and no
longer needs the `src/backend.rs` edit.

## 4. (d) The board procedure and the verdict lines

The image and record mechanics are the suite's; this is the ordering. No
board address, IP or console path belongs in this note — the runner
(`tools/board.sh`), the board recipe (`docs/es3/board-recipe-imx8mp.md`) and
the boot notes (`docs/notes/imx8mp-direct-boot.md`,
`docs/notes/imx8mp-netboot.md`) hold the environment.

1. Build, for the Verdin i.MX8MP (and, as the second data point, the
   Armadillo-X2 — same SoC family, `armadillo_x2/mimx8ml8/a53`):
   `west build -b verdin_imx8mp/mimx8ml8/a53 <gl_test> -- -DCONFIG_GC_GL_TEST_MRT4=y
   -DCONFIG_GC_GL_TEST_CASE=222` with the ES 3.0 shader set (the default; not
   `-DGL_TEST_ES100=ON`). A scratch tree with `DRAW_TARGETS = 4` in
   `src/backend.rs` needs neither knob for the library and must *not* be the
   tree that gets merged: the product library stays at 3 and only the image
   asks for 4.
2. Boot and run once, per the recipe; the image writes its SNVS record and
   resets the SoC (`main_imx8mp.c:18-25`), so one boot answers one question.
3. Read, in this order:
   * the console/panel lines, one per check, in case 106's shape:
     `gl_test:   mrt4 clear attachment 3 = 0xff808080  want 0xff808080`,
     `gl_test:   mrt4 draw attachment 3 = 0x0000ffff  want 0x0000ffff`, then
     the two `hole clear` / `hole draw` lines;
   * the runner's one line for the case: `222 mrt4 PASS`, `FAIL` or `SKIP`;
   * the SNVS CASE record (gc_result.h): PASS, or the first bad check with
     `got = fail mask`, `want = 0` (`report_result`,
     `main_imx8mp.c:426-436`); a kick that never completed records the case
     as UNFINISHED instead (`main_imx8mp.c:156-159`);
   * the hold code, if the image reports checks: 1 = pass, n+1 = the first
     failing check, 63 = failed with no check (a GL error,
     `main_imx8mp.c:681-695`); with `GC_GL_TEST_REPORT_CHECK_COLOUR` it
     carries the first bad pixel's `colour_code` 32..39 (32 black, 33 blue,
     34 green, 36 red, 38 yellow, 39 white/grey, `main_imx8mp.c:191-196`) —
     enough to tell "attachment 3 read black" from "read red" in one hold.
4. The verdict is the case's one check: **PASS = the fail mask is 0**, the
   framebuffer was complete, `glDrawBuffers(4, ...)` set no GL error and no
   kick timed out. `SKIP` means the image did not ask for four (or the
   library refused the knob) — rebuild with `CONFIG_GC_GL_TEST_MRT4=y`.
   `FAIL` reading guide:
   * attachment 3 black where grey is wanted (bits 0-2 fine, bit 3 set): the
     fourth target never reached the PE — the prime suspects are the bank
     words 0x14840/0x14908/0x14928 (section 1.2) and `RT_COUNT(3)`
     (`PS_CONTROL` bits 8-10);
   * attachment 3 red after a draw: `PS_OUTPUT_REG` byte 3 names output 0 —
     a shader-row lookup miss fell back to RT0's register
     (`shader.rs:316-318`), which for a *draw* (as opposed to a clear) is a
     bug, not a fallback;
   * parts 1-2 fine, part 3 or 4 wrong on attachments 0/1/3: the null-slot
     compaction at four slots (`hw_targets`), and `PS_OUTPUT_REG` byte 2 in
     the hole stages;
   * everything wrong in part 1 only: the clear quad does not reach
     `RT_COUNT(3)` — case 105-territory, the clear programs RT0's mask/
     overwrite from `clear_components` (`backend.rs:482-492`).
5. Record: the row replaces `tools/cts/RESULTS.md`'s MRT line and the
   `tools/cts/STATUS.md` summary (both files are #317's), comment the result
   with its commit on the board issue #2 and on the new mrt4-board-check
   issue, and hand the board's RC14 dEQP groups
   (`implementation_limits.max_draw_buffers` / `max_color_attachments`,
   `draw_buffers_indexed`, `fragment_out`: `docs/es3-cts-failures.md:274-286`)
   to #316's next run.

## 5. (e) What must hold before DRAW_TARGETS goes to 4 on gpu-vivante

1. **A board drew four.** Case 222 PASS on the i.MX8MP, recorded (the
   RESULTS.md/STATUS.md rows and the issue comments above); the X2 run is a
   second data point of the same part, not a prerequisite. The flip is the
   `gpu-vivante` arm of `DRAW_TARGETS` alone (`src/backend.rs:794-797`); the
   V3D and PowerVR arms keep 3 until their own halves land
   (`src/backend.rs:798-805`).
2. **The board's RC14 dEQP groups come out better, not worse** —
   implementation_limits and draw_buffers_indexed should turn from
   NotSupported/low-limit to running; the host probe (gc-mrt4-deqp-probe)
   lists what to expect. `fragment_out` may still fail its RC1 parts.
3. **The host gate, green against the current main tip**
   (the pull request's CI gate, which for this change is the local set — the const sits
   behind a path that reaches everything): `tests/es3_mrt_gl.rs` already
   proves the GL core at four, `tests/es3_mrt4_gc.rs` (gc-mrt4-host) the
   vivante words through the GL API, `backend_tests.rs:3715` keeps its four
   pins. No table struct gains a field, so the `YakoglBackend {` literal sweep
   of CLAUDE.md has nothing to recheck; nothing generated moves.
4. **The knob is decided**: keep `gc_debug_draw_targets` (its test then stays
   in `KNOBS_TESTS`, its names in `KNOB_VIVANTE_SYMBOLS`) or retire it in the
   flip's commit; either way the "no test knob in the artefact" row is green.
   Case 222 keeps its own `maxdb < 4` skip, so it runs against a product
   library as a SKIP and against the flipped one for real.
5. **Known MRT gaps are recorded as themselves, not silently inherited**:
   the per-RT blend-colour bank (§1.2's gap), a 64-bit integer target drawn
   only alone (`backend.rs` draw refusals, the `GC_INT_RT_64BPP` rule around
   `:2803-2831`) so the 128-bit MRT items (`gc-rt128-planes`,
   `gc-128bit-rt-shader.md`) keep their own paths, and the smaller-later-
   attachment refusal (`:2833-2843`). None blocks the flip; each has its own
   issue.
6. **Docs move in the same change as the flip**: RC14 in
   `docs/es3-cts-failures.md` shrinks to the two GPUs still at 3, and
   `docs/es3/backend-caps.md` records `GL_MAX_DRAW_BUFFERS` 4 for the
   GC7000UL — both files are held by other plans right now, so the flip
   waits for them to settle.

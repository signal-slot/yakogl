<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 float colour targets: the decision, its options and the rows each moves

Plan #343, item `an-float2`, written 2026-11-10 at `b385c667` (origin/main
`52662e6a` adds only a tools/v3d change). Read-only: no dEQP run for this
note, no code change. Facts from code carry `file:line` of that commit.
Facts from board runs say so. What came from a measurement of another item
(`deriv-host-v3d`) is marked with the item.

**Summary.**

1. The 15 `shaders.derivate` fails on an "incomplete framebuffer" are the
   `texture.msaa4` cases: a 4-sample RGBA8 renderbuffer. They are multisample
   (RC8, plan #339), not RGBA32F/RGBA16F. Claiming
   `GL_EXT_color_buffer_float` moves no derivate row. The premise in
   `docs/es3-cts-failures.md:81` and `:120` is wrong.
2. The ES 3.0 build **already claims `GL_EXT_color_buffer_half_float`**
   (`src/gl/state.rs:111`, `:251`) and already answers NotSupported for the
   32-bit rows (183 "Format not supported" on the AM62P). It is not
   inconsistent in the way the plan assumed. Its one real defect is **R32F**:
   it is complete as a colour target without the extension
   (`src/gl/framebuffer.rs:646`), and dEQP
   `fbo.completeness.renderable.{renderbuffer,texture}.color0.r32f` fail
   "Framebuffer checked as complete, expected incomplete" on the AM62P
   (`docs/es3/am62p-gles3-notpass.tsv`) and the Pi 4 (image FB1).
3. Chosen: option **A+B**, i.e. keep not claiming `GL_EXT_color_buffer_float`,
   keep claiming the half-float extension, and make R32F follow the same rule
   as RG32F and RGBA32F (section 6). No back end work.

## 1. The derivate rows, by sub-group and real class

dEQP source: `es3fShaderDerivateTests.cpp` at the pinned `93bca01` was **not
checked out** (no VK-GL-CTS under the deps cache; `tools/cts/README.md`
pins it). It was read through a web fetch summary, without line numbers, so
the statements below on the source are second-hand; the case logs confirm
them. The log lines give `es3fShaderDerivateTests.cpp` line 757 for the incomplete
check.

What the source does (second-hand):

- `fbo_msaa2` / `fbo_msaa4` / `texture.msaa4`: the target is a **GL_RGBA8**
  renderbuffer made with `glRenderbufferStorageMultisample(4, GL_RGBA8)`,
  resolved by a blit into a one-sample renderbuffer.
- `fbo_float`, `float_fastest`, `float_nicest`: the target is **GL_RGBA32UI**
  (the source says floating-point targets are not core), the shader packs the
  float with `floatBitsToUint`.
- `GL_RGBA32F` (highp) and `GL_RGBA16F` (mediump, lowp) appear only as the
  **sampled** texture of the `texture.*` cases, never as a render target.

So no derivate case renders into an RGBA32F or RGBA16F framebuffer.

`tools/cts/gles3-mustpass.txt` holds 282 `shaders.derivate` names. Count:
`grep -c shaders.derivate tools/cts/gles3-mustpass.txt`.

| GPU, source | sub-group | rows | class |
|---|---|---|---|
| AM62P, `docs/es3/am62p-gles3-notpass.tsv` | `texture.msaa4` (dfdx, dfdy, fwidth x float/vec2 highp+mediump, vec4 mediump) | 15 | **multisample**: `checkFramebufferStatus` is not complete (`:757`) |
| AM62P | `texture.basic`, `texture.float_fastest`, `texture.float_nicest` (dfdx 2, dfdy 4, fwidth 2) | 8 | image compare, message "Fail" |
| Pi 4, `tools/cts/log/2026-11-pi4-gles3-b/S10.console.txt` (the TSV has no derivate row: image S10 is not one of its 21) | `texture.msaa4` | 15 | multisample, same `:757` message |
| Pi 4, S10 | `texture.basic` 15, `texture.float_fastest` 20, `texture.float_nicest` 20 | 55 | image compare, message "Fail" |
| host GPU=v3d (item `deriv-host-v3d`, tip `f62bcfe1`, HOST-ONLY) | whole group | 167 Fail of 282 | 15 incomplete FBO (all 4-sample RGBA8), 0 GL error, 152 image compare (read-back is black, so only compile and link outcomes count) |

The Pi 4 count is mustpass names that failed in S10: 70 = 15 + 20 + 20 + 15.
The AM62P count is 23 = 15 + 8. Every failing derivate row is in
`texture.*`. The `fbo_float` cases are not among them on either board and
on the host V3D they fail at the compare, not on a framebuffer.
`GL_RGBA32UI` (`fbo_float`, 88 names per item `float-rows`) is the integer
render target work, see the last part of this section.

### Why the msaa4 target is incomplete on main

This is **not** a #339 defect and not a float problem. It is the documented
capability-off state, because #339's code part has not landed:

- `GL_MAX_SAMPLES` is the literal 1 (`src/gl/state.rs:1286`).
- `glRenderbufferStorageMultisample` answers `samples > 1` with
  `GL_INVALID_OPERATION` before it stores anything
  (`src/gl/framebuffer.rs:3484`). The Pi 4 log shows it: "GL error 0x502 in
  glRenderbufferStorageMultisample" for the `fbo_msaa4` cases of S10.
- The renderbuffer then has no storage, so `attachment_storage` returns
  `None` for it and `check_status` answers `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT`
  (`src/gl/framebuffer.rs:772-778`). dEQP does not look at `GL_MAX_SAMPLES`
  for these cases, so it fails at `:757` instead of NotSupported. (The
  `fbo.msaa` cases do look, and are NotSupported: "Maximum sample count ...
  is only 1", 32 rows on the AM62P.)
- What main has of #339 is the design (`docs/notes/es3-msaa-renderbuffers.md`),
  the pure rules (`src/gl/msaa.rs`, "not wired yet") and the `es3_msaa_*`
  pins. `YakoglBackend` has no `max_samples` field and `Cargo.toml` has no
  `msaa-fbo` feature at this commit (`grep max_samples src/backend.rs`: none).

How c4 ("no `shaders.derivate` case log contains a framebuffer-incomplete
failure on the host for any GPU") is met: **only by #339's items**
`gl-msaa-cap` and each `*-msaa-fbo`, run with `msaa-fbo` in
`host-deqp-run.sh` (`tools/cts/README.md` 2b, `6bc6913b`). Float work
cannot meet it, and no float option changes a derivate row. One more thing
the plan should know: the msaa note's row table (section d) does not list
`shaders.derivate.*.texture.msaa4` (grep: no `derivate` in that note), so
the RC8 row draft undercounts by 15 rows per GPU. Text of an issue for #339 is
in the report of this item.

### RGBA32UI and GL_INVALID_OPERATION on the host

`deriv-rgba32ui` traced it; re-traced here. It is a draw or clear refused by
name, after the framebuffer is complete:

- `refuse_undrawable_targets` returns `GL_INVALID_OPERATION` for a target
  whose surface `backend::rt_format_drawable` says the back end cannot render
  (`src/gl/draw.rs:2442`, called at `:2775` and `:3697`; the predicate is
  `src/backend.rs:559-580`).
- The integer surfaces a back end renders are `INTEGER_RT_FORMATS`
  (`src/backend.rs:513-530`). PowerVR and V3D have all 19, so RGBA32UI draws.
  The GC7000UL has 17: RGBA32I and RGBA32UI are `GC_INT_RT_128BPP`
  (`src/vivante/backend.rs:359`, "a hardware limit"). The Mali has 0
  (`src/mali/backend.rs:102`).
- So the host answer comes from a GPU=vivante (or mali) build, and it is
  the integer-target gap of `docs/es3/int-rt-gc.md` and
  `docs/notes/gc-128bit-render-targets.md`, owned by #335/#333. Class: back
  end limit, integer. Not float, not multisample. Not decided by this note.

## 2. The dEQP rows that need a float colour target

dEQP-GLES3 decides NotSupported from the extension string: with
`GL_EXT_color_buffer_float` absent, the 32-bit and `r11f_g11f_b10f` rows are
NotSupported, and the 16-bit rows run when `GL_EXT_color_buffer_half_float`
is listed. The AM62P run shows it (`docs/es3/am62p-gles3-notpass.tsv`): 183
"Format not supported", 30 "Format not supported, requires
GL_EXT_color_buffer_float", 6 `pbo.renderbuffer` "Renderbuffer format not
supported", and `rg16f`/`rgb16f`/`r16f` rows that ran and failed on content
(for example `fbo.color.blend.rg16f_src_over` "Image comparison failed").
The dEQP side (the format-to-extension map of `es3fFboColorbufferTests`,
`es3fFragmentOutputTests`) was read second-hand (item `float-rows`), so only
the AM62P rows are measured.

Recount (non-derivate names of `tools/cts/gles3-mustpass.txt` that carry a
float token in the case name and sit in a group that renders):

```
grep -E 'functional\.(fbo|fragment_out|pbo\.renderbuffer)' tools/cts/gles3-mustpass.txt \
  | grep -E '(rgba32f|rg32f|r32f|rgb32f|rgba16f|rgb16f|rg16f|r16f|r11f_g11f_b10f)'
```

628 names, of which 250 carry a 32-bit token and 378 only a 16-bit or
`r11f_g11f_b10f` token. This is an upper bound: `fbo.blit.conversion` names
carry both formats, and a name that mentions a float format may not render
into it (depth32f, msaa). The `~530` of the plan is a narrower count of
the same set. By sub-group (names with a token):

| sub-group | names | sub-group | names |
|---|---|---|---|
| `fbo.blit` | 228 | `fbo.completeness` | 58 |
| `fbo.render` | 102 | `fbo.msaa` | 27 |
| `fragment_out.basic` | 84 | `pbo.renderbuffer` | 10 |
| `fragment_out.array` | 84 | `fbo.invalidate` | 9 |
| `fbo.color` | 69 | `fbo.depth` / `fbo.stencil` | 6 / 2 |

Measured on the AM62P, the NotSupported rows in those groups: `fbo.blit` 134,
`fragment_out.basic` 48, `fragment_out.array` 48, `fbo.color` 33,
`fbo.render` 30, `fragment_out.random` 19, `fbo.msaa` 12, `pbo.renderbuffer`
6, `fbo.invalidate` 4: about 334. Those are the rows that become real tests if
`GL_EXT_color_buffer_float` is claimed (options C, D). Texture-only groups
(`texture.format.sized` 72, `texture.specification.*`, `texture.filtering.*`)
mention float tokens but need no float target, and no option moves them.

## 3. The hardware

What each GPU can render, Mesa 26.2.1 (`third_party/mesa-26.2.1` holds only a
table; the tree is `mesa-pan/mesa-26.2.1` in the deps cache) against ours.

| GPU | Mesa: renders | ours today | to render RGBA32F / RG32F |
|---|---|---|---|
| PowerVR BXS-4-64 (AM62P) | all of R16F, RG16F, RGBA16F, R32F, RG32F, RGBA32F: `src/imagination/vulkan/pvr_arch_formats.c` `pvr_pbe_format_table` (line 195), rows at lines 245 (RGBA16F), 248 (R32F), 251 (RG32F), 257 (RGBA32F). Blendable per item `float-caps-pvr` (not re-read here) | RGBA16F (with RGB16F, RG16F as it), R16F, R32F surfaces (`src/pvr/backend.rs:1654`, `:3649`). No RG32F, RGBA32F surface; an RGBA32F texture is stored as halves (`TEXF_RGBA32F` is sampled for shadow compare only, `src/pvr/backend.rs:1240`) | medium-large: new 32-bit RG and RGBA surfaces, four-dword outputs, 32-bit texture storage; `src/pvr/**`, `src/shaderc/online/powervr/**`, `src/backend.rs`, `src/gl/framebuffer.rs`. First verify the R32F PBE source format (we say F16 per channel, Mesa F32, `pvr_arch_formats.c` line 248) |
| V3D 4.2 (Pi 4) | R16F, R32F, RG16F, RG32F, RGBA16F, RGBA32F: `src/gallium/drivers/v3d/v3dx_format_table.c` lines 90-101; RGBA32F uses the 128-bpp internal format (`V3D_INTERNAL_BPP_128`); vertex-format list `v3d_screen.c` lines 432-447 | rows RGBA16F, R16F, R32F only (`src/v3d/state.rs:469-471`, `rt_format`); no RG32F, RGBA32F row though the file already names the 128-bpp class (`B128`, `:437`) | small-medium: a 128-bpp tile buffer row, 32-bit TLB writes in the compiler; `src/v3d/**` (plan #330) |
| GC7000UL (i.MX8MP) | RGBA32F as the emulated `G32R32F` two-plane format: `src/gallium/drivers/etnaviv/etnaviv_format.c` line 233 | R32F (`GC_PE_FORMAT_R32F`) and RGBA16F (`src/vivante/backend.rs:319-321`); RG32F and RGBA32F not rendered | the full two-plane path of `docs/notes/gc-128bit-render-targets.md`, board first; `src/vivante/**` (#328, #333, #335), `src/gl/framebuffer.rs`, `src/gl/draw.rs` (#335, #339) |

The V3D line numbers of Mesa are for the table rows, read here; the
two-plane GC path and the PowerVR costs are the estimates of items
`float-caps-gc` and `float-caps-pvr`.

GL core today (all in `src/gl/framebuffer.rs`):

- `is_color_format` (`:622`) lists the colour attachment formats. It has
  RGBA16F, RGB16F, RG16F, R16F **and R32F** (`:646`). It has no RG32F, RGBA32F,
  RGB32F or R11F_G11F_B10F.
- A texture attachment of those four gets `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT`
  (`check_status`, `:772-778`: `is_color_format` false). ES 3.0 4.4.4.1 asks
  exactly that for a non-colour-renderable image; `UNSUPPORTED` would be wrong.
  RGBA32F is stored as halves and reported as RGBA32F (`:546-551`,
  `GlTexture::rgba32f`, `src/gl/context.rs:532`).
- A renderbuffer of them is `GL_INVALID_ENUM` at storage
  (`renderbuffer_storage_args_ok`, `:2622`; `is_renderable_format` false).
- The extension string: both builds list `GL_EXT_color_buffer_half_float`
  (`src/gl/state.rs:111`, in the base list `:251`), neither lists
  `GL_EXT_color_buffer_float` (`src/gl/consts.rs:1175` defines only the
  constant). The ES 2.0 report further gates the 16F renderbuffers on the
  extension (`:2607-2608`).
- Mismatch: R32F needs `GL_EXT_color_buffer_float` in ES 3.0 (table 3.13: R32F,
  RG32F, RGBA32F, R11F_G11F_B10F need the extension, the 16F ones need it or
  the half-float one) yet it is complete. Pinned as complete by
  `src/gl/framebuffer.rs:6976` and `:7195` and by several
  `tests/es3_*_v3d.rs`.

## 4. Options

The row groups are those of section 2.

**A. Claim nothing new; refuse every float target the build cannot render
consistently.** The status is `INCOMPLETE_ATTACHMENT` (texture) and
`INVALID_ENUM` (renderbuffer), as ES 3.0 says, not `UNSUPPORTED`. Needs only
R32F moved into the refused set.
- Moves: `fbo.completeness.renderable.{renderbuffer,texture}.color0.r32f`
  Fail to Pass (2 rows per GPU, measured as Fail on the AM62P and the Pi 4).
  The 32-bit and r11f rows stay NotSupported (about 334 on the AM62P). No
  new Fail expected: dEQP gates those rows on the extension. One risk, not
  measured: a non-float group that uses an R32F colour target without the
  extension (`fragment_ops`, `transform_feedback`, `shaders`): the host
  rerun of item `rerun-*` finds it.
- Files: `src/gl/framebuffer.rs` (`is_color_format`,
  `is_renderbuffer_color_format`, `attachment_storage` R32F case), tests.
  Plan owners: #335 and #339 hold `framebuffer.rs`; #330 holds
  `tests/es3_*_v3d.rs` that pin R32F as renderable.

**B. Claim `GL_EXT_color_buffer_half_float` only.** That is today's state
(`src/gl/state.rs:111`). Moves nothing now; keeping it is why the 16F rows
(`rgba16f`, `rg16f`, `r16f`, `rgb16f`, 378 names with no 32-bit token, upper
bound) run. Files: none. Risk: those rows are real tests; their fails
(`fbo.color.tex3d.*16f`, `texcube.*16f`, `blend.rg16f_src_over`, ...) are
back-end work of #325, #330 and #335, not of this plan.

**C. Claim `GL_EXT_color_buffer_float` per back end, after the 32F paths exist.**
- Moves: about 334 NotSupported rows (AM62P count) to real tests, each Pass
  or new Fail. Fail until the back end renders: PowerVR and V3D RG32F,
  RGBA32F, GC7000UL RGBA32F, RG32F, plus R11F_G11F_B10F everywhere
  (nothing renders it; Mesa has the V3D and PowerVR formats, ours does not).
  Also `fbo.msaa` float rows, which wait for #339.
- Files: `src/gl/state.rs` (extension), `src/backend.rs`/`backend.h`/
  `abi_layout*` (a capability field, which every `YakoglBackend {` literal
  of `tests/` and `src/` needs), `src/gl/framebuffer.rs`, per back end
  `src/pvr/**`, `src/shaderc/online/powervr/**`, `src/v3d/**`,
  `src/vivante/**`. Owners: #325, #330, #333/#328/#335, #339. Not before
  they are done, and board-first on the GC7000UL.

**D. Per-GPU mix.** Claim the extension on a GPU only when that GPU has all
of R32F, RG32F, RGBA32F, R11F_G11F_B10F as targets (a capability, as C), and
keep A on the others. Moves: C's rows on that GPU alone. Files: C's.
In practice no GPU is there yet; D is C's end state per GPU.

## 5. Choice for item fix-float

**A+B**, the smallest change that makes the build consistent: the half-float
extension stays claimed (B, no change), the 32-bit float targets stay refused,
and R32F joins them (A). Reasons:

- It is the only option with a measured row gain (2 rows per GPU) and no
  hardware dependency; C and D need 32F paths that exist on no back end
  and live in files other plans hold.
- It meets "NotSupported or a working target, never an incomplete FBO" for
  dEQP: dEQP sees NotSupported for every 32-bit row because the extension is
  absent, and `INCOMPLETE_ATTACHMENT` only where it asks for it
  (`completeness.renderable`).
- The route to C stays open: one predicate (below) reads the extension string,
  so a back end that learns 32F later adds the name to the list and the
  predicate changes nothing else.

**Contract of fix-float** (it waits for #335 and #339 on `framebuffer.rs`):

- `src/gl/framebuffer.rs`: one predicate `float32_color_renderable(f)` true for
  R32F, RG32F, RGBA32F, RGB32F, R11F_G11F_B10F only when
  `state::extension_listed("GL_EXT_color_buffer_float")`; the 16F family is
  unchanged. `is_color_format` and `is_renderbuffer_color_format` use it
  (R32F leaves the lists at `:646` and its twin), `attachment_storage` keeps
  RGBA32F and R32F mapped to their sized names (`:550-553`) so the status is
  `INCOMPLETE_ATTACHMENT`, never a silent RGBA16F.
- No `YakoglBackend` field, no change to `src/backend.rs`, `state.rs` or any
  back end. (Should a field become necessary, grep `tests/` and `src/` for
  `YakoglBackend {` and check against the latest main.)
- Texture side: `R32F` textures stay creatable and sampled; only the colour
  attachment changes. Verify that no depth path attaches an R32F colour
  texture (depth-luminance storage, `docs/notes/v3d-depth-storage.md`).
- Tests: new `tests/es3_float_target.rs` (host, ES 3.0 build, no GPU gate
  needed because the rule is GL core): (1) the extension string lacks
  `GL_EXT_color_buffer_float` and has `GL_EXT_color_buffer_half_float`;
  (2) for R32F, RG32F, RGBA32F, RGB32F, R11F_G11F_B10F: texture attachment is
  `INCOMPLETE_ATTACHMENT`, renderbuffer storage is `INVALID_ENUM`; (3) RGBA16F,
  RGB16F, RG16F, R16F: complete, both kinds; (4) R32F texture still
  sampleable and `glReadPixels` of it through a complete one-sample RGBA8
  target is unaffected. Update the pins that said R32F is a complete target
  (`src/gl/framebuffer.rs:6976`, `:7195`, and `tests/es3_rb_formats_v3d.rs`,
  `es3_clearbuffer_formats_v3d.rs`, `es3_srgb_blit_v3d.rs`,
  `es3_formats_v3d.rs`, `es3_gl_core.rs`: those that render or clear into R32F
  need a host-only switch like `HOST_SRGB_RT` (`src/backend.rs:562`) for the
  extension, which belongs to the item but touches #330's test files).
- Gate: `tools/check.sh --changed origin/main` against the current tip;
  `src/gl` reach means `--local`. No generated file moves.
- Rows: `fbo.completeness.renderable.renderbuffer.color0.r32f` and
  `texture.color0.r32f` Fail to Pass on the host and boards. The rerun item
  must also look for R32F users among non-float rows.

Not in fix-float: the 15 `msaa4` derivate rows (#339), RGBA32UI (#335,
integer targets), any 32F render path (C, D), the `docs/es3-cts-failures.md`
rows `:81` and `:120`, which should read "15 `texture.msaa4`: multisample
(RC8)" and drop the Float targets row except for R32F.

## 6. Host `shaders.derivate` per GPU

Incomplete-FBO cases of a whole-group host run (282 mustpass names), HOST-ONLY.

| GPU | run | incomplete-FBO cases (float / msaa) |
|---|---|---|
| v3d | `f62bcfe1`, vendored V3D reference build, item `deriv-host-v3d`: 115 Pass, 167 Fail, 0 NotSupported, 0 Crash | 0 / 15 |
| powervr | pending (deriv-host-powervr2) | pending |
| vivante | pending (rerun-vivante) | pending |

Expected, to be confirmed by the runs: 0 / 15 on every GPU, because the
cause is `glRenderbufferStorageMultisample` in the GL core (section 1), not
the back end. Board lists agree: AM62P 0 float / 15 msaa, Pi 4 0 / 15.
c4 holds on the host once `msaa-fbo` is on for the GPU.

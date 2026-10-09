<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
-->

# Pi 4: the 27 `fbo.completeness.attachment_combinations.*_tex` cases

Item es2-v3d-fbo-combos-diag, plan #4, stage 1 (dEQP-GLES2 with no failures on
three boards). Host-only; nothing ran on a board. Repository state: integration
tip `1ad4695`. dEQP source: VK-GL-CTS `93bca01861b0` (the checkout in the deps
dir; `.git/HEAD`), the commit the repo's results are measured against
(`tools/cts/RESULTS.md`, the `fbo.completeness` notes).

## Verdict

- **No defect found. Prediction for the next Pi 4 run: all 27 Pass.** Nothing
  is NotSupported, nothing needs a fix.
- Why they were NotSupported at Pi 4 sweep 30: the V3D back end did not list
  `GL_OES_depth_texture` and `GL_OES_packed_depth_stencil`, and dEQP finds a
  stencil-attachable texture format only through that pair. The AM62P and the
  i.MX8MP passed from the commits that put the pair in their ES 2.0 extension
  list (`a3856a06`, `f83a7361`); the V3D was left out then because it could not
  sample or fill a depth texture.
- Since 2026-10-03/04 the V3D can (R32F look-up, 32-bit TMU output, depth read
  back on the CPU; gl_test 169 and 177 passed on the Pi 4) and
  `DEPTH_TEXTURE_EXTENSIONS = REPORT_ES2` (`src/gl/state.rs:195`) claims the
  pair on every GPU. `glCheckFramebufferStatus` is GPU-independent GL core
  code, so once the pair is claimed the V3D answers exactly what the PowerVR
  and the GC7000 answer (host proof in section 4).
- The one thing a host run cannot show: that the real V3D texture allocation
  accepts the two depth texture images (64x64, null data). That is the same
  memory as an RGBA8 texture (`attachment_storage`, `src/gl/framebuffer.rs:366`),
  and gl_test 169 already allocated and rendered to such textures on the Pi 4,
  so the residual risk is low. The board run (row 4 of `pi4-full-tree.txt`,
  image FB1) is what settles it.

## 1. The 27 cases

dEQP names an attachment-combination case `<color0>_<colorN>_<depth>_<stencil>`
(`glsFboCompletenessTests.cpp:769-773`), each part one of `none`, `rbo`, `tex`
(`:923-933`, a four-deep loop: 81 cases). The 27 are the ones whose stencil part
is `tex`, so the document's `<a>_<b>_<c>_tex` is color0, colorN, depth.

```
none_none_none_tex   none_none_rbo_tex   none_none_tex_tex
none_rbo_none_tex    none_rbo_rbo_tex    none_rbo_tex_tex
none_tex_none_tex    none_tex_rbo_tex    none_tex_tex_tex
rbo_none_none_tex    rbo_none_rbo_tex    rbo_none_tex_tex
rbo_rbo_none_tex     rbo_rbo_rbo_tex     rbo_rbo_tex_tex
rbo_tex_none_tex     rbo_tex_rbo_tex     rbo_tex_tex_tex
tex_none_none_tex    tex_none_rbo_tex    tex_none_tex_tex
tex_rbo_none_tex     tex_rbo_rbo_tex     tex_rbo_tex_tex
tex_tex_none_tex     tex_tex_rbo_tex     tex_tex_tex_tex
```

Each is `dEQP-GLES2.functional.fbo.completeness.attachment_combinations.<name>`.
The other 54 of the 81 pass on the Pi 4 already (sweep 30: `fbo.api` +
`fbo.completeness` 365 pass / 27 not supported; `tools/cts/RESULTS.md`
"Pi 4: sweep 30"), colorN cases included, because `GL_NV_fbo_color_attachments`
is claimed (see 2). So the 27 are exactly the stencil-texture cases and
nothing else is open in this group.

## 2. What dEQP requires, and what it expects (all lines in VK-GL-CTS `93bca01861b0`, `modules/`)

How a case is built (`glshared/glsFboCompletenessTests.cpp`):

1. `AttachmentTest::build` (`:812-832`) attaches colour 0 first
   (`attachTargetToNew`, `:814`), then, if colorN is not `none`, throws
   NotSupported "Multiple attachments not supported" unless
   `m_ctx.haveMultiColorAtts()` (`:816-818`) and attaches colourN to attachment
   points 1 .. `GL_MAX_COLOR_ATTACHMENTS`-1 (`:820-826`). ES 2.0 sets that flag
   from `GL_NV_fbo_color_attachments`
   (`gles2/functional/es2fFboCompletenessTests.cpp:354`). yakogl lists it
   (`src/gl/state.rs:122`, `:237`) on all three GPUs.
2. `makeDepthAndStencil` (`:788-810`) runs last. Stencil is `tex` in all 27.
   When depth is also `tex`, or the kinds are the same, it first looks for ONE
   core format that is depth- and stencil-renderable and valid for that kind
   (`:790-805`, `getCoreFormats()`); ES 2.0 has none (its core table,
   `es2fFboCompletenessTests.cpp:41-76`, has only `DEPTH_COMPONENT16` and
   `STENCIL_INDEX8` as renderbuffer formats), so it falls through to separate
   images (`:808-809`): the stencil image, then the depth image.
3. **What makes a case NotSupported.** `getDefaultFormat` (`:601-620`) takes the
   first core format for (attachment point, buffer kind), else the first
   *context* format from the extensions (`:614`), else throws
   `NotSupportedError("Unsupported attachment kind for attachment point")`
   (`:618`). For `GL_STENCIL_ATTACHMENT` with a texture there is no core
   format, and the only context format is the entry
   `{"GL_OES_packed_depth_stencil GL_OES_depth_texture", DEPTH_RENDERABLE |
   STENCIL_RENDERABLE | TEXTURE_VALID, {GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8}}`
   (`:207-209`; format list `:76-78`). `addExtFormats`
   (`glshared/glsFboUtil.cpp:233-`) keeps an entry only if every extension
   token is in the context's extension string. So the case is NotSupported
   exactly when `GL_EXTENSIONS` lacks `GL_OES_packed_depth_stencil` or
   `GL_OES_depth_texture`; it is the message the Pi 4 sweep 30 recorded
   ("Unsupported attachment kind for attachment point",
   `tools/cts/RESULTS.md`, sweep 28 note on `:618`). (`GL_ANGLE_depth_texture`
   entry `:217-220` is the alternative; yakogl does not claim it.)
4. The depth image, when depth is `tex`: the first context format flagged
   DEPTH_RENDERABLE|TEXTURE_VALID, `{GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT}` from
   `GL_OES_depth_texture` (`:65-69`, `:198-201`; also REQUIRED_RENDERABLE). When
   depth is `rbo`: `GL_DEPTH_COMPONENT16` (`es2fFboCompletenessTests.cpp:
   62-64`, required renderable). Colour 0 / colourN: `RGBA4` renderbuffer or the
   first unsized colour-renderable texture format (`:43-57`; texture formats are
   renderable but not required).
5. All images are 64x64 (`:814`, `:800`, `:808-809`), so no
   `INCOMPLETE_DIMENSIONS`.

What it expects once it runs (`TestBase::iterate`, `:535-590`;
`glshared/glsFboUtil.cpp:561-631`, `:459-`):

- The builder records the first GL error from the object creation and attach
  calls (`:537`, `FboBuilder::checkError`). Any error fails the case unless the
  reference allows it (`:546-558`); the reference allows none for a legal
  format (`glsFboUtil.cpp:561-600`, the texture is TEXTURE_VALID). So
  `glTexImage2D(GL_DEPTH_STENCIL, ..., GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8,
  NULL)` and the `GL_DEPTH_COMPONENT`/`GL_UNSIGNED_SHORT` one must not error.
- `glCheckFramebufferStatus` (`:560`). The reference status set: COMPLETE is
  allowed unless a mandatory violation exists; the stencil texture format and
  any texture colour format are *not* REQUIRED_RENDERABLE, so
  `INCOMPLETE_ATTACHMENT` is an allowed alternative (`glsFboUtil.cpp:459-`,
  "Attachment format is not required renderable"), and the ES 2.0 checker
  always allows `GL_FRAMEBUFFER_UNSUPPORTED` (`es2fFboCompletenessTests.cpp:
  231`). UNSUPPORTED passes outright (`glsFboCompletenessTests.cpp:576-585`).
  COMPLETE passes. INCOMPLETE_ATTACHMENT passes with a quality warning
  (`:586-587`) when COMPLETE was also allowed. Any other status
  (MISSING_ATTACHMENT, INCOMPLETE_DIMENSIONS) fails (`:566-575`).
- So for these 27 the accepted answers are COMPLETE, UNSUPPORTED, or (with a
  warning) INCOMPLETE_ATTACHMENT. None of them draws.

## 3. What yakogl does in a `report-es2` build

Extension string (`src/gl/state.rs`):

- `GL_NV_fbo_color_attachments` is in the common list (`:122`, string `:237`).
- `GL_OES_texture_npot GL_OES_depth_texture GL_OES_packed_depth_stencil` are the
  ES 2.0-only tail (`:149-180`, `ES2_ONLY_NAMES = 3` `:184`). They are claimed
  when `DEPTH_TEXTURE_EXTENSIONS` (`:195`) = `REPORT_ES2`, on every GPU; the
  const assert at `:197` ties it to `NPOT_EXTENSION`. The V3D has no exception
  (the earlier text "not the V3D" of `tools/cts/RESULTS.md`, "Depth textures on
  the ES 2.0 surface", is superseded by `:186-194` and
  `tools/cts/STATUS.md` 10/3). Unit tests pin it:
  `gl::state::tests::the_depth_texture_pair_is_claimed_where_it_works`
  (`:1422`).

Texture images (`src/gl/texture.rs`): `depth_image_error` (`:1855`, called
at `:1914`; the ES 2.0 upload rules; `state.rs:153` still calls it by an older
name, `es2_depth_image_error`) takes `GL_DEPTH_COMPONENT` with
`GL_UNSIGNED_SHORT`/`GL_UNSIGNED_INT` and `GL_DEPTH_STENCIL` with
`GL_UNSIGNED_INT_24_8`, `GL_TEXTURE_2D` only, base format = internal format; the
texture keeps its RGBA8-sized memory and an R32F look-up. The V3D look-up is
`src/v3d/texture.rs:168-174` (R32F, swizzle XXX1 with `depth_luminance`),
`return_32` at `:183`.

Renderbuffers: `GL_DEPTH_COMPONENT16`, `GL_STENCIL_INDEX8`, `GL_RGBA4`,
`GL_DEPTH24_STENCIL8` are storage formats on both surfaces
(`a_depth24_stencil8_renderbuffer_is_depth_and_stencil`).

Framebuffer (`src/gl/framebuffer.rs`, GPU-independent):

- `attachment_storage` (`:359-`): a texture of base format `GL_DEPTH_COMPONENT`
  or `GL_DEPTH_STENCIL` is an attachment with `internalformat`
  `GL_DEPTH_COMPONENT24` / `GL_DEPTH24_STENCIL8` (`:366-392`).
- `check_status` (`:548-625`): colour 0, then colour 1.. (each must be a colour
  format, same size on the ES 2.0 surface), depth (`is_depth_format`), stencil
  (`is_stencil_format`, which holds `DEPTH24_STENCIL8`, so the packed texture is
  a stencil attachment). No attachment at all: MISSING_ATTACHMENT. No colour 0:
  COMPLETE only if `backend::DEPTH_ONLY_TARGETS` and the framebuffer is
  depth/stencil only (`:618`), else UNSUPPORTED (dEQP accepts that).
- `DEPTH_ONLY_TARGETS` (`src/backend.rs:264`) is `DRAWS_WITHOUT_COLOUR`, and it
  is `true` for all three back ends: V3D `src/v3d/backend.rs:49`, PowerVR
  `src/pvr/backend.rs:90`, GC7000 `src/vivante/backend.rs:614`.

Per-GPU difference, then: none in the logic. The PowerVR and the GC7000 pass
because they claim the pair and the shared core completes the framebuffers as
above. The V3D failed at sweep 30 only because it did not claim the pair, so
dEQP threw NotSupported before any V3D code ran. Backend differences that remain
(PowerVR packed D24S8 ZLS store and GC7000 4x4 tiled depth, V3D
CPU read-back of D24S8) matter for *sampling* depth, not for this test.

## 4. Host runs (V3D, PowerVR, GC7000)

Existing coverage that already is the dEQP sequence for the main shapes:
`gl::framebuffer::tests::depth_and_depth_stencil_textures_complete_the_framebuffer_as_the_extensions_say`
(`src/gl/framebuffer.rs:6858`, builds the stencil texture first and the depth
image second as dEQP does, seven shapes), `a_depth_stencil_texture_on_the_stencil_point_is_queried_as_a_texture`,
`a_depth24_stencil8_renderbuffer_is_depth_and_stencil`,
`a_depth_texture_is_not_a_colour_attachment`.

```
cargo test --no-default-features \
  --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --lib -- \
  depth_and_depth_stencil_textures a_depth_stencil_texture_on a_depth24_stencil8 \
  a_depth_texture_is_not the_depth_texture_pair
```

```
test gl::state::tests::the_depth_texture_pair_is_claimed_where_it_works ... ok
test gl::framebuffer::tests::a_depth24_stencil8_renderbuffer_is_depth_and_stencil ... ok
test gl::framebuffer::tests::a_depth_stencil_texture_on_the_stencil_point_is_queried_as_a_texture ... ok
test gl::framebuffer::tests::a_depth_texture_is_not_a_colour_attachment ... ok
test gl::framebuffer::tests::depth_and_depth_stencil_textures_complete_the_framebuffer_as_the_extensions_say ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 2580 filtered out
```

All 81 cases, scratch test. I added a throwaway test to `src/gl/framebuffer.rs`'s
test module that builds each `<c0>_<cN>_<d>_<s>` framebuffer as dEQP does (64x64,
a fresh `TestGl` per case, colour renderbuffer `GL_RGBA4`, colour texture
`GL_RGB`/`GL_UNSIGNED_SHORT_5_6_5`, depth rbo `GL_DEPTH_COMPONENT16`, depth
texture `GL_DEPTH_COMPONENT`/`GL_UNSIGNED_SHORT`, stencil rbo `GL_STENCIL_INDEX8`,
stencil texture `GL_DEPTH_STENCIL`/`GL_UNSIGNED_INT_24_8`; colour N on points
1..`CAP_COLOR_ATTACHMENTS`-1; stencil attached before depth), records the first
GL error and the status, and applies dEQP's accept rule from section 2. The
test was run and then removed; the committed tree does not contain it (no file
but this document is committed). Commands (the three rows of
`tools/check.sh`'s configurations, `report-es2` added where the row has none):

```
cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --lib -- --nocapture scratch_combos
cargo test --no-default-features --features host-test,gpu-powervr,report-es2 --lib -- --nocapture scratch_combos
cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --lib -- --nocapture scratch_combos
```

Output, identical on all three (diffed line by line, the three outputs are
equal). The first eight characters of a line are the case, the rest what
glCheckFramebufferStatus returned; GL error was `0x0` for every case:

```
none_none_none_tex   COMPLETE     none_none_rbo_tex   COMPLETE     none_none_tex_tex   COMPLETE
none_rbo_none_tex    UNSUPPORTED  none_rbo_rbo_tex    UNSUPPORTED  none_rbo_tex_tex    UNSUPPORTED
none_tex_none_tex    UNSUPPORTED  none_tex_rbo_tex    UNSUPPORTED  none_tex_tex_tex    UNSUPPORTED
rbo_none_none_tex    COMPLETE     rbo_none_rbo_tex    COMPLETE     rbo_none_tex_tex    COMPLETE
rbo_rbo_none_tex     COMPLETE     rbo_rbo_rbo_tex     COMPLETE     rbo_rbo_tex_tex     COMPLETE
rbo_tex_none_tex     COMPLETE     rbo_tex_rbo_tex     COMPLETE     rbo_tex_tex_tex     COMPLETE
tex_none_none_tex    COMPLETE     tex_none_rbo_tex    COMPLETE     tex_none_tex_tex    COMPLETE
tex_rbo_none_tex     COMPLETE     tex_rbo_rbo_tex     COMPLETE     tex_rbo_tex_tex     COMPLETE
tex_tex_none_tex     COMPLETE     tex_tex_rbo_tex     COMPLETE     tex_tex_tex_tex     COMPLETE
COMBO total ok 81 of 81
```

(A first run reused one `TestGl` for all 81 cases and showed GL_OUT_OF_MEMORY,
0x505, for the later cases, and a MISSING_ATTACHMENT for `rbo_rbo_rbo_rbo`: the
mock heap filled because the scratch code deleted nothing. A fresh context per
case removed both; recorded so the next person does not repeat it.)

What the host run does and does not prove:

- It runs the real GL core (`glTexImage2D`, `glFramebufferTexture2D`,
  `glFramebufferRenderbuffer`, `glCheckFramebufferStatus`) with each back end's
  extension list and `DEPTH_ONLY_TARGETS`; the GPU is a recording mock. So it
  proves the GL-level behaviour, not the V3D's memory allocation on the board.
- It did not run the colour texture formats `RGBA/UNSIGNED_SHORT_4_4_4_4` and
  `RGBA/UNSIGNED_SHORT_5_5_5_1` (I used 5_6_5): which of the three unsized
  colour formats is "first" in dEQP's ordered set was not determined. That
  does not change the prediction: the 54 non-stencil-texture cases use the same
  colour image and passed on the Pi 4 at sweep 30, and a colour texture dEQP
  cannot render is an allowed INCOMPLETE_ATTACHMENT.
- `tools/check.sh` was not re-run: the committed change is one new document.

## 5. Per-case prediction for the next Pi 4 run (FB1 image, row 4)

Prediction key: **Pass** = dEQP Pass (COMPLETE, or the always-allowed
UNSUPPORTED). No case is NotSupported-acceptable, NotSupported-to-fix or Fail.
Deciding dEQP lines: UNSUPPORTED/COMPLETE accepted at
`glshared/glsFboCompletenessTests.cpp:576-589`; the NotSupported throw that
no longer fires at `:618` (it needs the ext entry `:207-209`); the colour-N gate
at `:818` (open since `GL_NV_fbo_color_attachments`, `es2fFboCompletenessTests.
cpp:354`).

| # | case (`..._tex` = stencil texture) | yakogl status (V3D = PowerVR = GC7000, host) | Pi 4 prediction | deciding line |
|---|---|---|---|---|
| 1 | none_none_none_tex | COMPLETE (stencil-only, depth-only targets) | Pass | `:589` |
| 2 | none_none_rbo_tex | COMPLETE | Pass | `:589` |
| 3 | none_none_tex_tex | COMPLETE | Pass | `:589` |
| 4 | none_rbo_none_tex | UNSUPPORTED (colour only on 1..) | Pass | `:576-585` |
| 5 | none_rbo_rbo_tex | UNSUPPORTED | Pass | `:576-585` |
| 6 | none_rbo_tex_tex | UNSUPPORTED | Pass | `:576-585` |
| 7 | none_tex_none_tex | UNSUPPORTED | Pass | `:576-585` |
| 8 | none_tex_rbo_tex | UNSUPPORTED | Pass | `:576-585` |
| 9 | none_tex_tex_tex | UNSUPPORTED | Pass | `:576-585` |
| 10-27 | rbo_* and tex_*: `{rbo,tex}_{none,rbo,tex}_{none,rbo,tex}_tex` (18 names, the list in 1) | COMPLETE | Pass | `:589` |

For row 1-3, if a back end ever answered UNSUPPORTED instead, still Pass; if
INCOMPLETE_ATTACHMENT it would pass with a quality warning (`:586-587`), and the
tally would show it (not predicted). The expected FB1 tally follows: all 392
`fbo.api` + `fbo.completeness` cases Pass, i.e. the 27 NotSupported of sweep
30 move to Pass (RESULTS.md sweep 30 listed 365 pass / 27 NotSupported for the
group), nothing else moves.

How to read a surprise on the board:

| result on the Pi 4 | what it means | where to look |
|---|---|---|
| still NotSupported "Unsupported attachment kind for attachment point" | the image was built without the claim: not `report-es2` (`driver/samples/zephyr/v3d_deqp/prj.conf:9` sets it), or a stale integration build | `GL_EXTENSIONS` in the log; `the_extension_string_and_the_name_list_agree` |
| Fail "Got wrong error code" / "Expected no error" | a texture image call or attach raised a GL error on the real V3D path (allocation) | the error code in the case log; `src/v3d/texture.rs`, texture level allocation |
| Fail "Framebuffer checked as incomplete, expected complete" | `attachment_storage` returned None (the 64x64 texture memory not allocated) | `src/gl/framebuffer.rs:366-392` |
| Fail "Framebuffer checked as complete, expected incomplete" | not possible from this logic: sizes equal, formats legal | – |

## 6. Root cause, and the plan

Root cause (settled by reading): extension claim only. dEQP's only route to a
stencil-attachable texture format is `GL_OES_packed_depth_stencil` together with
`GL_OES_depth_texture` (`:207-209`); the V3D lacked both in the ES 2.0 list until
10/3, so `getDefaultFormat` threw NotSupported at `:618` for every stencil
texture. The shared core had completed these framebuffers correctly all along
(the same framebuffer code passes on the other two boards since the claim).

Defect found: none. No driver fix is needed, so there is no fix plan for
`src/v3d/**` and no item should be added for this. Nothing here touched
`src/v3d/**` or any other file.

Follow-ups for other items (not done here):

1. `es2-docs-r1` / `pi4-es2-final-a`: row 4 (image FB1) is the check. Expected
   tally: `fbo.api` + `fbo.completeness` all Pass; record the 27 as moved from
   NotSupported. Change the `docs/es2-cts-failures.md` section C row from "to
   verify" to "predicted pass; confirmed/refuted by the board" with this
   document as the source.
2. Optional, not a defect fix: commit the scratch loop as a host test (name
   `attachment_combinations_follow_deqp_gles2` in the `tests` module of
   `src/gl/framebuffer.rs`, next to
   `depth_and_depth_stencil_textures_complete_the_framebuffer_as_the_extensions_say`),
   so all 27 (or 81) names are pinned and the next change to the extension list
   cannot silently reopen them. Per-case `TestGl::new(64, 64)` is required. This
   touches `src/gl/framebuffer.rs`, which no running item owns (checked against
   the plan item list), but belongs to a separate item because this one is
   documentation-only.
3. Inconsistency left for `es2-docs-r1`: `tools/cts/RESULTS.md` "Depth
   textures on the ES 2.0 surface" says the pair is claimed "(not the V3D,
   below)"; `src/gl/state.rs:186-194` and STATUS.md 10/3 say the V3D claims it.
   Also its sweep 28 note counts 9 stencil-texture NotSupported (colour N
   cases were gated by `:818` then); the 27 is the count once
   `GL_NV_fbo_color_attachments` is claimed.

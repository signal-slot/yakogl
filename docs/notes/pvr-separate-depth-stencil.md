<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR: separate depth and stencil attachments (NS-DEPTH-STENCIL)

Plan #400, item cause-sepds. This note covers the 12 NotSupported lines of
`docs/es3/am62p-gles3-notpass.tsv` that `tools/cts/am62p-notpass-causes.py`
sorts into the cause `NS-DEPTH-STENCIL`. Every claim below comes from source
read on the host at the integration tip `ad2b6cc9`. dEQP was read at the
pinned tag `vulkan-cts-1.4.6.2` (commit `f6a29701`, a sparse checkout of
`modules/gles3/functional`). Nothing here ran on a board.

**Finding.** The 12 cases are a correct NotSupported. ES 3.0 does not merely
allow `GL_FRAMEBUFFER_UNSUPPORTED` for a depth image and a stencil image that
are different objects: it requires it. dEQP also never asks the GL. It skips
these cases because `GL_EXT_separate_depth_stencil` is not in
`GL_EXTENSIONS`. The PowerVR hardware can keep depth and stencil in two
allocations, and yakogl's PowerVR backend already does this by default for
ES 2.0 contexts. Making the 12 cases run is optional work, and the change is
in the GL core (`src/gl/**`), not in `src/pvr/**`. The cause can be marked
`known`.

## 1. The 12 cases

All 12 are in `dEQP-GLES3.functional.fbo.blit.depth_stencil.` and all carry
the message `Separate depth and stencil buffers not supported`:

| Depth format | Cases |
|---|---|
| `depth_component16_stencil_index8_` | `basic`, `depth_only`, `scale`, `stencil_only` |
| `depth_component24_stencil_index8_` | `basic`, `depth_only`, `scale`, `stencil_only` |
| `depth_component32f_stencil_index8_` | `basic`, `depth_only`, `scale`, `stencil_only` |

These are TSV lines 224-235, rows 0-10, build `b25997e4`. The mustpass list
has 28 `fbo.blit.depth_stencil.*` cases. The 16 that are not listed here use
one image for both depth and stencil (`depth24_stencil8_*`,
`depth32f_stencil8_*`) or only one of the two (`depth_component{16,24,32f}_*`,
`stencil_index8_*`), so they do not depend on this cause. No other ES 3.0
mustpass case attaches separate depth and stencil images that dEQP expects to
complete: `es3fFboCompletenessTests` expects `GL_FRAMEBUFFER_UNSUPPORTED` for
them when the extension is not listed (section 2).

## 2. Question 1: does ES 3.0 allow NotSupported here?

**The spec.** OpenGL ES 3.0 section 4.4.4.2, "Whole Framebuffer
Completeness", lists among the conditions for a complete framebuffer:

> Depth and stencil attachments, if present, are the same image.
> { FRAMEBUFFER_UNSUPPORTED }

This rule is new in ES 3.0. ES 2.0 had no such rule, and its separate
`DEPTH_COMPONENT16` and `STENCIL_INDEX8` renderbuffers were the normal case.
Because the rule is in the core list, it is not an implementation-dependent
restriction. An ES 3.0 implementation must return `FRAMEBUFFER_UNSUPPORTED`
when depth and stencil are separate images. Only an extension can lift the
rule, and `GL_EXT_separate_depth_stencil` does exactly that: a depth image and
a different stencil image may then complete together.

dEQP's completeness test quotes the same sentence and keys its expected status
on the extension (`es3fFboCompletenessTests.cpp:221-225`):

```cpp
else if (m_depthStencilImage != att.imageName || m_depthStencilType != attachmentType(att))
{
    // "Depth and stencil attachments, if present, are the same image."
    if (!m_ctxInfo->isExtensionSupported("GL_EXT_separate_depth_stencil"))
        addFBOStatus(GL_FRAMEBUFFER_UNSUPPORTED, "Depth and stencil attachments are not the same image");
```

**dEQP's handling of the 12 cases.** `BlitDepthStencilCase::preCheck`
(`es3fFramebufferBlitTests.cpp:498-507`) runs before any GL object exists
(`FboTestCase::iterate`, `es3fFboTestCase.cpp:76-78`):

```cpp
void preCheck(void)
{
    if (m_depthFormat != GL_NONE)
        checkFormatSupport(m_depthFormat);
    if (m_stencilFormat != GL_NONE)
        checkFormatSupport(m_stencilFormat);
    if (!m_context.getContextInfo().isExtensionSupported("GL_EXT_separate_depth_stencil") &&
        m_depthFormat != GL_NONE && m_stencilFormat != GL_NONE)
        throw tcu::NotSupportedError("Separate depth and stencil buffers not supported");
}
```

The case list (`:1088-1131`) builds `<depth>_stencil_index8_*` only for the
three depth-only formats ("Do not try separate stencil along with a combined
depth/stencil", `:1110-1113`). `render()` (`:563-574`) puts the two
renderbuffers on `GL_DEPTH_ATTACHMENT` and `GL_STENCIL_ATTACHMENT`, then calls
`checkFramebufferStatus`. That path also ends in NotSupported, because
`FboTestCase::iterate` turns `GL_FRAMEBUFFER_UNSUPPORTED` into
`QP_TEST_RESULT_NOT_SUPPORTED` (`es3fFboTestCase.cpp:94-100`). The TSV's
message, however, is the preCheck's text, so the GL was never asked: the
result depends only on the extension string.

**yakogl's GL core** follows the spec. `check_status`
(`src/gl/framebuffer.rs:969-981`) returns `GL_FRAMEBUFFER_UNSUPPORTED` in ES
3.0 (`!crate::REPORT_ES2`) when `(type, name, level, layer)` of the depth and
the stencil attachment differ. The comment there names the extension as the
thing that would lift the rule. The extension string
(`src/gl/state.rs:276-295`) does not list `GL_EXT_separate_depth_stencil`. The
host test
`depth_and_depth_stencil_textures_complete_the_framebuffer_as_the_extensions_say`
(`src/gl/framebuffer.rs:8382`, the assertion at `:8413-8415`) pins this
behaviour.

**Answer.** Yes. Without `GL_EXT_separate_depth_stencil`, ES 3.0 4.4.4.2
requires these framebuffers to be `FRAMEBUFFER_UNSUPPORTED`, and dEQP reports
NotSupported for them by design. Claiming the extension is optional, so the 12
lines are not a conformance defect. The V3D (Pi 4) has the same 12 lines,
recorded as group G7 in `docs/notes/v3d-fbo-blit.md` ("Allowed by the spec as
NotSupported").

## 3. Question 2: can the PowerVR ISP use depth and stencil from different allocations?

**Yes. The register interface has it, and the backend already uses it by
default.**

- **Registers.** The ISP's Z load/store has two base registers.
  `CR_ISP_ZLOAD_BASE` is the depth base. `CR_ISP_STENCIL_LOAD_BASE` has its
  own address and an `enable` bit, which selects a separate stencil buffer
  (cr.xml `ISP_STENCIL_LOAD_BASE.enable`). `CR_ISP_ZLSCTL` also has
  stencil-only extents (`zlsextent_x_s`, `zlsextent_y_s`,
  `stencil_extent_enable`, `src/pvr/csbgen.rs:296-317`), separate from the
  depth extents `zlsextent_{x,y}_z`.
- **Backend: depth and stencil kept apart (`split_zs_on`).** Each frame,
  `src/pvr/backend.rs:10842-10854` packs `isp_zload_store_base` from the depth
  buffer (`frame_ds`) and `isp_stencil_load_store_base` from `frame_s`. When
  the frame has its own stencil buffer it uses that buffer with `enable =
  true`. Otherwise it uses the depth buffer with `enable = false`, which is
  the packed D24S8 case. `zs_keys` (`:2354-2363`) splits whenever
  `split_zs_on()` is true, the target is not the window, and `depth_id !=
  stencil_id`. In that case it returns a `DS_DEPTH` key for the depth object
  and a `DS_STENCIL` key for the stencil object (`DsKey`, `:1564-1580`).
  `split_zs_on()` (`:2169-2179`) is `true` in every build that lacks the
  `test-knobs` feature. With that feature, the knob `pvr_debug_split_zs`
  defaults to 1. The doc comment at `:2126-2160` records the board measurement
  on the AM62P: dEQP-GLES2 fbo 646/0 with depth and stencil apart against
  632/14 packed, and gl_test 123 `zsapart` passing, at `5ba5cc97` and again at
  `0b17df0d`. The 14 cases that moved were
  `fbo.render.shared_depthbuffer.*_stencil_index8` (7) and
  `recreate_stencilbuffer.no_rebind_*_stencil_index8` (7).
- **Partial render.** `docs/notes/pvr-pr-zs.md` ("The decision") keeps both
  base words unchanged in the 3D_PR command. `pr_zs::pr_zs_words`
  (`src/pvr/pr_zs.rs:38-48`, called at `src/pvr/backend.rs:3966-3973`) carries
  `stencil_load_base` through unchanged. So a split frame that runs a partial
  render stores both buffers and loads both again; nothing there assumes
  packed depth and stencil.
- **Buffer pool.** `docs/notes/pvr-frameset-depth.md` covers the frame-set
  depth, not depth/stencil buffers, and says nothing that rules out a split
  frame. A split frame takes two of the `PVR_BACKEND_MAX_DS` slots
  (`src/pvr/backend.rs:817`) instead of one. This already happens in ES 2.0.

Caveats, all recorded in the comment at `:2126-2160`:

- Each half gets a whole D24S8 buffer. The ISP's separate-stencil layout is
  still unread, so the stencil buffer is sized at 4 bytes a pixel, which is
  more than it needs.
- The depth half is stored as `24BITINT` (`isp_zlsctl_stores`, `:758-782`),
  the same as on the packed path. `depth_component32f` therefore has the same
  precision whether it shares a buffer with stencil or not.
- Mesa's pvr never pairs a depth image with a separate stencil image, because
  Vulkan has a single depth/stencil attachment. The split path therefore has
  no Mesa reference; it was validated only on the board.

## 4. Question 3: the change that would make the 12 cases run

The hardware and the PowerVR backend need nothing new. The work is in the GL
core, and it is optional. `src/gl/**` is held by plan #397, so this is a named
change for a later item and is not made here:

1. **A backend fact in `src/backend.rs`**, next to `DEPTH_ONLY_TARGETS`
   (`:854`): for example `pub const SEPARATE_DEPTH_STENCIL: bool`, taken from
   each backend's module. The PowerVR value is `true` because of `split_zs_on`.
   The V3D also keeps depth and stencil apart (`v3d_split_zs`,
   `docs/notes/v3d-fbo-blit.md` 3.5). The Vivante and Mali values must be
   decided on their own evidence. The rule is in the shared GL core, so a
   global switch would change every backend.
2. **`check_status`** (`src/gl/framebuffer.rs:974-981`): skip the "same
   image" rule when `SEPARATE_DEPTH_STENCIL` is true. The window target stays
   packed, because `zs_keys` never splits the window.
3. **The extension list** (`src/gl/state.rs`, `ALL_EXTENSION_NAMES` /
   `base_extension_string!` / `EXTENSION_STRING`, `:60-295`): list
   `GL_EXT_separate_depth_stencil` where the constant is true, so that the
   string and `extension_listed` agree.
4. **Read the extension's own specification first.** Listing the extension
   commits the GL to everything it specifies, not only to the completeness
   answer. One example is how `glClearBufferfi(GL_DEPTH_STENCIL)` and a
   `GL_DEPTH_STENCIL_ATTACHMENT` query behave with two images. The
   specification was not read for this note.
5. **Blit.** Check that a depth/stencil blit between two framebuffers with
   separate images copies each plane from its own object. `ds_format`
   (`src/gl/blit.rs:392`) already looks up the depth and the stencil
   attachment separately, and the copies are per plane (`DS_PLANE_DEPTH`,
   `DS_PLANE_STENCIL`, `clear_undrawn_dst`, `:1099-1120`). This is read, not
   measured.
6. **Tests.** Change the assertion at `src/gl/framebuffer.rs:8413-8415` so
   that it expects `COMPLETE` where the backend claims the extension. Add a
   host PowerVR test: blit a `DEPTH_COMPONENT24` renderbuffer and a separate
   `STENCIL_INDEX8` renderbuffer into a second FBO with the same layout, read
   the depth and stencil back, and check the values against those drawn. The
   12 dEQP cases are the board check. With the extension listed, dEQP's
   `fbo.completeness` expectations also change (section 2). Those cases must
   be rerun with it.

Expected effect: the 12 lines leave NotSupported and become Pass or Fail. It
would not be the first separate-stencil path on this backend, because ES 2.0
contexts have used it since `5ba5cc97`.

## 5. The cause's status

`NS-DEPTH-STENCIL` is **known**: these are NotSupported cases that the spec
requires, not a defect. Wording for the rule row in
`tools/cts/am62p-notpass-causes.py`, for the causes-index item:

```python
("NS-DEPTH-STENCIL", {NS}, None, r"Separate depth and stencil", "known",
 "pvr-separate-depth-stencil.md",
 "ES 3.0 4.4.4.2 requires FRAMEBUFFER_UNSUPPORTED for separate depth and stencil images; GL_EXT_separate_depth_stencil is optional and not claimed"),
```

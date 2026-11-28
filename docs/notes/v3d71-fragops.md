# The V3D 7.1 fragment-op draw state, as Mesa's v3d driver emits it

Design input for refs71b (reference pins) and compose71d (the composition into
`src/v3d/draw71.rs`, `glue71.rs`, `table71.rs`). It states which binner-list
packets Mesa emits for blend, colour write masks, stencil and depth/Z, which
fields differ between ver 42 and ver 71, what `draw71` emits today and what it
lacks. Written from Mesa 26.2.1 and the tip at c28470db.

## 0. Sources and how they are cited

* `third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml` (the packet XML,
  cited `X:<line>`) and `src/broadcom/common/v3d_limits.h` are vendored.
* The vendored tree does **not** hold the Gallium driver's `v3dx_emit.c`,
  `v3dx_state.c` and `v3dx_draw.c`; only `v3dx_format_table.*` is there. Those
  are cited by their path in the same Mesa 26.2.1 release
  (`src/gallium/drivers/v3d/`, cited `E:`, `S:`, `D:` for `v3dx_emit.c`,
  `v3dx_state.c`, `v3dx_draw.c`), as the farm's full source checkout holds them
  (`mesa-pan/mesa-26.2.1` under the project's dependency cache, the tree
  `MESA_V3D_TREE` / tools/v3d/README.md name). The XML is the same file in both
  trees. A later item that wants the three files in `third_party/` needs the
  owner's decision; this note does not touch `third_party/`.
* `v3dx_emit.c` is compiled once per `V3D_VERSION` (42, 71); the `#if
  V3D_VERSION` lines below are the ver 42/71 differences.
* "refs" are the Mesa 7.1 CLIF dumps in `tools/v3d/pi5/*.clif`.

## 1. The packets Mesa emits

Mesa re-emits a group when its dirty bit is set (`E:` `v3dX(emit_state)`);
`draw71` has no dirty tracking: every draw emits the whole set, in Mesa's
order (`draw71.rs` header, "The order a draw emits them in").

| packet (opcode) | Mesa emit | XML | ver 42 vs 71 | `draw71` |
|---|---|---|---|---|
| `BLEND_ENABLES` (83) | `E:524-571` (mask = `blend->blend_enables`, or 0 when `use_software` or `framebuffer_soft_blend`, `E:565-568`) | `X:773`, `Mask` 8 bits | none (8 bits at both) | emits (`BlendEnables`, `translate`) |
| `BLEND_CFG` (84) | `E:94-124` `emit_rt_blend` (skips a target whose `blend_enable` is off, `E:101-103`); called per target for independent blend `E:531-535`, in two groups by `blend_dst_alpha_one` `E:536-554`, else once with mask `(1<<max_rts)-1` `E:555-562` | 42: `X:777` (`Render Target Mask` 4 bits at 24); 71: `X:787` (8 bits at 24) | mask width only; the other six fields are at the same bits | emits (`BlendCfg`, `translate`'s `to_cfg`): one packet with mask `0xff` without independent blend, one per enabled target with it |
| `BLEND_CONSTANT_COLOR` (86) | `E:587-598`, only when `V3D_DIRTY_BLEND_COLOR`; `swap_color_rb` swaps red and blue (`E:589-596`) | `X:797`, four F16 | none | struct `BlendCcolor` and its spec exist; **not** in `StatePackets71`. The C builder emits it every draw (`driver/src/v3d/v3d_job71.c:581-587`); pkt71 owns the spec |
| `COLOR_WRITE_MASKS` (87) | `E:573-585`; per target `i`: `~colormask & 0xf`, shifted `4*i`, `translate_colormask` `E:83-92`; non-independent blend reads RT 0's mask for all targets | `X:804`, `Mask` 32 bits | none; 71 fills eight nibbles, 42 four (`V3D_MAX_RENDER_TARGETS`, `common/v3d_limits.h:44`) | emits (`ColorWriteMasks`), 8 nibbles, RT 0's for all without independent blend |
| `STENCIL_CFG` (80) front, back | `E:600-619` (front when `stencil[0].enabled`, back when `stencil[1].enabled`; the reference is patched in at emit time from `stencil_ref`); packed once at CSO creation `S:215-253`: front `front_config = true`, `back_config = !back->enabled`; back `front_config = false, back_config = true`; masks, function, the three ops | `X:761` | none | emits (`StencilCfg`, `face_cfg`, `translate`), front then back when two-sided; the reference is clamped to 8 bits |
| `CFG_BITS` (96) | `E:290-386` (dirty on rasterizer, ZSA, prim mode, blend) | 42: `X:831`; 71: `X:849` | see 1.1 | emits (`CfgBits`), with the gaps in 1.1 and section 2 |
| `DEPTH_OFFSET` (106) | `E:408-420` (only when `offset_tri`); packed at CSO creation `S:93-111` (limit, f187 units, f187 factor) | `X:894` | 42 only: the Z16 variant with units x 256 (`S:106-112`, `E:409-413`, `ver == 42` and Z16 depth); 71 has one packet | emits when `polygon_offset` (`DepthOffset`, `f187` truncates to the top 16 bits as Mesa's packer does) |
| `POINT_SIZE` (104) | `E:434-438`, max(point size, .125) from `S:90-91` (HW-2726); 42 emits it every draw as the binner-FIFO flush (`E:421-433`) | `X:886` | ver 42 re-emits per draw; 71 only on rasterizer change | spec and pack exist (`PointSize`); the C builder emits it (`v3d_job71.c:504-507`, 1.0 for ES) |
| `LINE_WIDTH` (105) | `E:440-443`; `v3d_get_real_line_width` (`v3d_context.c:207-220`): smoothing adds `floorf(sqrt2*w)+3` | `X:890` | none | spec and pack exist; C builder `v3d_job71.c:508-510` |
| `CLIPPER_XY_SCALING` (110) | `E:446-462`; 42: half size x 256; 71: x 64 | 42 `X:919`, 71 `X:924` | unit 1/256 vs 1/64 | emits x 64 |
| `CLIPPER_Z_SCALE_AND_OFFSET` (111) / `_NO_GUARDBAND` (112) | `E:464-486`; 71 picks `..._NO_GUARDBAND` when `fabsf(scale[2]) < 0.01` | `X:929`, `X:934` (min_ver 71) | 112 is 7.1-only | 111 emits; 112 is a named refusal `DepthScaleTooSmall` |
| `CLIPPER_Z_MIN_MAX_CLIPPING_PLANES` (109) | `E:489-493`, `util_viewport_zmin_zmax(.., clip_halfz)` | `X:914` | none | emits, the -1..1 case only |
| `VIEWPORT_OFFSET` (108) | `E:495-520` | `X:907` | none | emits |
| `CLIP_WINDOW` (107) | `E:259` (from the viewport and scissor, `E:236-285`) | `X:900` | none | emits |
| `DEPTH_BOUNDS_TEST_LIMITS` (93) | not emitted by the GL driver at 26.2.1 | `X:826` (min_ver 71) | 7.1-only | not emitted (GL ES has no state for it) |

### 1.1 `CFG_BITS`: field by field (E:300-385)

| field | Mesa | ver 42 | ver 71 | `draw71` |
|---|---|---|---|---|
| `enable_forward/reverse_facing_primitive` | `E:300-316`: forward on for lines and points regardless of cull | same | same | culls polygons only (`d.polygon &&`), same rule |
| `clockwise_primitives` | `front_ccw` ("seems backwards", `E:312-316`) | same | same | `front_cw ^ flip_y`, as `glue71`/compose pin it |
| `enable_depth_offset` | `offset_tri` | same | same | yes |
| `rasterizer_oversample_mode` | `multisample \|\| sample_mask != 0xf` `E:322-326` | 2 bits | 2 bits (`X:859`) | always 0; multisample is refused (`Multisample`) |
| `direct3d_provoking_vertex` | `flatshade_first` `E:328` | | | false: GL ES default is last vertex |
| `blend_enable` | `blend_enables && !soft_blend && !use_software` `E:330-332` | | | target 0's enable, or "any" with independent blend |
| `early_z_updates_enable`, `early_z_enable` | `E:335-345`, only `V3D_VERSION == 42` (`job->ez_state`) | bits 16, 17 | **gone**: early-Z moved to the render config's `Early-Z disable` (`X:1002`, `X:1044`) and the draw's shader record | not in `CfgBits`; `EarlyZ71` carries the rules (`tile_mode_early_z_disable: true`) |
| `z_updates_enable` | `depth_enabled ? depth_writemask : 0` `E:340-343` | | | yes, plus: off under `GL_EQUAL` (draweq71, as the Mesa dumps show) |
| `depth_test_function` | `depth_enabled ? depth_func : ALWAYS` `E:346-350` | | | same |
| `stencil_enable` | `stencil[0].enabled` `E:352` | | | `d.stencil_test` |
| `line_rasterization` | smoothing: `PERP_END_CAPS`, else `DIAMOND_EXIT` `E:357-359` | 2 bits `X:842` | 1 bit `X:861` | always 0 (diamond exit); GL line smoothing is not a GL ES 3.0 state |
| `direct3d_wireframe_triangles_mode`, `direct3d_point_fill_mode` | polygon mode `E:361-378` | | | false (GL ES has no polygon mode) |
| `z_clipping_mode` | **71 only** `E:379-383`: `clip_halfz ? ZERO_TO_ONE : MIN_ONE_TO_ONE`; `NONE` when neither `depth_clip_near` nor `depth_clip_far` | absent | 2 bits at 22, enum `X:322` | always `MinOneToOne` (GL ES) |
| `z_clamp_mode` | **71 only** `E:384`: `depth_clamp` | absent | bit 10 | false (GL ES has no `GL_DEPTH_CLAMP`) |
| `depth_bounds_test_enable` | not set by the GL driver | absent | bit 5 | false |

### 1.2 Blend details that decide the packets

* `v3d_factor` (`E:33-80`): with `dst_alpha_one` (the target's format has no
  alpha, `S:548-561`) `DST_ALPHA` becomes `ONE`, `INV_DST_ALPHA` `ZERO`,
  `SRC_ALPHA_SATURATE` `ZERO`.
* Min/max: gallium hands `ONE, ONE` for MIN/MAX; `draw71` does the same.
* A `blend_enable` target with a blend factor of 0 gives `ZERO`
  (`E:36-38`): only when blending is off, not reached.
* Dual-source blend, `V3D_DBG(SOFT_BLEND)` and formats Mesa cannot blend in
  hardware fall to software blend (`S:121-146`, `S:515-545`): no
  `BLEND_CFG`, `BLEND_ENABLES.mask = 0`, the blend is in the fragment shader.
  GL ES 3.0 has no dual source; the "format cannot blend" case
  (`v3d_fb_needs_soft_blend`) is the one the 7.1 table must refuse or
  lower, today it is not reached (see section 2).
* `swap_color_rb` is cleared at `S:547` and set nowhere else in 26.2.1's
  state file (BGRA-order targets are a 4.2 matter); keep it out of 7.1 until a
  target format needs it.

## 2. The gap against `draw71`, per packet

`translate` (`draw71.rs`, near the end) covers: CLIP_WINDOW, CFG_BITS,
DEPTH_OFFSET, CLIPPER_XY/Z/Z_MIN_MAX, VIEWPORT_OFFSET, BLEND_ENABLES,
BLEND_CFG, COLOR_WRITE_MASKS, STENCIL_CFG and `EarlyZ71`. Compose5/stencilops
pin the byte equality with refs71b's clifs. What it **lacks**, by packet:

1. `BLEND_CONSTANT_COLOR`: not in `StatePackets71`. Only the C job builder
   (and `fill71`'s `blend_color_f16`, `fill71.rs:740`, clamped to 0..1 in
   `glue71.rs:650`) carries it. Mesa emits it only when the colour changed; the
   7.1 builder emits it every draw. Equal bytes either way. Owned by pkt71.
2. `blend_dst_alpha_one` factor rewriting (`E:33-80`): `draw71` refuses a
   target without alpha that reads destination alpha
   (`Refusal71::BlendReadsMissingAlpha`) where Mesa rewrites the factor and
   draws. GL semantics (dst alpha = 1 for an RGB target) match Mesa's rewrite,
   so the refusal is a **divergence from Mesa**, not from GL. Lifting it is the
   largest real gap of the blend half (needs `has_alpha` per target, already in
   `DrawState.targets`).
3. Two-group `BLEND_CFG` when targets differ in having alpha without
   independent blend (`E:536-554`): not done (`draw71` emits one `0xff`
   packet). Needed together with item 2 for an MRT mix of RGBA and RGB
   targets.
4. `BLEND_CFG` mask for the non-independent case: Mesa uses
   `(1<<max_rts)-1`; at 7.1 that is `0xff`, as `draw71`. Correct.
5. `Z clipping mode` `ZERO_TO_ONE`, `Z clamp`, `NONE`: GL ES 3.0 has none
   (`glClipControl`/`GL_DEPTH_CLAMP` are desktop GL), so no gap for the
   gl_test or dEQP-GLES2/3 paths; only the enum plumbing exists.
6. `line_rasterization` PERP_END_CAPS and the line-width formula for smoothing:
   not a GL ES 3.0 state; no gap. `LINE_WIDTH`/`POINT_SIZE` themselves: spec
   and pack exist, not in `StatePackets71` (pkt71, C builder emits them).
7. Soft blend (`E:565-568`, `S:121-146`): absent; a blend on a format the
   hardware cannot blend (the format tables' `blend` flag) has no refusal in
   `draw71`. Check `table71`/`format71` for where to refuse. (Finding; no
   defect is opened, no GL ES 3.0 test case in the six reaches it.)
8. `DEPTH_OFFSET` Z16 variant is 4.2 only: no gap at 7.1. The 7.1 table has
   no 16-bit depth formats yet (`format71`/`rt71`: check before adding).
9. Dirty tracking: Mesa emits a group only when it changed, `draw71` emits
   all. Behaviour-equal; the byte counts differ from Mesa's bin lists in
   draws that follow another draw, which is why refs71b's pins compare the
   first draw of a shape or all draws of a shape (compose4/5 do per-draw).
10. Early-Z: `EarlyZ71` states the rules; nothing in the table enables
    early-Z yet (`tile_mode_early_z_disable: true`), as the dumps show
    for the reference shapes. A change to enable it must read
    `program_disables_job_early_z` (Mesa `v3d_update_job_ez`, `D:918`, called at `D:1236`).
11. Stencil: the reference value is Mesa's per-draw patch (`E:605-617`); `draw71`
    takes it from the draw. No gap. The `ez_state` bits for stencil
    (`S:215-221`) are a 4.2 matter.

## 3. How refs71b and compose71d split

Both items are marked done in the plan; this is what they pinned and what
is still open, so a follow-up item (named in "Handoff" of the report) does not
redo it.

* **refs71b (references).** Mesa 7.1 CLIF dumps of GL shapes, produced by
  `tools/v3d/pi5/src/gl_shapes.py`, one shape per packet group:
  `gl_blend`, `gl_blendsub` (SUBTRACT, REVERSE_SUBTRACT, MIN, SATURATE),
  `gl_depthfunc`, `gl_depthfuncs` (every compare function, mask off),
  `gl_stencilops`, `gl_colormask`, `gl_writemask`, `gl_mrt4`/`gl_mrt8`,
  plus indexed, point and line shapes. Missing references, to be added when
  the gaps of section 2 are lifted: **(a)** a blend on an RGB (no alpha)
  target reading destination alpha (item 2/3: `E:536-554`); **(b)** an MRT
  with RGBA and RGB targets, no independent blend (the two-group case);
  **(c)** polygon offset with `limit != 0` (`gl_depthoffset`: units, factor,
  clamp; today only `DEPTH_OFFSET` presence is pinned); **(d)** `glBlendColor`
  changed between two draws (the "emit only on change" side). Each needs the
  Mesa side built with `build-v3d` (tools/v3d/README.md); none needs a board.
* **compose71d (composition).** `draw71::translate` -> `glue71` ->
  `fill71` -> `backend71`/`table71`: carrying the packets above into
  `StatePackets71` (blend constant, point size, line width as fields, so a
  draw is one value), and the `dst_alpha_one` rewrite in `to_cfg`
  (`BlendFactor` mapping needs `has_alpha` per target, in `DrawState`
  already). It must not change the packet bytes for the shapes already pinned.

### 3.1 Host tests each needs (`tests/v3d71_*`)

Existing, and what they already pin: `v3d71_draw` (every layout against the XML
text and the generated 7.1 packer; round-trips), `v3d71_compose{,2,3,4,5}` (the
state packets of each shape against the clifs, byte for byte), `v3d71_stencilops`
(STENCIL_CFG per draw), `v3d71_job71_fixed` (the C job's fixed-function
packets against the same clifs), `v3d71_glue_prims`, `v3d71_mrt4`.

To add, one new file per item (the plan keeps `tests/**` additions new-file only):

| test file | what | needs |
|---|---|---|
| `tests/v3d71_blend_dstalpha.rs` | RGB target + `DST_ALPHA`/`INV_DST_ALPHA`/`SATURATE` give `ONE`/`ZERO`/`ZERO` in `BLEND_CFG`; two-group mask split (`E:536-554`) for an RGBA+RGB MRT | ref (a), (b) from refs71b; the lifted refusal |
| `tests/v3d71_depthoffset.rs` | `DEPTH_OFFSET`'s limit, f187 units and factor against a clif with `limit != 0`; absent without `offset_tri`; no Z16 variant at 7.1 | ref (c) |
| `tests/v3d71_blendcolor.rs` | `BLEND_CONSTANT_COLOR` bytes from `glue71`'s clamped colour equal Mesa's F16 words, red/blue unswapped | ref (d); pkt71's spec |
| `tests/v3d71_cfgbits_zclip.rs` | `Z clipping mode`, `Z clamp`, `Depth Bounds` and `Line Rasterization` stay at their GL ES values across every pinned shape (a guard, no new Mesa output) | none |
| extension of `v3d71_compose5.rs`'s idea | the three-draw sequence where a later draw changes only the stencil reference or only the blend colour | a ref built from `gl_shapes.py` |

## 4. What this note does not decide

* Whether `StatePackets71` should own point size, line width and the blend
  constant (pkt71 decides; this note only shows Mesa emits all three in the same
  group, `E:421-443`, `E:587-598`).
* Whether the three Gallium driver files should be vendored under
  `third_party/` (the owner's call; section 0).
* Hardware behaviour: nothing here was run on a board.

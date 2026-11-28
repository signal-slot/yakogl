<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR (AM62P): the 24 `fragment_out.array.float` fails (FO-ARRAY-FLOAT)

Item cause-foarray of plan #400. This is a cause note: nothing in it is
implemented. Line numbers are at `ad2b6cc9`. Grep the quoted name if they
drift. The dEQP source is VK-GL-CTS `93bca01`,
`modules/gles3/functional/es3fFragmentOutputTests.cpp`.

## 1. The cases

`docs/es3/am62p-gles3-notpass.tsv` has 24 `Fail` lines under
`dEQP-GLES3.functional.fragment_out.array.float.*`. All 24 say "Image
comparison failed". They are tree row 4, board build `b25997e4`, run
2026-11-22 (commit `818aa90d`). That build already contains `f3409705`
(three 16F colour outputs compile per target) and `c558647b` (RG16F and
RGB16F are colour targets).

| format | precisions | types | Fail |
|---|---|---|---|
| RG16F | lowp, mediump, highp | float, vec2, vec3, vec4 | 12 |
| RGBA16F | lowp, mediump, highp | float, vec2, vec3, vec4 | 12 |

The rest of `array.float` on the same run:

- R16F: 12 cases, all Pass.
- R32F, RG32F, RGBA32F, R11F_G11F_B10F: 48 cases, all NotSupported ("Unsupported
  attachment format": no `GL_EXT_color_buffer_float`, which ES 3.0 allows).

**The array shape** (`es3fFragmentOutputTests.cpp:1207-1245`). Every
`array.float` case has three attachments of one format, 64x64, single
sample (`numTargets = 3`). It has one output
`layout(location = 0) out <prec> <T> out0[3]`, so the array length is
always 3, with element k at location k. There is no clear: the grid of
triangles covers the whole viewport (`:590-597`, `:810`). Each attachment
is read with `glReadPixels` and compared with a ULP threshold of at least
1 half-float ULP (`:898-939`). So precision alone cannot fail it.

## 2. The cause

**The third attachment is spilled to tile buffer 0, and that spill does
not draw correctly on the board.** This is the overflow side of the
output-dword budget (`docs/notes/pvr-mrt4-budget.md`, section 2.2, row
"2, 2, 2"). It is not the budget refusing the draw, and it is not one of
the other three suspects.

The format decides the cause:

- The GL core keeps RG16F (and RGB16F) on the RGBA16F surface:
  `src/gl/framebuffer.rs:849-859`, `GL_RGBA16F | GL_RGB16F | GL_RG16F =>
  GLES_SURF_RGBA16F`.
- So an RG16F attachment takes 2 output dwords, like RGBA16F:
  `surf_out_dwords`, `src/pvr/backend.rs:1074-1080`.
- Three such attachments are 6 dwords. That is past the pixel's 4 output
  registers, so `mrt_layout` places them at r0-1, r2-3 and tile buffer 0,
  layout `(3,1)`, program `eot3_tb`: `src/pvr/mrt_layout.rs:196`, and
  `chain::target_layout` at `src/shaderc/online/powervr/chain.rs:1696-1721`.
- R16F takes 1 dword per attachment: r0, r1, r2, with no spill and the
  plain three-target end-of-tile program. Those 12 cases pass on the same
  board run.

The only difference between the 12 passing R16F cases and the 24 failing
RG16F/RGBA16F cases is the spill. They share the front end's
array-element path, the draw-buffer count (3) and the per-target compile.

Why the other three suspects are ruled out:

| suspect | verdict | why |
|---|---|---|
| draw-buffer count (3 on the PowerVR, backend-caps.md 11.4) | no | the cases use exactly 3 attachments; the FBO is COMPLETE and the draw is made (one kick, `num_rt` 3, section 3) |
| float format packing | no | `fragment_out.basic.float.{rg16f,rgba16f}_*` (one target, same `f16f16` packing) are not in the not-pass list, so they pass; attachments 0 and 1 of the array case use that same packing from registers |
| array output indexing in the compiler | no | R16F arrays pass on the board through the same front-end path (`store_shadows`, per-element `StoreOutput` at `location + k`); `tests/es3_pvr_fragout_array.rs` (`half_float_arrays_of_three_targets`) pins each element's own lanes |
| output-dword budget | **yes, its spill** | only the 6-dword layouts fail; the host records the intended `eot3_tb` spill exactly, so the defect is in the part of the spill the host does not run |

## 3. Host evidence

I wrote a scratch copy of `tests/pvr_f16_mrt_gl.rs` with two more cases:
3 x RG16F and 3 x RGBA16F, dEQP's `out0[3]` shape. It ran with
`cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online`.
It was deleted afterwards and is not committed.

- **RG16F and RGBA16F give the same draw, word for word:**
  - `glGetError() == GL_NO_ERROR`, one kick, `num_rt` 3;
  - end-of-tile program `0x2003` (`eot3_tb`, `eot_doutu_src0_tb_n[3]`);
  - PBE states `F16F16F16F16` (packmode 10, F16 source) at `source_pos`
    0 (`mrt_index` 0), 2 (1) and 0 from the tile buffer (2);
  - the fragment record has `num_outputs 4` and `num_tile_buffers 1`.

  The existing `three_rgba16f_draw_through_tile_buffer_0`
  (`tests/pvr_f16_mrt_gl.rs:793-823`) pins the same words for RGBA16F,
  including tile buffer 0's address pair, which the draw uploads to the
  record's `tile_buffer_sh`. So on the host, the draw the board gets is
  the designed one.
- **The host cannot run the spilled store.** The fragment program of the
  key (`SHC_BLEND_KEY_REPLACE | shc_blend_key_rt(SHC_RT_F16X4)`, 0x40088621)
  run through the host byte model stops with `"a savmsk other than vm"`
  (`tests/usc_exec/mod.rs:632-634`). The spilled target's store begins
  with `savmsk.icm`, so `TileStores` group 1 (section 4) is never executed
  on the host. Neither is the `st.tiled` at `PCO_SR_TILED_ST_COMP0 + c`, nor
  the `eot3_tb` reload. The register targets (r0-r3) have been checked
  through the byte model, and the spilled one has not.
- Both formats log the same three lines in the draw:
  - "3 colour attachments but the fragment shader writes 2 output(s)",
    twice (`src/pvr/backend.rs:9471-9477`; it compares `fs.num_outputs`
    with the target count);
  - "frame load: colour attachment 2 (RGBA16F) is not restored (no output
    registers for it)" (`src/pvr/backend.rs:4956`).

  dEQP's grid writes every pixel of every attachment, so the skipped load
  of attachment 2 cannot change these cases' pictures. It would matter
  for a partial draw over a spilled attachment.
- No host dEQP run can decide this. Host dEQP-GLES3 render verdicts are
  meaningless (`docs/es3/host-gles3-tip.tsv` header), and
  `docs/es3/host-gles3-part-powervr.tsv` lists these cases only as
  render-verdict-free.

**Not checked:** which of the three attachments differs on the board. The
board record (`records/am62p/deqp-gles3-b25997e4/`) is gitignored and was
not on this host. The cause predicts attachment 2 (the spilled one), with
attachments 0 and 1 correct.

## 4. Code locations of the spill

| step | where |
|---|---|
| RG16F/RGB16F kept as RGBA16F | `src/gl/framebuffer.rs:849-859` |
| 2 dwords per 16F attachment | `src/pvr/backend.rs:1074-1080` (`surf_out_dwords`) |
| layout (3,1) -> `eot3_tb` | `src/pvr/mrt_layout.rs:196`; `src/shaderc/online/powervr/chain.rs:1696-1721` (`target_layout`), `:1191-1263` (tile-buffer shared registers, `TargetPlace::TileBuffer`) |
| the fragment program's tile store | `src/shaderc/online/powervr/assemble.rs:991-1074` (`tiled_st_comp`, `TileStores`: `savmsk.vm`/`.icm`, `pck.cov`, `ushr`, `tstz`, `csel`, then per dword `add64_32` of `TILED_ST_COMP` and `st.tiled`), `:2468-2518` (placement, `Step::Tile`) |
| tile-buffer address upload | `src/pvr/backend.rs` (the `tile_buffer_sh` DMA, pinned by `tests/pvr_f16_mrt_gl.rs:812-822`) |
| tile buffers and the `eot_tb` programs | `driver/src/pvr_render.c:615-630` (`tile_buf`, `eot_tb_upload`), `driver/src/pvr_render.h:66-73` (`PVR_TILE_BUFFER_BYTES`: 16 x 16 x 16 partitions x 2 dwords, which matches `third_party/mesa-26.2.1/src/imagination/common/device_info/bxs-4-64.h:93-104`) |
| pixel output width | `src/pvr/backend.rs:556-600` (`usc_pixel_output_ctrl`, `CR_USC_PIXEL_OUTPUT_CTRL`) |

## 5. Proposed fix

The defect is in a hardware step that the host does not model. So the fix
goes in this order: first make the host able to see the spill, then find
on the board which step is wrong, then correct that step.

1. **Host model of the spill** (`tests/usc_exec/mod.rs`, new test
   `tests/pvr_fragout_f16_spill.rs`, feature set
   `--no-default-features --features host-test,gpu-powervr,shaderc-online`):
   - Model `savmsk.icm`, `pck.cov`, `csel` and `st.tiled`, with
     `TILED_ST_COMP0..` as inputs.
   - Then assert that the 0x40088621 program of `out0[3]` writes element
     0 to r0-1, element 1 to r2-3, and element 2's two dwords to tile
     buffer 0 at the offsets `TILED_ST_COMP0`/`1`.
   - Check the group order against pco's `trans_store_output_fs`.
   - This either finds the defect in `TileStores` or clears it.
2. **Board probe** (a later AM62P item, not this plan's board items):
   - Rerun `fragment_out.array.float.rgba16f_highp_vec4` and
     `rg16f_highp_vec2` with the dEQP log's per-attachment images (the
     record of tree row 4).
   - If only attachment 2 is wrong, the spill is confirmed. Then check its
     values:
     - zeros or the stale contents mean the store or the reload never
       happened (the `tile_buffer_sh` upload, or the `eot3_tb` immediates
       of `eot_tb_upload`);
     - another pixel's values mean the tile offsets or the tile-buffer
       size or partitioning are wrong (`driver/src/pvr_render.h:66-73`,
       `CR_USC_PIXEL_OUTPUT_CTRL` width or partitions,
       `src/pvr/backend.rs:556-600`).
   - If attachments 0 or 1 are wrong too, this note's cause is wrong for
     them; reopen the packing row of section 2.
3. **The fix itself** goes in whichever of these the probe names:
   `src/shaderc/online/powervr/assemble.rs` (`TileStores`),
   `driver/src/pvr_render.c` (tile buffers, `eot_tb_upload`) or
   `src/pvr/backend.rs` (the tile-buffer upload, the pixel output width).
   No `src/gl` change is needed.
4. **Fallback** if the spill cannot be made to draw: answer
   `GL_FRAMEBUFFER_UNSUPPORTED` for the (2,2,2) list through the back-end
   hook of `docs/notes/pvr-mrt4-budget.md` section 5. That turns the 24
   Fails into NotSupported, which `fragment_out` allows. It is a
   `src/gl/framebuffer.rs` plus `src/pvr/backend.rs` change, and section
   4.3 of that note argues against it for the Quick 3D G-buffer (3 x
   RGBA16F), so it is the last resort.

The same spill is reached by every three-target list of 16F attachments
(the Quick 3D G-buffer, `docs/notes/pvr-mrt4-budget.md` 4.3). It may also
be reached by the seven `fragment_out.random.*` "Image comparison failed"
lines (3, 6, 42, 43, 52, 87, 93), whose target lists were not checked
here. `docs/es3/int-rt-gc.md` section 6 (the `fix-fragout-design` intro)
says this cluster "is fixed by `f3409705` and `c558647b`". The
`b25997e4` board run shows that is true on the host only.

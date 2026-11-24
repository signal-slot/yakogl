<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 7.1 table: how `yakogl_backend_v3d71` maps the GL core onto fill71

Design note of ROADMAP stage 5, step P9 (the Pi 5's first gl_test cases 5,
58, 59, 66, 67 and 102). It fixes how the 7.1 GL table, the
`YakoglBackend` called `yakogl_backend_v3d71`, turns the GL core's
`GlesDraw` into the inputs of the pure 7.1 modules and the C builder
`driver/src/v3d/v3d_job71.{c,h}`. Two plan items build it:
`table71-draw` (draws and full clears) and `table71-cases` (the six cases and
a host run of them). There is no code here. Every line number was read on
`f642eb31`.

Read first: `docs/notes/v3d71-gl-table.md` (the P9 figures, the build
selection of its section 4) and `docs/notes/v3d71-refusals.md` (every named
7.1 refusal, and the predictions of its sections 7 and 9, which section 6
below revises).

Where the table lives: `src/v3d/backend71.rs` holds the pure job assembly
(`init` `:99`, `assemble_filled` `:743`). The table glue (its global state,
the `extern "C"` entries, the `YakoglBackend` static) is new code beside it.
Its file name and its `mod` line belong to `feat71`.

## 1. The entries the six cases need

The 4.2 table is `src/v3d/backend.rs:7265-7318`. "Same" means the 7.1 entry
copies the 4.2 function's contract word for word, over the 7.1 job.

| Entry (`src/backend.rs`) | 4.2 function | 7.1 entry: what differs |
|---|---|---|
| `name` `:1550` | `"Broadcom V3D 4.2 (Raspberry Pi 4)"` `:7266` | `"Broadcom V3D 7.1 (Raspberry Pi 5)"` |
| `native_attrib_formats` `:1553` | float32 only `:7268` | Same. fill71 refuses every non-float record (`REF_FLOAT_ATTRS`, `fill71.rs:120`, check `:755-757`) |
| `rt_rgba8888_is_bgra` `:1562` | false `:7271` | False: `color_format` keeps `r_b_swap` false (`fill71.rs:505`), and the RGBA8 store writes R, G, B, A |
| `mip_level_align`, `compressed_blocks_in_rows`, `native_layered_textures`, `front_facing_shader_variant`, `etc1_decoded_at_upload` `:1583-1622` | 0, true, true, false, true `:7272-7282` | The same values. None of the six cases samples a texture; the values stay right for P10 (`texture71`, `v3d71-gl-table.md` section 3) |
| `init` `:1623` | `v3d_init` `:6326` (VPM query `:6334-6345`, arenas, clear program, black texture) | `v3d71_init`: (a) the VPM query as at 4.2; (b) the hub identity through `backend71::init(hub_ident1, hub_ident3)` (`backend71.rs:99`), which gives the `Table71` (`ident`, `rev`) or ident71's named refusal. **The OSAL has no query for the identity** (section 4, F1); (c) the job memory of section 4; (d) the clear-quad program (section 2.4). No black texture and no TFU queue in the first cut |
| `shutdown` `:1624` | `v3d_shutdown` `:6498` | Frees what init allocated. Same contract |
| `draw` `:1627` | `v3d_draw` `:6705` → `draw_call` `:6786` | `v3d71_draw`: the mapping of section 2, which queues `fill71::Op`s for the open frame and dry-runs `fill71::fill` so that a refusal is the return value of this call (section 2.5). Keeps the 4.2 stamp logic (`stamped`/`open_alias`, `:6708-6720`) |
| `flush` `:1629` | `v3d_flush` `:7032` → `flush_job` `:2951` | `v3d71_flush`: `fill71::fill` on the queued ops, then for each `FilledJob` the C builder, then the submit (section 4) |
| `finish` `:1631` | `v3d_finish` `:7041` | Same: flush, then wait for every job in flight |
| `finish_previous` `:1633` | `v3d_finish_previous` `:7093` | Same contract. The 7.1 OSAL runs one job at a time and its submit finishes the job before (`zephyr_v3d71.c:45-46`, `:673-674`), so at most one job is outstanding. Waiting for the previous job is waiting for nothing or for that one |
| `kick_id` `:1639` | `v3d_kick_id` `:7147` (`kicks + 1`) | Same. Section 2.5 keeps "the kick holding the draw" true when fill71 splits a flush into several jobs |
| `kick_pending` `:1642` | `v3d_kick_pending` `:7160` | Same rule over the 7.1 in-flight record (`fence_done` through the OSAL's `completed`, `zephyr_v3d71.c:735-741`) |
| `heap_base` `:1643` | `v3d_heap_base` `:7171` | Same (shader/program heap and data heap) |
| `counters` `:1644` | `v3d_counters` `:7184` | Same (kicks, waits). The gl_test log prints them |
| `texture_released` `:1648` | `v3d_texture_released` `:5097` | `None` in the first cut. The 7.1 table keeps no copy of texel memory. It drops the colour history of a released range (section 2.3) |
| `surface_swapped` `:1653` | `v3d_surface_swapped` `:5520` | The depth/stencil history of the window's `ds_id` goes back to `Untouched` (EGL 1.5 3.10.1), as 4.2. gl_test calls no swap between cases, but the entry is cheap and keeps the GL contract |
| `wait_kick` `:1683` | `v3d_wait_kick` `:7065` | Same, with one outstanding job |
| `samples_texel_copies` `:1687` | true `:7301` | False. The 7.1 table samples nothing yet. False makes the GL core wait as for a GPU reader, which is the safe answer |
| `attachment_gone` `:1691` | `v3d_attachment_gone` `:5114` | Frees the D24S8 buffer kept for that `ds_id` (section 2.3). Case 67 deletes renderbuffers between frames |
| `uniforms_host_only` `:1756` | true `:7312` | True. fill71 takes the blocks as `&[u32]` from `vs_uniform_cpu` and `fs_uniform_cpu` (section 2.2) |
| `max_instance_divisor` `:1679`, `max_samples` `:1781`, `linear_row_align` `:1748` | 0xffff, 0, 0 | 1 (backend71 refuses instancing, `REF_INSTANCED` `backend71.rs:666`), 0, 0 |
| `depth_resolve`, `stencil_resolve`, `ds_write`, `query_*`, `generate_mipmap`, `return_mipmap_chain` | `Some(..)` `:7303-7316` | `None` in the first cut. The GL core then refuses by name what needs them, or does it on the CPU (`generate_mipmap` `None` = 0, `:1723`). None of the six cases reaches them |

## 2. GlesDraw and the GL core's clears onto `fill71::Op`, `Framebuffer`, `History`

`fill71::fill(fb, rev, hist, quad, ops)` (`fill71.rs:964-1073`) takes one
flush of one framebuffer. The table therefore keeps an **open frame**: a
`Framebuffer`, its `History` and the queued `Op`s. A `GlesDraw` whose frame
key (section 2.1) differs from the open frame's ends the open frame (flush)
first. 4.2 does the same with `g.j.target != target` (`backend.rs:6923-6928`).

### 2.1 `Framebuffer` (`fill71.rs:178-189`) and its key

| `Framebuffer` / `ColorBuffer` / `DsBuffer` field | from `GlesDraw` (`src/backend.rs`) | Note |
|---|---|---|
| `width`, `height` | `target_w` `:1186`, `target_h` `:1187`, intersected with each `targets[k]` written (4.2 `extra_targets`, `backend.rs:6752-6780`) | The six cases have one target |
| `flip_y` | `flip_y` `:1315` | A window is flipped, an FBO is not (`fill71.rs:184-186`) |
| `tile_alloc` | not from the draw: the table's tile-allocation pool (section 4) | 64-byte aligned |
| `colors[0].gl_format` | `target_format` `:1188` with `target_alpha_bits` `:1204`: `GLES_SURF_RGBA8888` with 8 alpha bits gives `GL_RGBA8`, with 0 alpha bits (GL_RGB, case 67's frame) `GL_RGB8`, `GLES_SURF_SRGB8_A8` gives `GL_SRGB8_ALPHA8` | Every other surface format is refused by name before fill71 (fill71 maps only these three, `fill71.rs:217-224`). `XRGB8888` and `RGB565` would need `r_b_swap` or a 565 row, which fill71 does not set: refused as `"V3D 7.1: ..."`. gl_test's surface is RGBA8888 |
| `colors[0].address` | `gpu32(target_gpu)` `:1185` (4.2 `gpu32`, `backend.rs:3572`) | 0 (`DRAWS_WITHOUT_COLOUR`, `backend.rs:57`) is refused by name in the first cut: fill71 has no frame without a colour buffer |
| `colors[0].memory_format`, `height_in_ub_or_stride` | `V3D_MEMORY_FORMAT_RASTER`, `draw_pitch(d) * 4` (`src/backend.rs:1433-1439`, `target_pitch` `:1394`) | Raster, at the GL core's pitch: the store goes straight into the GL core's storage (section 4.3) |
| `colors[1..]` | `targets[1..num_targets]` `:1294-1295` | Not reached by the six cases |
| `ds` | `None` unless `ds_persist` `:1231`. With it: `gl_format` from the identities `depth_id` `:1241` and `stencil_id` `:1247`: both non-zero gives `GL_DEPTH24_STENCIL8`, depth only `GL_DEPTH_COMPONENT24`, stencil only `GL_STENCIL_INDEX8`. `address` = the D24S8 buffer the table keeps for `ds_id` `:1232`, `V3D_MEMORY_FORMAT_UIF_NO_XOR` with its padded height in UIF blocks (4.2 `zs_layout`, `backend.rs:1622`) | `GlesDraw` does not carry the depth format, so `GL_DEPTH_COMPONENT16` is kept as D24 (D24S8 image). 4.2 does the same, one D24S8 per `ds_id` (`zs_buf`, `backend.rs:1643`). Mesa's reference keeps D16 (`gl_fbods.clif`); a byte-exact D16 needs a new `GlesDraw` field (F3) |

The frame key is (`colors[*].address`, size, `gl_format`s, `ds_id`, `flip_y`).
`ds_id` is part of it because case 67 frames 3 and 4 draw into one colour
texture with two depth renderbuffers (`gl_test.c:8698-8702`). Those draws
must end the frame (4.2: `Target::ds`, `backend.rs:6851`).

### 2.2 `GlesDraw` to `Op::Draw(DrawIn)` (`fill71.rs:339-359`)

`index_count != 0` makes one `Op::Draw`, queued after the `Op::Clear` of
2.3 if the draw carries a fused clear (4.2 clears first, then draws:
`backend.rs:7013-7029`). The 4.2 refusals of `state::refusal` and
`state::unsupported` run first, unchanged (section 3).

| `DrawIn` / `DrawState` field (`draw71.rs:1020-1055`) | from `GlesDraw` |
|---|---|
| `state.viewport` | `viewport` `:1313` as f32 |
| `state.depth_range` | `depth_range` `:1314` |
| `state.depth_test`, `depth_write`, `depth_func` | `depth_test` `:1316`, `depth_write` `:1318`, `depth_func` `:1317`. The `GLES_CMP_*` values (`src/backend.rs:33-40`) are `draw71::Compare`'s, 0 NEVER to 7 ALWAYS (`draw71.rs:360-369`), so the conversion is a checked cast |
| `state.stencil_test`, `stencil_two_sided` | `stencil` `:1343`. Two-sided when any back field differs from its front field (`:1352-1358` against `:1345-1351`) |
| `state.stencil_front` (`StencilFace`, `draw71.rs:936-944`) | `func` from `stencil_func`; `sfail`, `zfail`, `zpass` from `stencil_sfail`, `stencil_dpfail`, `stencil_dppass` through `state::stencil_op` (`state.rs:354`, GLES_SOP to the hardware code), then to `draw71::StencilOp` by value (the hardware order, `draw71.rs:442-451`; GLES_SOP is not that order, `src/backend.rs:68-75`); `reference` `stencil_ref`, `value_mask` `stencil_cmp_mask`, `write_mask` `stencil_write_mask` |
| `state.stencil_back` | the `stencil_back_*` fields, the same way |
| `state.polygon_offset`, `offset_factor`, `offset_units`, `offset_clamp` | `polygon_offset` `:1319`, `polygon_offset_factor` `:1320`, `polygon_offset_units` `:1321`; clamp 0 |
| `state.front_cw`, `cull` | `front_cw` `:1344`. `cull` `:1340` is the culled winding (`GLES_CULL_CW`/`CCW`, `src/backend.rs:44-46`), turned into a face as `state::job_state` does (`state.rs:654-665`): culling CW with `front_cw` true is `Cull::Front`, otherwise `Cull::Back` |
| `state.scissor` | `scissor.then(scissor_rect)` `:1341-1342`, in GL coordinates. draw71's `scissor_box` flips it (`draw71.rs:1204`) |
| `state.targets[0]` (`TargetState`, `draw71.rs:984-989`) | `blend_enable` from `blend` `:1329`. `blend` from `blend_eq_*` and `blend_src/dst_*` `:1330-1335` (the `shc_blend_*` codes) mapped to `BlendMode`/`BlendFactor`. `color_mask`: `color_write ? color_mask : 0` (`:1360`, `:1339`). `has_alpha` is overwritten by fill71 (`fill71.rs:658-660`) |
| `state.samples` | 1. `samples` `:1189` above 1 is refused (no MSAA at 7.1 in the first cut) |
| `state.independent_blend` | `per_target_mask` `:1303` (fill71 then requires equal masks, `REF_ONE_MASK`) |
| `state.target_w/h`, `flip_y`, `nr_targets`, `polygon`, `frag` | overwritten by fill71 (`fill71.rs:653-663`) |
| `line_width` | `line_width` `:1259` |
| `blend_color` | `blend_color` `:1337`, clamped to 0..1 |
| `program` (`Program`, `fill71.rs:269-286`) | from `fs` `:1274` (the linked `ShcShader`; `ShcV3d` `src/shaderc/mod.rs:1278-1290`, the same record at 7.1: `abi71.rs:1-40`). `desc.fs/vs/cs.prog.code` = `fs_usc_gpu` `:1276` + `code_off` (the checks of 4.2 `programs`, `backend.rs:3712-3723`); `threads`, `single_seg`, `vpm_*`, `uses_vid/iid/base_instance`, the varying flags, `writes_z`, `disable_ez`, `uses_center_w`, `lock_scoreboard_on_first_thrsw`, `uses_implicit_point_line_varyings`, `uses_pid` field by field from `ShcV3dFs`/`ShcV3dVs` (`src/shaderc/mod.rs:1223-1271`); `min_input_segments` 1, `min_output_segments` 0, `propagate_nans` false, `sample_rate_shading` false (what fill71 requires, `REF_RECORD_FIXED`, `fill71.rs:744-749`); `point_size` = `vs.writes_psiz`; `cs_reads`/`vs_reads` = `vattr_sizes`; `*_vcm_cache_size`, `separate_segments` as named; `discards` = `fs.disable_ez != 0` (ShcShader has no discard flag; disable_ez is the compiler's answer for a discarding program); `*_uniforms` = the `ShcV3dUniform` lists as `uniform71::Word` (`contents`, `data`), **copied** (see 2.5) |
| `attrs` (`attr71::Attr`, `attr71.rs:169-191`), index = location | for each `attribs[i]` `:1278-1279`: index `vi_reg`; `enabled` true; `components`; `gl_type` GL_FLOAT (float32 only, `native_attrib_formats`); `normalized`; `integer` false; `stride`; `offset` 0 and `buffer_addr` = the staged copy of `cpu`/`cpu_bytes` in the data arena (4.2 `stage`, `backend.rs:3512`), or `buffer_gpu` + `offset` for a buffer object; `divisor` from `per_instance`/`divisor` |
| `dummy_addr` | the data arena's first byte (GFXH-930, `attr71.rs`; `v3d_job71.c:692-696`) |
| `prim` (`prim71::Draw`, `prim71.rs:114-122`) | `mode` = the GL mode of `prim` `:1258` (`GLES_PRIM_*` to GL_TRIANGLES etc.); `first` 0 (the GL core puts the first vertex into the array offsets, 4.2 `backend.rs:3954`); `count` `index_count` `:1260`; `indices` from `index_buf_gpu`/`index_cpu`/`index_size` `:1262-1269` (refused by backend71 `REF_INDEXED` in the first cut); `instances` `max(instance_count, 1)` `:1281`; `base_vertex`, `base_instance` 0 |
| `vs_words`, `fs_words` | `vs_uniform_cpu`/`vs_uniform_dwords` `:1285-1286`, `fs_uniform_cpu`/`fs_uniform_dwords` `:1288-1289`, **copied** |
| `oq_addr` | 0 (`occlusion_query` `:1402` is refused when non-zero: no `query_begin`) |

Fields fill71 does not read are refused by name when they are set: `textures`
and `vs_textures` (`:1306-1311`, until `texuni71` is wired),
`alpha_to_coverage`, `sample_mask != 0xf`, `instance_id != 0`,
`depth_texels`, and `index_count2` (already `state::unsupported`).

### 2.3 The GL core's clears to `Op::Clear(ClearReq)` (`fill71.rs:289-306`)

The GL core sends a clear as fields of a `GlesDraw`. The draw is clear-only
when `index_count` is 0, fused when it is not. The GL core plans each clear
itself (`src/gl/draw.rs:2906-2948`, `apply_clear_plan` `:3016-3053`,
`clear_quad` `:3226`). Any of `do_clear`, `do_clear_depth` or
`do_clear_stencil` makes one `ClearReq`:

| `ClearReq` field | from `GlesDraw` |
|---|---|
| `color` | `do_clear` `:1190` gives `Some(clear_rgba)` `:1210` (alpha already quantised by the GL core, `draw.rs:3043-3045`) |
| `draw_buffers` | `clear_target_mask` `:1256`: 0 means every target, `(1 << n) - 1`; `0x80` means none (`state.rs:128`); otherwise the mask |
| `color_mask` | `clear_color_mask` `:1366`, where 0 means 0xf (field docs) |
| `depth` | `do_clear_depth` `:1228` gives `Some(clear_depth)` `:1230` |
| `depth_mask` | true. The GL core drops a depth clear under `glDepthMask(GL_FALSE)` (`draw.rs:2925`), which is case 59's depth part |
| `stencil` | `do_clear_stencil` `:1229` gives `Some(clear_stencil)` `:1359` |
| `stencil_mask` | `clear_stencil_write_mask` `:1373`, where 0 means `stencil_write_mask` (as `state.rs:260`) |
| `scissor` | `clear_quad && scissor` gives `Some(scissor_rect)` (`:1227`, `:1341-1342`). The GL core sets the box only for a partial one (`draw.rs:2930-2943`) and drops an empty box itself (`:2932-2934`) |

`clear_quad` itself is not passed on. clearplan71 decides tile clear or quad
from the box and the masks (`clearplan71.rs:176`), and fill71 calls it with
`ds_valid` and `draws_queued` from its own open job (`fill71.rs:991-1006`).

### 2.4 `ClearQuad` (`fill71.rs:327-336`)

The table owns one `Program` for the float clear program, built at init. The
4.2 table compiles its clear source through the online compiler
(`build_clear_program`, `backend.rs:4273`); once `shc71` routes the link
through `link71`, the same source links to 7.1 code. The pinned 7.1 output
of that program is `tools/v3d/gltest-shaders71/ours_clear.h`, `u_color` at
fragment words 0..3 (`v3d71-gl-table.md` section 5). `attr` is one float4
array at a 64-byte slot of the data arena, re-staged per quad. After
`fill`, the table writes each `FilledDraw::quad` (`fill71.rs:397-399`)
there before the job is submitted. `dummy_addr` is the same dummy as for
draws.

### 2.5 `History` (`fill71.rs:193-207`) and when the table calls `fill`

`History` is per framebuffer. `colors[n]` is true when a job stored target n.
The table keeps it per colour address, as 4.2 keeps `zs_known` per target
(`backend.rs:1595-1620`). `z`/`s` are `zskeep71::Known71`
(`zskeep71.rs:134-146`), kept with the D24S8 buffer of the `ds_id`, as 4.2
keeps `ZsBuf::z/s`. A new address starts from `History::NEW`.
`texture_released` and a colour respecification drop the entry, and
`surface_swapped` resets the window's `z`/`s` to `Untouched`. After each
`FilledJob` the next history is `FilledJob::after` (`fill71.rs:440-446`).

Decision (fill at flush, dry run at draw):

1. `v3d71_draw` maps the `GlesDraw` into **owned** ops. The `GlesDraw`
   pointers are valid only for the call (`src/backend.rs:1627`; the 4.2
   table records into the C job at draw time). Owned means the uniform
   words, the `Program`'s uniform lists, and the array data staged into the
   job's data arena.
2. It runs `fill71::fill` on the queued ops plus the new ones (pure, no
   FFI). An `Err(Refusal)` returns the refusal (-ENOTSUP, the reason logged
   as `"{module}: {reason}"`) and leaves the queue as it was. The arena
   marks roll back, as 4.2's `record` does (`backend.rs:3849-3858`).
3. If the dry run gives one job more than the queue alone (clearplan71's
   `new_job`: a whole clear after queued draws, `fill71.rs:1009-1014`), the
   split is at the new clear. The table flushes the queued ops first
   (their jobs are submitted, `kicks` moves) and starts the queue with the
   clear. So `kick_id` after the call names the kick that holds the draw,
   the contract at `src/backend.rs:1634-1639`. 4.2 flushes before a TLB
   clear for the same reason (`backend.rs:6958-6965`).
4. `v3d71_flush` runs `fill71::fill` for real. Each `FilledJob` goes through
   `backend71::assemble_filled` (`backend71.rs:743-757`) with the
   `CBuilder71` (`backend71.rs:441`), and the result is submitted
   (section 4). The open frame's history becomes the last job's `after`.

A cap of `OSAL_QUERY_V3D_DRAWS_PER_JOB` queued draws (`zephyr_v3d71.c:808`)
bounds the dry run's cost (quadratic in the ops of one flush). When the
queue reaches it, the frame is flushed, as at 4.2.

## 3. `state.rs` reused, and what blocks reuse

Reusable unchanged (pure, `GlesDraw` in, plain values out; the module is
compiled in a `gpu-v3d71` build because the feature implies `gpu-v3d`,
`v3d71-gl-table.md` section 4):

| Function | Used for |
|---|---|
| `state::refusal` `state.rs:27` | first check of every draw (unknown colour format, sample count) |
| `state::unsupported` `:53` | alpha-to-coverage, `out_shift`, second index list |
| `state::stencil_op` `:354` | GLES_SOP to the hardware stencil op (section 2.2) |
| `state::colour_needs_quad` `:144` | not needed: clearplan71 decides it. It may be logged for comparison |
| `state::zs_use` `:829` | not needed: fill71 marks the planes (`fill71.rs:817-819`) |
| `state::draw_buffer_order` `:115` | a later MRT step. Not reached by the six cases |

Not reusable, and why. The 7.1 table does not call these. Each is named for
the planner, and this plan changes none of them:

- **`state::job_state` `:634`** returns a 4.2 `V3dJobState` (`ffi.rs`: 204
  bytes, `size_of` pin in `ffi.rs`), not a `draw71::DrawState`. The 7.1
  table maps the same fields into `DrawState` (section 2.2). The two
  translations are parallel; sharing them needs a GL-level state struct that
  both consume (fix F4, `state.rs`).
- **`state::viewport` `:290`, `wpos` `:310`, `scissor` `:321`** produce 4.2
  types, and draw71 has its own `viewport_xform`/`scissor_box`
  (`draw71.rs:1187`, `:1204`) at 7.1's 1/64-pixel clipper. Not needed.
- **`state::rt_format` `:500`, `clear_words` `:597`** are the 4.2 TLB rows.
  rt71 has the 7.1 ones (`fill71.rs:494`, `:1037`). Not needed.
- **`state::blend_factor` `:371`** maps to 4.2 `V3D_BLEND_FACTOR_*`. The
  numbers are the CLE's in both generations, but the table needs
  `draw71::BlendFactor`. It maps `shc_blend_*` itself.
- **`state::clear_quad_draw` `:209`** makes a 4.2-style six-vertex
  `GlesDraw`. fill71 makes its own blitter quad (`fill71.rs:858-915`).
- **The 4.2 table's own refusals** (`record`, `backend.rs:3814-3824`: "the
  draw reads depth/stencil an earlier job wrote, and the target has no
  depth/stencil buffer to keep it in") are not called. Their 7.1 twins are
  `zskeep71::ZsRefusal71::ReadLost`, word for word (`zskeep71.rs:460-478`).
  The string at `backend.rs:3791` cited by `v3d71-gl-table.md` section 5 has
  moved to `:3823`. The 67 string `"a colour format with fewer than 8
  alpha bits"` (`state.rs:969` in that note) no longer exists: the test
  `state.rs:1121-1135` pins that such a target is drawn
  (`v3d71-refusals.md` section 7).

4.2-only assumptions in shared files that a 7.1 build passes through (all in
files this plan does not change):

- **F1, `src/osal/mod.rs` and `src/osal/zephyr_v3d71.c`.** No OSAL query
  returns `HUB_IDENT1`/`HUB_IDENT3`. `zephyr_v3d71.c` reads them
  (`s.id.hub_ident[]`, `:228`; `struct v3d_identity`,
  `driver/src/v3d/v3d.h:69-79`) but answers only the VPM size (`:770-772`).
  `backend71::init` needs both words. Fix: a new
  `OSAL_QUERY_V3D_HUB_IDENT` (out = two u32), answered by
  `zephyr_v3d71.c`'s `z_query`. Until then, the table passes the expected
  D0 words and relies on the OSAL's own refusal of a non-7.1 part
  (`zephyr_v3d71.c:231-234`). The IPREV check (`>= 10`) would then not run
  on the board. Owner: `feat71`/`zlib71` (both shared files).
- **F2, `src/backend.rs:1800-1802`.** `g_backend` is cfg'd on `gpu-v3d`,
  which `gpu-v3d71` implies, so a 7.1 build would export both
  `yakogl_backend_v3d` (`#[no_mangle]`, `backend.rs:7264`) and the 7.1
  table, and the 4.2 table references the 4.2 C builder (`v3d_job_begin`,
  `ffi.rs:749`). That is `feat71`'s cfg pick. It must also make sure the
  7.1 image does not need `v3d_job.c` at link time (gate the 4.2 static, or
  link the 4.2 builder too). Owner: `feat71`/`gltest71`.
- **F3, `src/backend.rs` `GlesDraw`.** There is no depth/stencil GL format,
  only identities (`:1232-1253`). The 7.1 table keeps every depth buffer as
  D24S8 (section 2.1). Byte equality with Mesa's D16 frames
  (`gl_fbods.clif`) needs a field. That is not needed for a pass.
- **F4, `src/v3d/state.rs`.** The GL-to-gallium translation is written
  against 4.2's `V3dJobState`. A shared GL-level struct would end the
  duplication in section 2.2. Optional. `src/gl` is held by plan #397.
- The `src/backend.rs` hooks `samples_rendered_depth` `:2050-2053`,
  `defers_depth_storage` `:2079-2082`, `storage_current` `:2104-2105` and
  `gl/egl.rs:451` (`msaa_on`) call the **4.2** backend in a `gpu-v3d71`
  build. Each checks its own `inited`, so each answers no, does nothing, or
  offers no 4x config while the 4.2 table is not initialised
  (`backend.rs:2224-2228`, `:4593-4594`). That is the right answer for the
  7.1 first cut, and nothing needs changing until 7.1 samples depth or
  keeps render images. `DEPTH_ONLY_TARGETS` (`src/backend.rs:859-861`) is
  4.2's `true`, so the GL core sends `target_gpu` 0 draws, which the 7.1
  table refuses by name (section 2.1).

## 4. Memory, submit and readback

### 4.1 What the 7.1 OSAL offers (`src/osal/zephyr_v3d71.c`)

| OSAL op | 7.1 behaviour | Table use |
|---|---|---|
| `gpu_alloc`/`gpu_free` `:359`, `:398` | granule map over the carve-out, Normal non-cacheable CPU view (`:65-67`); frees deferred until the last job that may read them completes (`:25-26`) | arenas, tile pools, D24S8 buffers, clear-quad vertices |
| `query` `:768-821` | `VPM_BYTES`, `CPU_OF_GPU`, `JOB_OPEN`, `SPLIT_ZS`, `S8_STENCIL`, `TILED_BUDGET`, `DRAWS_PER_JOB`, `BIN_POOL`; **not** the TFU ones and **not** the identity (F1) | VPM check, CPU view of a GPU address, the job-open flag for deferred frees (as 4.2 `job_open_to_osal`, `backend.rs:3351`) |
| `submit` `:655-733` | takes the 4.2 `struct v3d_kick` (`src/v3d/v3d_kick.h:30-37`; Rust `V3dKick`, pinned 40 bytes `src/abi_layout.rs:712`). It finishes the job in flight first, zeroes the tile state, binning polled to FLDONE with the overflow blocks in hand, then the render. Returns the fence | one `V3dKick` per `FilledJob` |
| `wait`/`completed` `:748`, `:735` | FRDONE polled, caches cleaned, held frees drained | `finish`, `wait_kick`, `kick_pending` |

### 4.2 The job's memory

For each `FilledJob` the table needs: three `V3dCl` lists (`bcl`, `rcl`,
`ind`) for `v3d_job71_begin` (`v3d_job71.h:282-283`), a data arena (staged
arrays, the GL uniform blocks the builder writes into streams, the quad
vertices, the dummy), a tile allocation buffer (its address goes into
`Framebuffer::tile_alloc` **before** `fill`; size from
`v3d_job71_alloc_sizes` after `v3d_job71_end`, `v3d_job71.h:299-300`,
rounded to `backend71::PAGE_SIZE` 16 KiB, `backend71.rs:74`), and the tile
state (size from the same call). The 4.2 layout carries over: two arena sets
used in turn (`choose_arenas`, `backend.rs:3106`) and pools sized at init for
the largest target. With one job outstanding at 7.1 (section 1), one arena
set is enough, and a second set only overlaps CPU recording with the GPU.
4.2's `arena_sizes` (`backend.rs:73`) is for 4.2 lists: the 7.1 record is 32
bytes, not 36, and there is no default-attribute BO (`v3d71-gl-table.md`
items 1 and 3). The 7.1 table sizes its own from `v3d_job71.h`. The tile
pool is sized for 7680 px at one render target (`rt71.rs:40-47`). The six
cases use 64x64.

The submit is 4.2's `submit_job` (`backend.rs:3376-3499`) without the 4.2
parts. It flushes the CPU caches of the written lists and the arena (a no-op
on the NC carve-out, kept for the contract), builds `V3dKick { bcl_start,
bcl_end, rcl_start, rcl_end, tile_alloc, tile_alloc_size, tile_state,
tile_state_size }` from `v3d_job71_end`'s lists, calls `submit`, and records
`InFlight { kick, fence }`. No `ArenaUse` bin-pool accounting and no
render-image or MSAA bookkeeping.

### 4.3 Readback: does the GL core read what v3d_job71 stores?

Yes, for the six cases, given section 2.1's choices:

- The colour target is stored **raster** at the GL core's own address and
  pitch (`memory_format` RASTER, stride `draw_pitch * 4`). fill71 stores
  every target that has a buffer (`fill71.rs:63-65`, `end_job` `:923-933`).
  `glReadPixels` in the GL core waits for the target's kick (`kick_pending`
  / `wait_kick` / `finish`) and reads the bytes through the CPU view.
  Nothing 4.2-specific sits in between: the lazy raster store and render
  image (`storage_current` → `v3d_storage_current`, `backend.rs:2224`) is a
  4.2 table feature, and in a 7.1 image its state is uninitialised, so it
  returns at once (`:2226-2228`).
- Byte order: RGBA8 output format, R, G, B, A in memory
  (`rt_rgba8888_is_bgra` false). The window's y flip is in the viewport and
  the clipper (`draw71::viewport_xform`), not in the store, as at 4.2.
- GL_RGB (case 67 frame 2): stored as RGBA8 with `force_alpha_1`
  (`fill71.rs:506`). Draws do not write alpha and clears write 1
  (`fill71.rs:58-62`). The GL core reads alpha 1, as GL requires. The
  host shapes test lists the clear value against Mesa's as the allowed
  difference `RGBALPHA` (`tests/v3d71_backend71_shapes.rs:1240`).
- Depth and stencil are never read back by the six cases. Their buffers
  are the table's own UIF D24S8, which the CPU does not read
  (`depth_resolve` and `stencil_resolve` are `None`).

## 5. The host-test mock

There are two layers. Both exist in the tree today; each 7.1 item uses the
one that fits:

1. **The table's decisions, with a fake builder** (the 4.2 pattern,
   `src/v3d/backend_tests.rs:1-25`): a `#[cfg(test)]` module of the table
   (`backend71_tests.rs` through `#[path]`, as `backend.rs:7320-7322`)
   under `--features host-test,gpu-v3d71,shaderc-online,no-shader-table`.
   It defines `v3d_job71_begin`, `v3d_job71_draw`, `v3d_job71_end` and
   `v3d_job71_alloc_sizes` as recording fakes (names distinct from the 4.2
   fakes, so both modules link in one test binary). It installs the
   `Rig`-style `OsalOps` (`backend_tests.rs:1156`: `gpu_alloc` with 32-bit
   addresses, a VPM query, a `submit` recording each `V3dKick`, the F1
   identity query once it exists) under a `TestGl` guard
   (`backend_tests.rs:1268-1297`). It checks what the table hands over: the
   `V3dJob71Target`, each `V3dJob71Draw`, the `V3dJob71Clear`, the kicks,
   the refusal log lines (`logs_with`, `:1382`). Use this for
   `table71-draw`.
2. **The real C builder, out of process**: `tests/v3d71_backend71_shapes.rs`
   (the six shapes through `fill71::fill` and
   `backend71::assemble_filled` against Mesa's
   `tools/v3d/pi5/gl_*.clif`, with the C builder in a child process,
   `tests/v3d71_backend71_harness.c`). It already shows the six cases'
   jobs build with only named differences (`ALLOW`, `:1240-1245`). Use it
   for `table71-cases`, extended to drive the shapes through the table's
   `GlesDraw` mapping (the `glue71` module) instead of its own GL model
   (`:485-640`). Tests are new files only (`tests/v3d71_*.rs`; plan #400
   holds `tests/**` edits).

## 6. The six cases: expected verdicts and paths

Programs: `color_uv` for the draws, the clear program for every clear quad.
Both are pinned 7.1 outputs (`tools/v3d/gltest-shaders71/`, simulated in
`tests/v3d71_gltest_shaders.rs`), so the compiler refuses no case. Line
numbers are `driver/tools/pvrlab/gl_test.c`'s, dispatch `:24064-24181`.

| Case | What it does | Path through the table | Expected |
|---|---|---|---|
| 5 `case_scissor` `:2353` | window; full colour clear, finish; scissor (0,0,32,32), clear, 6-vertex draw, finish | clear: `ClearReq{color, scissor}` → clearplan71 `Draw` (box not whole) → fill71 clear quad; draw: `Op::Draw` with `state.scissor`, `scissor_box` → `V3dJob71State.scissor*` (`fill71.rs:712-714`) → `CLIP_WINDOW` | **PASS** |
| 58 `case_clearscis` `:8101` | three parts: whole clear then scissored clear; clear, draw, scissored clear; zero-width scissor | the scissored clear is a quad (as 5); after a draw it stays in the same job (`route_clear` with draws queued, `Draw` plan has no `new_job`); the zero-width box never reaches the table (the GL core drops it, `draw.rs:2932-2934`) | **PASS** |
| 59 `case_clearmask` `:8173` | window D24S8; colour clear under mask 0 (dropped by the GL core); depth clear under `glDepthMask(FALSE)` (dropped, `draw.rs:2925`); colour+stencil clear, then a stencil clear under mask 0x0f (quad), draw testing `EQUAL 0xff` (must not draw), finish, draw testing `EQUAL 0x0f` | window `ds_persist` → `DsBuffer` Combined D24S8 (section 2.1); the 0x0f clear is a clear quad with stencil REPLACE through the mask (`fill71.rs:880-881`), written plane; job end: zskeep71 stores stencil (`Known71::Stored`); next job (after `glFinish`) loads stencil only (`zs_load_aspects` STENCIL, G1 `v3d71-gl-table.md` section 5) | **PASS**. If `table71-draw` ships without the kept window buffer (`DsBuffer.address` 0, `Layout71::TileOnly`), the second job is refused by name: `"the draw reads stencil an earlier job wrote, and the target has no depth/stencil buffer to keep it in"` (`ZsRefusal71::ReadLost(Stencil)`, `zskeep71.rs:463-478`). That is the named-refusal outcome the plan expects |
| 66 `case_clearvp` `:9240` | full clears under a full and a half viewport, then a scissored clear | a clear ignores the viewport: tile clear (whole) or quad over the whole target with `blitter_clear`'s full viewport (`draw71.rs:1091-1098`); no draw | **PASS** |
| 67 `case_fbods` `:8722` | FBOs: RGBA texture with DEPTH16 renderbuffer (near/far draws), GL_RGB texture frame, two FBOs sharing one colour texture with two DEPTH16 renderbuffers, STENCIL_INDEX8 frames; read back by pixels | each FBO is its own frame (key includes `ds_id`, section 2.1); DEPTH16 kept as D24S8 (F3), STENCIL_INDEX8 as a D24S8 moved as stencil (`fill71.rs:247-250`); GL_RGB → `GL_RGB8`, `force_alpha_1`; an undefined depth plane starts at 1.0 (`zskeep71` `UNDEFINED_Z_BITS`, the far quad wins); readback raster (section 4.3) | **PASS**. The old alpha refusal is gone (section 3). The 7.1 named refusals that could appear instead are the ones `v3d71-refusals.md` section 9 lists (`ReadLost(Stencil)` with the tile-only fallback of case 59's row; `"V3D 7.1: ..."` for a frame the table has no row for) |
| 102 `case_maskclear` `:23057` | full clear; clear under mask R|A; scissored clear under mask G | the masked clear: clearplan71 `Draw` (partial mask), quad with `colormask` = the one mask (`one_mask`, `fill71.rs:585-598`), `COLOR_WRITE_MASKS` in the builder; the scissored+masked one is a quad with the box as vertices | **PASS** |

The tally is **6 of 6** with the kept window/FBO depth-stencil buffer, which
is the design here. **5 of 6** (59 refused by the stencil string) if the
first cut keeps depth/stencil in the tile buffer only. The plan text's
"59 and 67 refused" is the Pi 4's first run (`pi4-v3d-bringup.md:322-335`):
its 67 string no longer exists, and its 59 string was then "stencil is not
kept between jobs yet". Today's 4.2 table keeps the window's D24S8
(`target_zs_bufs`, `backend.rs:2817`; `resolve_ds_storage`,
`src/gl/draw.rs:3073-3085`), so the Pi 4 itself now meets the 59 string only
for an FBO without a depth/stencil attachment
(`backend_tests.rs:6316-6328`). The board session replaces these predictions
with its verdicts. Any refusal text not named in this section or in
`v3d71-refusals.md` section 9 is a defect of the 7.1 path.

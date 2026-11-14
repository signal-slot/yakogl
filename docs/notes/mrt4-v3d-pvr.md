<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 4.2 and PowerVR BXS-4-64: drawing on the fourth colour target — the knob and the board check

Design note of plan #368, item `note`, for the items that follow it (`knob`,
`pvr-render`, `v3d-*`, `pvr-*`, `gltest`, `issues`) and the Pi 4 and AM62P
board sessions. RC14 (`docs/es3-cts-failures.md`, "RC14. MRT: three draw
buffers"): the GL core caps the draw-buffer count at the back end's
`DRAW_TARGETS`, 3 on every GPU (`src/backend.rs`, the `DRAW_TARGETS` const),
where ES 3.0 asks for 4 (Table 6.34). The V3D's TLB and the PowerVR's
end-of-tile programs already reach four; what is missing is the proof that
four targets are *drawn*, and on the PowerVR the fourth render-target slot of
the render layer. Plan #368 builds that on the host behind an opt-in knob and
keeps `DRAW_TARGETS` at 3 until a board draws four. This note gives the words
of the fourth target on each GPU, the knob's shape (the same as plan #335's
for the GC7000UL), the gl_test case, the board procedure with its verdict
lines, and what must hold before `DRAW_TARGETS` flips. **No code comes with
this note, and nothing here was run on a board.**

Sources: Mesa 26.2.1, tag `mesa-26.2.1` (`third_party/mesa-26.2.1/UPSTREAM`,
read only). `third_party/` holds the V3D packet XML
(`src/broadcom/cle/v3d_packet.xml`, cited `v3d_packet.xml`), the PowerVR
csbgen XML (`src/imagination/csbgen/rogue/pbestate.xml`, `pds.xml`, `cr.xml`)
and the PDS generator (`src/imagination/vulkan/pds/pvr_pds.c`). The rest of
Mesa — `gallium/drivers/v3d/v3dx_rcl.c`, `broadcom/compiler/nir_to_vir.c`,
`broadcom/common/v3d_util.c`, and the PowerVR Vulkan driver's
`pvr_arch_cmd_buffer.c`, `pvr_arch_job_common.c`, `pvr_arch_mrt.c`,
`pvr_arch_hw_pass.c` — is not in `third_party/`; as in
`gc-mrt4-board-check.md` it was read from the same release's tree in the deps
cache (`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1/src/`) and is cited with the
26.2.1 line numbers. Own code is cited at the tip `0ab3ca11`. Items running
alongside edit `src/backend.rs`, `src/v3d/backend.rs`, `src/pvr/**` and
`src/gl/**`, so **line numbers there can drift**: own code in those files is
given with its function name, which is the address.

Already written, not repeated: `docs/notes/v3d-fragment-out.md` (the Pi 4
`fragment_out` rows, "Four draw buffers", design piece 4),
`docs/notes/v3d-draw-buffers-indexed.md` (what the V3D has per render target,
section (c)), `docs/notes/v3d-mrt-clear.md`, `docs/notes/v3d-mrt-sizes.md`,
`docs/notes/pvr-int-mixed-targets.md` (the PowerVR mixed-target key and its
back end), `docs/es3/backend-caps.md` (the capability matrix, rows "MAX_DRAW_BUFFERS"
at section 3 and "Real hardware limits", section 4), and
`docs/notes/gc-mrt4-board-check.md` (plan #335's note for the GC7000UL, whose
knob this one copies).

## 0. What is already in the tree, and what is not

| layer | V3D 4.2 | PowerVR BXS-4-64 |
|---|---|---|
| GL core (`GL_MAX_DRAW_BUFFERS`, `GL_MAX_COLOR_ATTACHMENTS`, `gl_MaxDrawBuffers`, attachment checks) | reads `backend::draw_targets()`; `tests/es3_mrt_gl.rs` walks four with `HOST_DRAW_TARGETS = 4` | same |
| job / render layer | four RTs fit: `V3dJobTarget::rts` up to 4 with `rt: [V3dJobRt; 3]` (RT 1-3), `v3d_job_begin` refuses `rts > 4` (`driver/src/v3d/v3d_job.c`), `v3d_job_clear` has `load_rt[3]`, `rgba8_rt[3]`, `clear_rt[4][4]` (`v3d_job.h`) | **three**: `PVR_RENDER_MAX_RT` 3 (`driver/src/pvr_render.h:57`, `src/osal/render.rs:20`, `driver/rust/src/gpu/pvr/render.rs` `MAX_RT`); the EOT programs and the kick's `pbe_word` already reach four (`PVR_RENDER_EOT_MAX_RT` 4) |
| shader compiler | writes four TLB colours: `MAX_RTS` 4 (`src/shaderc/online/v3d/prepare.rs`, `FragColor(rt)` guard) | `pco_shc` and the mixed-key compiler place outputs by `src/pvr/mrt_layout.rs` (up to four); `src/target_key.rs` `SHC_MIXED_MAX_TARGETS` is 3 (plan #361's) |
| back end | `extra_targets` (`src/v3d/backend.rs`) walks up to `GLES_MAX_TARGETS`; clear quad `clear_mrt_source` writes up to `clear_mrt::MAX_RTS` (4) | `PVR_BACKEND_MAX_RT` = `PVR_RENDER_MAX_RT` (`src/pvr/backend.rs`): the draw, the PDS pixel-event data and the emit loop clamp to it |
| proof | tests stand a job builder in (`tests/es3_four_rt_v3d.rs`); **nothing pins four targets through the recorded job words or the read-back** | nothing |
| silicon | no board has drawn four | no board has drawn four |

## 1. (a) V3D 4.2: the words of render target 3

V3D 4.2 has four render targets and no more (`V3D_MAX_RENDER_TARGETS(ver)` is
`ver < 71 ? 4 : 8`, `broadcom/common/v3d_limits.h:44`; Mesa sets
`caps->max_render_targets` from it, `gallium/drivers/v3d/v3d_screen.c:332`).
Four is therefore not new state: it is the fourth element of every per-RT
field the job builder already writes for RT 0-2. Nothing in `v3d_packet.xml`
is conditional on the RT number below 4 on this version. Below, the field in
the packet XML, Mesa's emitter, and ours (`driver/src/v3d/v3d_job.c`, the
product job builder; `driver/src/v3d/v3d_frame.c` is the first cut's
single-RT frame and is not on the GL path).

| word | rt = 3 | Mesa 26.2.1 | ours |
|---|---|---|---|
| `TILE_BINNING_MODE_CFG` number of render targets and max bpp | count 4 (the field is stored minus one) | `v3dx_draw.c` `v3dX(start_binning)` | `v3d_job_begin` (`bin.number_of_render_targets = job_rts(t)`, `bin.maximum_bpp_of_all_render_targets = job_max_bpp(t)`) |
| `TILE_RENDERING_MODE_CFG_COMMON`: number of render targets (bits 4-7, `minus_one`), max bpp (bits 40-41) | 3 stored; max over all four | `v3d_packet.xml:996-1023`; `v3dx_rcl.c:761,768` | `v3d_job_end` (`common.number_of_render_targets`, `common.maximum_bpp_of_all_render_targets = max_bpp`) |
| `TILE_RENDERING_MODE_CFG_COLOR` (sub-id 1): RT 3 bpp bits 28-29, internal type bits 30-33, clamp bits 34-35 | one packet carries RT 0-3 | `v3d_packet.xml:1062-1083`; `v3dx_rcl.c:897-913` (`v3d_setup_render_target(job, 3, ...)`; the RT-type/clamp choice `:396-448`) | `v3d_job_end`, the `if (rts > 3)` block: `color.render_target_3_{internal_bpp,internal_type,clamp}` from `rt_internal(rt_fmt(t, 3))`; RT 3 stays 0 for a 1-3 RT job, as `v3d_setup_render_target` leaves it |
| clear colour of RT 3: `CLEAR_COLORS_PART1` (low 32 + next 24 bits), `PART2` at 64 bpp and up, `PART3` at 128 bpp, each with "Render Target number" = 3 (4-bit field at bit 4) | one to three packets, in RT order | `v3d_packet.xml:1103-1158`; `v3dx_rcl.c:833-860` | `v3d_job_end`: the per-RT loop (`clear.render_target_number = n`; words from `c->clear_rt[n]` when the job is `typed`, else `c->rgba8_rt[n - 1]`); an RT the job loads clears to 0 |
| load of RT 3: `LOAD_TILE_BUFFER_GENERAL`, "Buffer to Load" = 3 | one packet, raster, SAMPLE_0 | `v3d_packet.xml:528-558` (values 0-3; 4-7 exist only on 7.1 and only for the store); `v3dx_rcl.c:195-211` | `emit_tile_list`'s load loop (`for n < job_rts`, `V3D_LOAD_BUFFER_RT0 + n`, address and stride from `t.rt[n - 1]`) |
| store of RT 3: `STORE_TILE_BUFFER_GENERAL`, "Buffer to Store" = 3 | one raster packet, `output_image_format` of the RT | `v3d_packet.xml:491-526`; `v3dx_rcl.c:247-283` | `emit_tile_list`'s store loop `for (n = 1; n < job_rts; n++)` calling `store_rt` (`V3D_STORE_BUFFER_RT0 + n`; an RT with address 0, a `GL_NONE` buffer, is configured but never stored) |
| blend enables / colour write masks (4 RTs on 4.2) | bit 3 of `BLEND_ENABLES`; 4 bits at 12-15 of `COLOR_WRITE_MASKS` | `v3dx_emit.c` `v3dX(emit_state)` (`BLEND_ENABLES` at `:529,565`, `COLOR_WRITE_MASKS` at `:576`); `docs/notes/v3d-draw-buffers-indexed.md` (c) | the job state (`state::job_state`, `v3d_job.c` blend-enable comment "(1 << max_rts) - 1, max_rts 4") |

**The fragment shader's TLB writes.** Mesa writes each colour output in RT
order, `for (rt = 0; rt < V3D_MAX_DRAW_BUFFERS; rt++) vir_emit_tlb_color_write(c, rt)`
(`broadcom/compiler/nir_to_vir.c:2140-2141`); a write is skipped when the
draw's `cbufs` bit or the output variable is absent (`:1953-1955`). The TLB
specifier word is `0xffffff00 | sample mode (1<<2, per pixel) | (7 - rt) << 3
| type | vector size` (`:1957-1980`; `TLB_RENDER_TARGET_SHIFT` 3, "Reversed!
7 = RT 0, 0 = RT 7", `:1932`, applied at `:1968`). For RT 3 the target field is 4 << 3 = 0x20:

| RT 3 output | specifier | notes |
|---|---|---|
| f16 target, 3-4 components (`TLB_TYPE_F16_COLOR` 3<<6, `SWAP_HI_LO` 1<<1, `VEC_SIZE_4_F16` 1) | `0xffffffe7` | RT 0's default is `0xffffffff` and goes through TLB, not TLBU; every other RT's config is not the default and the first word goes through TLBU |
| f16 target, 1-2 components | `0xffffffe6` | |
| 32-bit class (`TLB_TYPE_F32_COLOR` 0, pure integer, R32F, 16-bit norm), n components | `0xffffff20 \| 0x04 \| (n - 1)` = `0xffffff24 + (n - 1)` | the integer path (`rt_int`) |

Ours: `tlb_config_f16(rt, components)` and `tlb_config_32b(rt, components)` in
`src/shaderc/online/v3d/select.rs` build `(7 - rt) << TLB_RENDER_TARGET_SHIFT`
with `rt` a parameter, and the colour loop of the fragment end calls them for
`rt` 0..`MAX_RTS`; `prepare.rs` (`color: [[S; 4]; MAX_RTS]`, `rt_comps`) holds
four. So **a fourth output is already compiled**; what a host test must add is
the *words*: that `tlb_config_*(3, n)` equal the table above and that a
four-output program writes them in RT order. Item `v3d-shout4` pins that.

**The tile size.** `v3d_choose_tile_size` on V3D 4.x
(`broadcom/common/v3d_util.c:185-199`) is `idx = (n > 4 ? 3 : n > 2 ? 2 : n > 1
? 1 : 0) + (msaa ? 2 : double_buffer ? 1 : 0) + max_internal_bpp` into
`{64x64, 64x32, 32x32, 32x16, 16x16, 16x8, 8x8}` (`:133-140`). Four colour
attachments take **the same index as three** (`> 2`), so four RTs cost no
tile-size step: 32x32 at 32 bpp, 32x16 at 64 bpp, 16x16 at 128 bpp. Ours is
the same expression, `v3d_frame_tiling_gen` (`driver/src/v3d/v3d_frame.c:89`,
`idx = (rts > 2 ? 2 : rts > 1 ? 1 : 0) + (msaa ? 2 : 0) + (max_bpp > 2 ? 2 :
max_bpp)`), called with `job_rts` from `v3d_job_end` and with
`target.rts.max(1)` from the back end (`src/v3d/backend.rs`, the
`v3d_frame_tiling_bpp` calls in the job-start code). Mesa counts to the last
non-null colour buffer (`v3d_context.c:298`, `max_cbuf_idx + 1`); we count the
draw's slot count, hole included — the same value for any count above two.
No 4x with several colour attachments: `draw_call` refuses it by name
("4x multisampling of several colour attachments") and an FBO is
single-sampled.

**The back-end side.** `extra_targets` (`src/v3d/backend.rs`) returns `(rts,
rt: [u32; 3], formats: [u8; 3])` for slots 1..3 and walks up to
`GLES_MAX_TARGETS`; `Target::rts` is a `u8`, `Target::rt[2]` is RT 3's
address; the refusals a four-target draw can reach are listed in
`docs/notes/v3d-fragment-out.md` "Design", piece 4 ("a colour output with no
attachment slot" — `fs.v3d.fs.rt_mask >> slots`, which passes for 0b1111 at
`rts` 4; "colour attachments of different sizes"; the 4x one). The clear:
`mrt_pattern` and `draw_clear_quad` (`src/v3d/backend.rs`) take `rts` up to
`clear_mrt::MAX_RTS` and name RTs by `clear_target_mask` bit; `clear_mrt_source`
(`src/v3d/clear_mrt.rs`) writes up to four `u_color<rt>` uniforms.

**What the host test (`tests/es3_mrt4_v3d.rs`, item `v3d-mrt4`) pins.** The
GL-level four-target walk exists (`tests/es3_four_rt_v3d.rs`, with a stand-in
job builder), so the new proof is the layer below it, through the real
`v3d_job_begin`/`v3d_job_end` and the recorded control lists, with the knob
on: (1) the common and COLOR packets with RT count 4 and RT 3's bpp, type
and clamp; (2) four `CLEAR_COLORS_PART1` packets numbered 0-3 (and `PART2`
for a 64-bpp RT 3); (3) four loads and four stores of buffers 0-3 with
the addresses and strides of `t.rt[n - 1]`, a `GL_NONE` slot 2 storing three;
(4) the tile size for 4 RTs at each bpp (it equals the 3-RT size); (5) the
fragment program's four TLB specifiers; (6) a `glClear`/`glClearBuffer*` of
buffer 3 through `clear_mrt_source`; (7) a draw and a `glReadPixels` of
`GL_COLOR_ATTACHMENT3` through the GL API. The test sets the knob of section 3
before creating the context, since `gl_MaxDrawBuffers` is baked at compile
time. Not measurable on the host: that the TLB stores RT 3 to memory (the
simulator reference is item `v3d-sim4`'s; silicon is the board's).

## 2. (b) PowerVR BXS-4-64: the fourth PBE state, the end-of-tile program and the output registers

### 2.1 What a render target is to the hardware

Each colour attachment that the end-of-tile (EOT) program emits has one **PBE
emit state**, in two forms, both written per render:

* **PBESTATE_STATE_WORD0/1** (`pbestate.xml:226-243`): WORD0 is the surface
  address low (`address_low`, bits 0-31, `shift 2`); WORD1 holds `address_high`
  (bits 0-5), `emptytile` (6), `packmode` (7-12), `norm` (13), `source_pos`
  (14-15, the output register the emit reads: 0, 32, 64, 96 bit), `mrt_index`
  (16-18, the emit's index, 0-7), `source_format` (19), and
  `source_pos_offset_128` (28, **only on cores with
  `eight_output_registers`** — `gx6250.h:42,153`, `gx6650.h:25`; the
  BXS-4-64 does not have it, `common/device_info/bxs-4-64.h`). Mesa packs
  them in `pvr_arch_pbe_pack_state` (`pvr_arch_job_common.c:82-135`), with
  `mrt_index` the emit's index (`pvr_arch_cmd_buffer.c` `pvr_setup_pbe_state`,
  the `render_params.mrt_index = mrt_index` store) and `source_pos` from the
  register position (the `switch (position)` there: 0 → `BIT0`, 1 → `BIT32`, 2
  → `BIT64`, 3 → `BIT96`). Those words go into the **PDS DATA segment**
  (`pvr_sub_cmd_gfx_per_job_fragment_programs_create_and_upload`,
  `pvr_arch_cmd_buffer.c`), which the fixed PDS CODE moves into the **shared
  registers** the EOT USC program reads, two per emit: emit i reads shared
  registers 2i and 2i+1.
* **PBESTATE_REG_WORD0/1/2** (`pbestate.xml:245-292`): the layout/size/clip
  words (`memlayout`, `linestride`, the swizzles `swiz_chan0-3`, `gamma`,
  `downscale`, `minclip_x`, `size_x/y`, `maxclip_x/y`, `zslice`). These go to
  the **firmware's 3D command**, `pbe_word[8][3]` (Mesa's `pbe_reg_words`
  array, `pvr_job_render.h:136`). Word 2 is 0 in Mesa
  (`pvr_arch_job_common.c:96`) and in ours.

Ours: `pbe_emit_words(addr, fmt, source_pos, mrt_index)` builds the STATE
words (it masks `mrt_index & 7`, so index 3 is just 3), `pbe_reg_words(...)`
the REG words (`src/pvr/backend.rs`); the back end's frame close
(the code around the `pvr_pds_generate_pixel_event_tq` call and the
`b.kick.frag.pbe_word[rt]` stores) fills them per emit. The firmware command
takes the REG pairs at dword `30 + 4 * i` (`PBE_STRIDE_DW` 4, "two PBE
register words per colour attachment ... four dwords apart, measured on the
board", `driver/rust/src/gpu/pvr/render.rs`), clamped to `EOT_MAX_RT` (the
`nrt` of the kick-building function): **emit 3's REG pair is dwords 42-45**,
already copied (the host test of the kick pins `30..30 + EOT_MAX_RT *
PBE_STRIDE_DW`).

### 2.2 The end-of-tile selection, and what `PVR_RENDER_MAX_RT` 3 still stops

`eot1`..`eot4` are captured USC programs (`driver/src/pvr_usc_eot_blob.h`,
`tools/pco_shc/usc_blobs.txt`), uploaded for every count 1..`PVR_RENDER_EOT_MAX_RT`
(`pvr_render.c`, the `eot_n[n]` loop; `eot_tb_n[n]` and `eot_tb4_k[k]` for the
tile-buffer forms). `pvr_render_static_get` hands the back end
`eot_doutu_src0_n[n]` and `event_pixel_pds_info_n[n]` for n up to four, and
the back end picks the program by the emitted count
(`event_pixel_pds_info_n[nrt]`, `eot_doutu_src0_n[nrt]`, `tb_n`/`tb4_k` for a
spilled layout; `src/pvr/mrt_layout.rs` `EotProgram::doutu`/`pds_info`). So
the four-emit program, its DOUTU and its REG words exist. Three things still
carry the count three:

1. **The fixed PDS CODE.** The pixel-event PDS code sits at a heap offset the
   firmware runs for every render (`PVR_RENDER_EOT_PDS_CODE_OFF` 0x80,
   `pvr_render.h`), so it is generated once, for `PVR_RENDER_MAX_RT` pairs
   (`pvr_render.c`, the block after `eot_tb_upload`:
   `pvr_pds_generate_pixel_event_tq(R.eot_doutu, emit, PVR_RENDER_MAX_RT, ...)`),
   and "a program that emits fewer simply does not read the rest". The
   comment at `pvr_render_static_get` says it outright: *a four-target
   program reads a fourth pair that program does not move yet*. The generator
   emits constants for `2 * N` emit words and `N` control words
   (`pvr_pds.c:549-560`) and a code of `1 + N` instructions (`:610`), so N =
   4 moves shared registers 6 and 7 as well.
2. **The info word.** `event_pixel_pds_info_n[n]` is
   `pvr_cr_event_pixel_pds_info(PVR_RENDER_MAX_RT)` for every n
   (`pvr_render.c:508`; `pvr_render_regs.c:22-37`): `const_size` from the
   generator's data size (unit 4, `cr.h:1200`) and `usc_sr_size` = `2N` shared
   registers rounded to a unit of 16 (`cr.h:1194`) — one unit for both N = 3
   and N = 4.
3. **The arrays.** `PVR_BACKEND_MAX_RT` and the per-render arrays sized from it
   in `src/pvr/backend.rs` (`emit`, `ed`/`ec`, `slot_of`, `PR_EOT_DATA_MAX_DW`
   = `3 * N + 2`, the `targets` array of `GlesDrawTarget`, the clamp
   `num_targets.min(PVR_BACKEND_MAX_RT)` in the frame-open function, and the
   `(2..=PVR_BACKEND_MAX_RT).contains(&n)` mixed-key range of the output
   placement), and the render layer's own (`pvr_render.c`'s `emit[2 *
   PVR_RENDER_MAX_RT]`, `d[3 * ... + 2]`, `c[... + 2]`; the partial render's
   RTA arrays of `MAX_RT` dwords and `RTA_CTL_MAX_RTS`, `driver/rust/src/gpu/
   pvr/render.rs`, written only with the partial-render knob).

**The change** (item `pvr-render`): `PVR_RENDER_MAX_RT` goes **3 → 4** in the
three places, `driver/src/pvr_render.h:57`, `src/osal/render.rs:20` and
`driver/rust/src/gpu/pvr/render.rs` (`MAX_RT`); `PVR_RENDER_EOT_MAX_RT` was
already 4 (after it the two are equal, and the comment "one more than a draw
writes yet" goes). The static PDS CODE, the info words and the arrays then
follow. The back end's clamp follows `PVR_RENDER_MAX_RT` automatically
(`PVR_BACKEND_MAX_RT`), so `draw_targets()` raising to 4 on the PowerVR is the
*knob's* clamp (section 3), not a back-end edit.

**What "1-3 target kicks byte-identical" can and cannot cover.** The plan
asks that kicks of one to three targets stay byte-identical. Three groups of
words must be told apart, because the static PDS code is shared by *every*
render:

* **Identical by construction**: the kick's `pbe_word[0..2]` and `num_rt`, the
  per-attachment STATE words and their `source_pos`/`mrt_index`, the DOUTU of
  `eot_doutu_src0_n[1..3]`, the whole 1-3-emit placement (`mrt_layout::layout`
  gives the same places for the same dwords), `frag.event_pixel_pds_data` for a
  given heap layout.
* **Not identical, by design**: the PDS CODE at 0x80 (one more instruction and
  two more constants), the **PDS DATA segment** (the generator lays out
  "the 64-bit words together followed by the 32-bit words", `pvr_pds.c:545-547`,
  so the offsets of every constant depend on N, not just the length), and
  `event_pixel_pds_info_n[*]`, whose `const_size` is computed from that data
  size and may cross a unit boundary from 3 to 4. These change for **every**
  render of the PowerVR build, knob on or off, the moment the constant moves.
* **Moves only with the partial-render knob**: the HWRT data's RTA arrays
  (`[0,1,2]` → `[0,1,2,3]`, `RTA_CTL_MAX_RTS` 3 → 4).

The note's recommendation, for the planner to confirm: take the change, and
have `tests/pvr_render_rt4.rs` pin the *new* static words explicitly (the
code, data and info of N = 4) and the *unchanged* per-kick words of N = 1..3;
and **require a board regression of the three-target path (gl_test 106 `mrt3`,
the regression set 7, and one tile-buffer layout — `eot3_tb`, three RGBA16F-class targets)
on the AM62P before the PowerVR knob is relied on**, because the product
build's PDS words move with the constant and no host test can run the PDS.
Two alternatives, both worse: (B) keep `event_pixel_pds_info_n[n <= 3]` at the
three-pair value while the code moves four — the info word's `const_size`
would then cut the data segment before the fourth pair's constants, which the
fixed code reads into shared registers 6-7; harmless if the PDS tolerates it
and unknown if it does not; (C) generate the four-pair code only when the
knob asks, which needs a render-layer entry point the knob can reach before
`pvr_render_init`, so a product build stays byte-identical, at the cost of a
second code path nobody ships. B and C are listed so the choice is made, not
inherited.

### 2.3 The USC output-register packing: four registers a pixel

The BXS-4-64 has **four 32-bit output registers per pixel** (`
rogue_get_max_output_regs_per_pixel` returns 8 only with
`eight_output_registers`, 4 otherwise: `include/hwdef/rogue_hw_utils.h`,
quoted in `docs/es3/backend-caps.md` section 3 as `:189-195`;
`usc_min_output_registers_per_pix` 2, `bxs-4-64.h:104`). Mesa's allocator
places each attachment at the next free register run of `pixel_size` dwords
and, for a 64- or 128-bit source on a core with `pbe2_in_xe` (the BXS-4-64
has it, `bxs-4-64.h:53`), at an **even** register
(`pvr_arch_mrt.c:55-82`, the same rule in `pvr_arch_hw_pass.c:370-392`); one
that does not fit goes to a tile buffer (`USC_MRT_RESOURCE_TYPE_MEMORY`,
`pvr_arch_mrt.c:137-195`) and the EOT program loads it back. Mesa's Vulkan
driver advertises `maxColorAttachments` 4 on this core
(`pvr_physical_device.c:739`). This is exactly the rule `src/pvr/mrt_layout.rs`
("layout B") implements and the compiler and the back end share:

* a target takes its format's dwords (`surf_out_dwords`, `src/pvr/backend.rs`:
  8-bit-class, R32F and the 32-bit and `RG16UI` integers **1**, RGBA16F, sRGB
  and RGBA16UI **2**, RGBA32UI **4**); two or more dwords start at an even
  register; registers are never back-filled;
* the first target that does not fit, and **every one after it**, goes to a
  tile buffer (the k-th spilled to buffer k, from dword 0, `source_pos` 0);
  target 0 always fits, so at most three spill (`PVR_RENDER_TILE_BUFFERS` 3).

For four targets that gives, with `layout(dwords)` (the module's own unit
tests pin these):

| four targets (output dwords) | places | EOT program | static slot |
|---|---|---|---|
| 1,1,1,1 (four RGBA8-class) | r0 r1 r2 r3 | `eot4` | `eot_doutu_src0_n[4]` |
| 1,1,2,1 / 1,1,1,2 (RGBA8 ×2 + RGBA16F, any 8-bit-class after) | the 64-bit one at r2, the fourth target spills | `eot4_tb` | `eot_doutu_src0_tb4_k[1]` |
| 2,2,1,1 / 2,2,2,2 (two RGBA16F-class first) | r0, r2, then tile buffers 0, 1 | `eot4_tb2` | `eot_doutu_src0_tb4_k[2]` |
| 4,1,1,1 / 1,4,x,x (an RGBA32UI first) | r0 (or later), the rest in buffers 0..2 | `eot4_tb3` | `eot_doutu_src0_tb4_k[3]` |
| **refused** (`LayoutError::NoProgram`) | two targets whose second spills (4,1 / 3,2) and three targets whose last two spill (4,1,1): no EOT program was captured for those shapes | none | none |

**Which four-target mixes fit.** *In registers alone* (no spill, `eot4`): four
targets of one dword each. Anything with a 64-bit target among four
spills at least one, because five dwords do not fit four registers. *With tile buffers*: any mix the table's first four rows
reach, i.e. **every four-target layout except the two refused shapes above**
(a four-target layout can only produce 0, 1, 2 or 3 spilled; all four are
captured). The refusals are shapes of *two* or *three* targets, so a
four-target draw is never refused by the layout rule; it is refused by the
back end's own limits (`src/pvr/backend.rs`): today the emit loop lets **only
the last attachment** come from a tile buffer (the comment above the
`spos + regs > PVR_BACKEND_MAX_OUT_REGS` test: "the compiler spills in output
order, and one program per count is captured") and breaks out ("not emitted",
logged once) otherwise — i.e. `eot4_tb2` and `eot4_tb3` are captured and
selectable by `mrt_layout` but **not yet wired into the emit loop**; item
`pvr-pack`/`pvr-backend` takes that. Integer and sRGB mixes keep their own
refusals (`docs/notes/pvr-int-mixed-targets.md` sections 5-6: a `GL_NONE`
slot between an integer target and another, sRGB in a mixed key, `SHC_MIXED_MAX_TARGETS` 3 in
`src/target_key.rs`, which plan #361 owns and which must reach four before a
mixed-class four-target draw can be keyed — `pvr-int-mixed-targets.md` §2.3
reserves the fourth slot at bits [26:30]); a plain RGBA8 set with a `GL_NONE`
hole is not a mix and works the way three-target case 106 does (the emit
loop skips the empty slot: `slot_of`).

**The fourth state in one line**: emit 3 = `mrt_index` 3, `source_pos` =
`mrt_layout`'s place (register 3 for the all-RGBA8 layout; 0 for a tile-buffer
emit), PDS pair 3 (shared registers 6-7), `pbe_word[3]` at kick dwords
42-45. Four *tile-buffer-spilled* 32-bpp targets also need the tile buffers:
`PVR_RENDER_TILE_BUFFERS` 3, one tile's worth each (`PVR_TILE_BUFFER_BYTES`),
already allocated by `pvr_render_init`.

## 3. (c) The knob, and where it joins (nothing here is edited)

Plan #335's shape (`gc-mrt4-board-check.md` section 3), for all three GPUs:
**no new Cargo feature**; a runtime knob behind the existing `test-knobs`
feature (`Cargo.toml`, the `test-knobs` comment block), set before the
context is created, with a C setter.

* **The shared static.** `cfg(feature = "test-knobs") static
  backend::TEST_DRAW_TARGETS` in `src/backend.rs`, an atomic `u8` like
  `HOST_DRAW_TARGETS`, feeds `draw_targets()` **after** `HOST_DRAW_TARGETS`:
  non-zero, clamped to `GLES_MAX_TARGETS`, is what `GL_MAX_DRAW_BUFFERS`,
  `GL_MAX_COLOR_ATTACHMENTS`, the attachment checks and `gl_MaxDrawBuffers`
  read. #335's `gc_debug_set_draw_targets` can write the same static. It must
  be set **before context creation**: `gl_MaxDrawBuffers` is baked at compile
  time (`src/shaderc/online/lower.rs`), so a later change speaks only to
  programs compiled after it. Without `test-knobs` the static does not exist
  and `draw_targets()` is the `DRAW_TARGETS` const as today.
* **The per-GPU C setters and statics**, in the `v3d_debug_*` / `pvr_debug_*`
  namespaces, in new files `src/v3d/knob.rs` and `src/pvr/knob.rs`:
  `int v3d_debug_set_draw_targets(int n)` and `int pvr_debug_set_draw_targets(int n)`,
  with `#[no_mangle] static mut v3d_debug_draw_targets: i32` and
  `pvr_debug_draw_targets: i32` (the shape of `pvr_debug_narrow_la` /
  `pvr_debug_set_narrow_la`, `src/pvr/backend.rs`, whose setter returns the
  previous value; this note assumes the new setters do too, and the item may
  say otherwise). **0 is the product answer** (the knob leaves
  `DRAW_TARGETS` alone); any other value is **clamped to what the back end
  draws** — 4 on the V3D, 4 on the PowerVR once `pvr-render`/`pvr-backend`
  land, and a value of 1 or 2 is honoured (a *lower* count, useful for the
  test's own sweep); a value above the back end's clamp is clamped, never
  refused. The setter writes the static and the shared `TEST_DRAW_TARGETS`.
* **The image knobs.** `CONFIG_V3D_GL_TEST_MRT4` in the Pi 4 menu and
  `CONFIG_PVR_GL_TEST_MRT4` in the AM62P menu of
  `driver/samples/zephyr/gl_test/Kconfig` (both `bool`, `default n`,
  `depends on YAKOGL_TEST_KNOBS`; the `V3D_GL_TEST_MSAA` block is the model: a
  startup knob whose help names the measurement and says products leave it
  off). `src/main_rpi4.c` calls `v3d_debug_set_draw_targets(4)` in a block
  beside the `v3d_debug_set_msaa` one (`#ifdef CONFIG_V3D_GL_TEST_MRT4`), and
  `src/main_am62p.c` calls `pvr_debug_set_draw_targets(4)` beside the
  `pvr_backend_set_option` line, each **before `gl_test_run`** and before any
  GL context exists. Nothing in `boards/*.conf`: the symbols default n and an
  image turns them on with `-DCONFIG_V3D_GL_TEST_MRT4=y` /
  `-DCONFIG_PVR_GL_TEST_MRT4=y`.

Where the knob joins once item `knob` lands it (this note edits none of it):

* **`tools/check.sh`, the "no test knob in the artefact" row**
  (`check_knobs_absent`, `knob_sets`, the row at its end, `scope pvr vivante`):
  `KNOB_POWERVR_SYMBOLS` (today `pvr_debug_narrow_la pvr_debug_set_narrow_la
  pvr_debug_set_split_zs pvr_debug_split_zs`) gains `pvr_debug_draw_targets
  pvr_debug_set_draw_targets`. There is **no V3D list today**: add
  `KNOB_V3D_SYMBOLS="v3d_debug_draw_targets v3d_debug_set_draw_targets"` and a
  third `knob_sets v3d '<pattern>' "$KNOB_V3D_SYMBOLS"` line, with `scope pvr
  vivante v3d` on the row (the `gpu-v3d` Zephyr archive is built the same way,
  `base="osal-zephyr,gpu-v3d,shaderc-online,no-shader-table"`). **The pattern
  must not be `^(v3d_debug_|gl_test_)`**: `v3d_debug_msaa` and
  `v3d_debug_set_msaa` are defined **without** `cfg(test-knobs)`
  (`src/v3d/backend.rs`, `#[no_mangle] pub static mut v3d_debug_msaa`), so the
  product archive defines them today and a broad pattern would turn the row
  red. Use `'^v3d_debug_(set_)?draw_targets$'`, or fold the two msaa symbols
  into the expected set of both archives; the first is the smaller change.
* **Host rows with the knob on**: `tests/es3_mrt4_v3d.rs` and
  `tests/es3_mrt4_pvr.rs`, `cargo test --test <name>` with
  `host-test,gpu-v3d,shaderc-online,no-shader-table,test-knobs` (the V3D set,
  `V3D_CONFIG` + `test-knobs`) and `host-test,gpu-powervr,shaderc-online,test-knobs`
  (`KNOBS_POWERVR_CONFIG`); each is a row of its own in `check.sh` (the way
  `tests/gc_tiled_rt.rs` is a `knobs_file_tests` row), so a file that stops
  compiling turns the gate red. Any `--lib` unit test of the setter joins
  `KNOBS_POWERVR_TESTS` by name (`knobs_lib_tests`); a V3D equivalent list is
  not needed unless a lib test is added.
* **Pinned unchanged without the knob** (item `default-pin`): a default
  build and a `report-es2` build answer 3 for `GL_MAX_DRAW_BUFFERS` and
  `GL_MAX_COLOR_ATTACHMENTS`; `tests/es2_report_guard.rs` already pins the ES 2.0
  refusals of `glDrawBuffers`.
* **`Cargo.toml`**: the `test-knobs` comment block lists every knob of every
  GPU; the three draw-target knobs join it. No new crate feature, for the
  reason in #335's note: a feature that flipped the `DRAW_TARGETS` const would
  put a test branch into the product build's one read of it.

## 4. (d) The board procedure and the verdict lines

The environment (images, board addresses, consoles) belongs to the recipes,
not to this note: `docs/es3/board-recipe-pi4.md` for the Pi 4, and the
AM62P's boot notes (`docs/notes/am62p-direct-boot.md`,
`docs/notes/am62p-netboot.md`) and `tools/cts/am62p-deqp-images.sh` for the
AM62P. No address, IP, port or UART path goes into a file or a commit. **One
GL client runs per board at a time**, and **every image feeds the board's
watchdog for a bounded time** (the Pi 4's `CONFIG_V3D_GL_TEST_HOLD_S`, default
200 s, `rpi_wdog` resets the board that long after boot; an AM62P image ends
through `pvr_sample_finish`), so a hung run ends by itself and one boot
answers one question.

### 4.1 The gl_test case 245 `mrt4`

Plan #335's `case_mrt4` (`driver/tools/pvrlab/gl_test.c`; `gc-mrt4-board-check.md`
section 2 gives its body and C sketch, as case 222). At the tip the name table
(`case_names[]`, ends at `[243] = "divmix"`) and the dispatch switch end at
243 and **222 is taken (`dtpu`)**, so #335's case carries the number 245 in
this plan; it does not exist in the tree yet, and this plan adds nothing to
it. What the case draws, for these two GPUs: four 64x64 `GL_RGBA8` texture
attachments, `glDrawBuffers(4, ATT0..ATT3)`; a four-output fragment shader
(`o0..o3` = red, green, blue, yellow); and four parts read back at the
centre through `glReadBuffer` between the stages — (1) a clear to grey
reaches all four, (2) one draw: red, green, blue, **yellow in attachment 3 —
the board's proof**, (3) `{ATT0, ATT1, NONE, ATT3}` cleared to black, 0, 1, 3
black and 2 still blue, (4) the same draw again. The one recorded check is the
fail mask `expect_int("mrt4 fail mask (...)", fail, 0)` (bits 0-3 the clear,
4-7 the draw, 8-11 the `GL_NONE`-slot draw, 12-15 the `GL_NONE`-slot clear);
a `GL_MAX_DRAW_BUFFERS` below 4 returns the skip code (`-3`,
`RC_SKIPPED`, `driver/tools/pvrlab/runner/src/lib.rs`) as case 106 does for 3.
RGBA8 on both GPUs: on the PowerVR it is four one-dword targets, layout 1,1,1,1,
`eot4`, no tile buffer (section 2.3), so the case proves the *plain* fourth
emit and nothing about the spilled forms; a second PowerVR check on a spilled
layout (e.g. RGBA8, RGBA8, RGBA16F, RGBA8: `eot4_tb`) is a follow-up of the
`pvr-backend` item, not of this case.

The case body compiles in every image and is inert without the knob; the
capability, not the case, is what the opt-in builds. Lines to read, per
check (`expect_int`, `gl_test.c`): `gl_test:   mrt4 fail mask (…)  = 0  want 0
ok  [check N]`, plus one `gl_test:   mrt4 clear|draw|hole clear|hole draw
attachment N = 0x… want 0x…` line per attachment per part (the case-106
shape).

### 4.2 Pi 4 (V3D 4.2) — issue #313

1. Build (the Pi 4 recipe's environment): `west build -b rpi_4b
   driver/samples/zephyr/gl_test -- -DCONFIG_V3D_GL_TEST_MRT4=y
   -DCONFIG_V3D_GL_TEST_CASE=245 -DCONFIG_V3D_GL_TEST_RECORD=y` — the ES 3.0
   shader set (the default), the frozen record on (`main_rpi4.c`, the screen
   record at the end of the run), the watchdog hold as built.
2. Boot, run once. Read in this order: the case's check lines (above); the
   per-case verdict line `v3d_gl_test: case 245 PASS` / `FAIL` / `SKIP`
   (`verdicts()` in `main_rpi4.c`, `rc_name`: `PASS`, `FAIL`, `SKIP`,
   `TIMEOUT`, `no such case`); the summary `v3d_gl_test: 1 of 1 cases passed
   (N ms)` — a `SKIP` is counted as neither (`counted` skips `-3`), so a
   skipped run prints `0 of 0`, which is not a pass. An empty run prints
   `v3d_gl_test: FAIL (no case ran; gl_test_run returned N)`.
3. **Verdict.** `PASS`: `mrt4 fail mask = 0 … ok`, the framebuffer complete,
   `glDrawBuffers(4, …)` set no GL error, no job faulted or timed out.
   `SKIP`: `gl_test: case 245 mrt4: SKIP (GL_MAX_DRAW_BUFFERS N < 4)` — the
   image did not ask for four (rebuild with the knob) or the library clamped
   the request. `FAIL` reading guide: attachment 3 black where grey is wanted
   with 0-2 fine (bit 3 of the clear group): the fourth RT never reached the
   TLB — suspect the COLOR packet's RT 3 field or the store packet for buffer
   3 (§1); attachment 3 holding another output's colour after the draw: the
   fragment's TLB specifier for RT 3 (`(7 - 3) << 3`, §1); parts 1-2 right and
   3-4 wrong on 0, 1, 3: the `GL_NONE`-slot compaction (RT 2 configured, never
   stored); part 1 only wrong: the clear quad (`clear_mrt_source`'s fourth
   uniform).
4. Then the dEQP probes (below), one image, one run.

### 4.3 AM62P (PowerVR BXS-4-64) — issue #316 or #320

1. Build: `west build -b verdin_am62p/am62p54/a53
   driver/samples/zephyr/gl_test -- -DCONFIG_PVR_GL_TEST_MRT4=y
   -DCONFIG_GL_TEST_CASE=245` (the AM62P menu's case symbol; with the ES 3.0
   shader set). This build also carries the `pvr-render` change to the static
   PDS words (§2.2): **run case 106 `mrt3` and the regression set first in the
   same image or the one before it**, so a regression of the three-target path
   is attributed to the constant and not to the fourth target.
2. Boot, run once. Read: the check lines (above); the case line `gl_test: case
   245 mrt4: PASS|FAIL` (the harness's per-case line, `gl_test.c` header);
   a skip prints `gl_test: case 245 mrt4: SKIP (GL_MAX_DRAW_BUFFERS N < 4)`; and
   the image's close, `gl_test: OK (rc=0)` or `gl_test: FAILED (rc=N)`
   (`main_am62p.c`) — a skip-only run prints `OK`, so the case line, not the
   close, is the verdict.
3. **Verdict.** `PASS` as 4.2. `FAIL` reading guide: attachment 3 unchanged
   (grey missing) with the rest fine: the fourth emit — `mrt_index` 3 /
   `source_pos` 3 in STATE word 1, PDS pair 3 (shared registers 6-7), the
   `pbe_word[3]` pair at kick dwords 42-45, or the `eot4` DOUTU; **a GPU fault
   or `TIMEOUT` (`-4`) after the draw**: an emit reading an unprepared PDS
   pair (a 4-pair program against a 3-pair static code — §2.2), or an output
   register past the fourth, which "faults the GPU" (the emit-loop comment in
   `src/pvr/backend.rs`); the hold/diag record names the stall. Attachments
   0-2 wrong where case 106 passes: the static-code change, not the fourth
   target.

### 4.4 The dEQP probes, both boards

`dEQP-GLES3.functional.implementation_limits.max_draw_buffers`,
`…max_color_attachments`, `functional.draw_buffers_indexed.*` (76 rows today,
all `NotSupported`; 66 can run, `v3d-draw-buffers-indexed.md` table),
`functional.fragment_out.*` (the 26 G5 rows and G3's four-target share,
`v3d-fragment-out.md`). Today's rows: Pi 4 `docs/es3/pi4-gles3-fails.tsv`
(`A1`: `Fail …max_color_attachments`, `Fail …max_draw_buffers`), AM62P
`docs/es3/am62p-gles3-notpass.tsv` (`Fail …max_draw_buffers  Requirement not
satisfied`). Lines are `deqp: Pass|Fail|NotSupported|… <case>`
(`tools/cts/deqp-uart-tally.py`), one per case.

The dEQP sample selects `YAKOGL_TEST_KNOBS` and "sets none of them"
(`driver/samples/zephyr/deqp/Kconfig`), so **it has no draw-target knob in
this plan's items**: the probe image either (a) is built from a scratch tree
with `DRAW_TARGETS = 4` in `src/backend.rs` for that GPU (not merged;
#335's route), or (b) gets the same two `CONFIG_*_MRT4` symbols and calls in
`driver/samples/zephyr/deqp` (`main.cpp`), a small follow-up to the
`gltest` item, which the planner should add. Expected, with four drawn:
`deqp: Pass dEQP-GLES3.functional.implementation_limits.max_draw_buffers` and
`…max_color_attachments`; `draw_buffers_indexed.*` stay `NotSupported` until
`GL_EXT_draw_buffers_indexed` is claimed (a different item: the extension, not
the count — the `random.max_required_draw_buffers` rows need both); and the
`fragment_out` random rows with a fourth attachment move from `Fail`
(`INVALID_ENUM` at `es3fFragmentOutputTests.cpp:286`) to `Pass` or to
`NotSupported`/`Fail` for their *other* cause (G1-G4's float formats,
`v3d-fragment-out.md`). **A probe line is evidence about the count, not about
the formats**: do not read a remaining `fragment_out` failure as a fourth-target
defect without checking its format against G1-G4.

## 5. What must hold before DRAW_TARGETS goes to 4

Per GPU, the arm of `src/backend.rs` (`DRAW_TARGETS`) alone; the three move
independently (GC7000UL is plan #335's).

1. **A board drew four.** Case 245 `PASS` on that GPU, with the commit and
   image recorded; the Pi 4 / AM62P `RESULTS.md` row and `STATUS.md` summary
   updated by the board session, and a comment on the board issue (#313 for
   the Pi 4, #316 or #320 for the AM62P) with the record, as CLAUDE.md asks.
2. **The dEQP count probes come out better, not worse**: `max_draw_buffers`
   and `max_color_attachments` `Pass`, and no row that passed before turns
   `Fail` (the `fbo.*`, `state_query.*`, `shaders.*` rows that read
   `GL_MAX_DRAW_BUFFERS` — `state_query.integers.max_draw_buffers_*` and
   `max_color_attachments_*` should turn `Pass` too).
3. **The host gate is green against the current main tip**: `tests/es3_mrt_gl.rs`
   (GL core at four), the new `tests/es3_mrt4_v3d.rs` / `tests/es3_mrt4_pvr.rs`
   / `tests/pvr_render_rt4.rs`, the default-build pin, and the "no test knob in
   the artefact" row green with the new symbols. No table struct gains a field,
   so CLAUDE.md's `YakoglBackend {` literal sweep has nothing to recheck; nothing
   generated moves.
4. **PowerVR only**: the three-target regression of §2.2 passed on the AM62P
   *with the new static PDS words*, since the constant moves for every render
   (this is the one gate the V3D does not have, its job layer being unchanged).
5. **The knob is decided**: keep `*_debug_draw_targets` (its names stay in
   `KNOB_*_SYMBOLS`) or retire it in the flip's commit; either way the
   artefact row is green. Case 245 keeps its own `maxdb < 4` skip, so it
   runs as a `SKIP` against a product library and for real against the flipped
   one.
6. **Known gaps recorded as themselves**: the PowerVR emit loop's "only the last
   attachment spills" (§2.3) and `SHC_MIXED_MAX_TARGETS` 3 (§2.3) — a
   four-target mixed-class draw is refused by name until they land, and neither
   blocks the *plain* four-target flip that case 245 proves; V3D's 128-bpp
   fourth target needs `GL_EXT_color_buffer_float` (`v3d-fragment-out.md`) and
   does not block the flip either.
7. **Docs move in the same change as the flip**: the RC14 row of
   `docs/es3-cts-failures.md` shrinks to the GPUs still at 3 and
   `docs/es3/backend-caps.md` records `GL_MAX_DRAW_BUFFERS` 4 for the GPU, in
   the docs item after plan #364 has merged (both files and `docs/README.md`
   are held by other plans at this tip; **this note's index line in
   `docs/README.md` is left for that docs item**).

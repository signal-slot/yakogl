<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The PR EOT program recipe: Mesa's SPM EOT against our end-of-tile path

Host-only reading, no code, 2026-10-29 (plan #148 item pr-eot-read, issue
#3). It is part 3 "EOT store" of "Design: the firmware's partial render" in
[pvr-render-pool.md](pvr-render-pool.md), made word-exact. Nothing here is
measured. Each claim is marked:
- **read**: what a source says;
- **inferred**: a conclusion from read facts that no source states, to be
  measured on the board before code relies on it.

Mesa is 26.2.1 (MIT); its files are cited without the
`src/imagination/vulkan/` prefix. The vendored copy under `third_party/` has
the PDS generator (`pds/pvr_pds.c`) only; the rest was read from the
26.2.1 tarball tree. Our files are cited from the repository root.

Scope: colour only, one PR emit, no tile buffers, no depth/stencil
(pr-zs). The background-load program is pr-bgload-read's.

## 1. What Mesa builds for the PR EOT

- **read, when:** per framebuffer and per hardware render, not per frame.
  `pvr_arch_spm_init_eot_state` is called from framebuffer creation
  (pvr_arch_framebuffer.c:151). The scratch address is baked into the
  program, so the program is rebuilt for each scratch buffer.
- **read, the PBE state** (pvr_arch_spm.c:460-560, 285-327):
  - For up to 4 output registers (`!= 8`), one call of
    `pvr_spm_setup_pbe_state` writes the pair for emit 0
    (pvr_arch_spm.c:531-541) with `dword_count` = the render's
    `output_regs_count`, source start `PVR_PBE_STARTPOS_BIT0`, and the
    scratch address. It then adds one more pair for each tile buffer
    (:549-575; we have none).
  - The surface params are (pvr_arch_spm.c:299-316): swizzle X Y Z W
    (identity), packmode = `pvr_spm_get_pbe_packmode(dword_count)` (U32 /
    U32U32 / U32U32U32 / U32U32U32U32 for 1..4, :258-272), source format
    `8_PER_CHANNEL`, address = scratch, linear, stride = align(width,
    `LINESTRIDE_UNIT_SIZE`). The unset fields are zero: `is_normalized` (so
    `norm` = 0), `gamma` (none), `down_scale` (0), `mrt_index` (0, `render_params`
    sets none, :311-315). The render params are `max_x_clip` = width - 1,
    `max_y_clip` = height - 1, the others 0.
  - `pvr_arch_pbe_pack_state` turns those into the two STATE words and
    the three REG words (pvr_arch_job_common.c:72-257):
    - STATE_WORD0 = `address_low`;
    - STATE_WORD1 = `address_high`, `source_format`, `source_pos` (0),
      `mrt_index`, `norm`, `packmode` (:98-121);
    - REG_WORD0 = `tilerelative`, memlayout LINEAR, rotation 0,
      `linestride` = (stride - 1) / unit, `minclip_x` 0, the four swizzles,
      `downscale` (:123-257);
    - REG_WORD1 = `minclip_y` 0, `maxclip_x`, `maxclip_y`, `zslice` 0
      (:259-267; the twiddle sizes only for a twiddled layout);
    - REG_WORD2 = 0 (:80).
  - The bytes the emit stores: stride x height x sample_count x 4 x
    dword_count (pvr_arch_spm.c:324-326). That is the next emit's scratch
    offset.
- **read, the USC program** (`pvr_usc_eot`, pvr_usc.c:86-181), built with
  `shared_words = false`, `emit_count = 1`, `state_words` = the pair above
  (pvr_arch_spm.c:583-589):
  - one emit with no preceding `wop` (:100-103, only `u > 0` adds it);
  - its two STATE words are immediates in the program
    (`nir_imm_int`, pvr_usc.c:163-165). So the program carries the
    scratch address;
  - `emitpix` with `freep` set on the last emit, then a return
    (:168-176); `olchk_skip` is set (:177-179);
  - the temp count comes from pco (pvr_arch_spm.c:594) and goes into the
    PDS DOUTU (:423-427);
  - uploaded once with alignment 16 (:597-603).
- **read, Mesa's normal EOT is the same function.** It is built per job in
  `pvr_sub_cmd_gfx_per_job_fragment_programs_create_and_upload`
  (pvr_arch_cmd_buffer.c:537-640) with the same `shared_words = false`
  (:553-556), one emit per colour target. So in Mesa the PR EOT differs
  from the normal one only in its PBE state words (and so in its
  immediates), never in its instructions.
- **read, the PDS side:**
  - `pvr_pds_pixel_event_program_create_and_upload`
    (pvr_arch_spm.c:412-452) has `num_emit_word_pairs` = 0: DOUTU only,
    with the USC address, the temp count and sample rate `INSTANCE`
    (:423-427). Its data is the device's one data size, 4-dword aligned
    (:431-446).
  - The normal job does the same (pvr_arch_cmd_buffer.c:543-545, 595-607,
    622-633).
  - The code segment is not uploaded per program. It is the static one
    (pvr_arch_cmd_buffer.c:619-621 comment).
  - Both programs share one `CR_EVENT_PIXEL_PDS_INFO`: `const_size` from the
    device's data size and `usc_sr_size` from `PVR_STATE_PBE_DWORDS` (2)
    (pvr_arch_job_render.c:1187-1195; pvr_common.h:58). It is written
    once, for the normal command, and the PR command is a copy of it
    (:1388).
- **read, what the PR command changes** (pvr_arch_job_render.c:1375-1406):
  every PBE register word (`pr_pbe_reg_words`, a whole array copy), and the
  address in `CR_EVENT_PIXEL_PDS_DATA` (`pr_pds_pixel_event_data_offset`).
  They come from `spm_eot_state->pbe_reg_words` and
  `pixel_event_program_data_offset` (pvr_arch_spm.c:619,
  pvr_arch_cmd_buffer.c:1742-1746) when the render needs a PR through
  scratch.

## 2. Our normal EOT path

- **read, the USC side.** We have no per-frame USC compile. One program
  per colour-target count, offline (tools/pco_shc, from Mesa's pco), the
  *transfer context's* form: `shared_words` true, `state_regs[i]` = 2i
  (driver/src/pvr_usc_eot_blob.h:12-20, "emit i reads its PBE state from
  shared registers sh[2i], sh[2i+1]"). `eot1` is 4 dwords, one emit
  (tools/pco_shc/usc_blobs.txt:7, temps 0), `eot2` 8 and `eot3` 12 (:8-9).
  The driver uploads them at init (driver/src/pvr_render.c:479-488) and
  keeps each one's DOUTU `src0` (`eot_doutu_n[n]`), reporting them as
  `eot_doutu_src0_n` (src/osal/render.rs:45).
- **read, the PDS side.** The PDS CODE is one program, generated once for
  the largest count and copied to the fixed heap offset the firmware runs
  (`PVR_RENDER_EOT_PDS_CODE_OFF` 0x80; driver/src/pvr_render.c:572-582;
  src/osal/render.rs:15). It moves `PVR_RENDER_MAX_RT` (3) pairs by DOUTW
  into shared registers 2i, 2i+1, then DOUTU
  (driver/src/pvr_pds_pixel_event_gen.c:34-60, Mesa's
  `pvr_pds_generate_pixel_event`, third_party/mesa-26.2.1/.../pds/pvr_pds.c:535-607).
  What changes per frame is only the DATA segment: the DOUTU `src0` and the
  emit pairs.
- **read, the DATA segment** is written per frame into one slot of
  `pds_static` (src/pvr/backend.rs:8212-8229): `pvr_pds_generate_pixel_event_tq`
  with `doutu` = the program for the `nrt` emits, the `emit` pairs from
  `pbe_emit_words(addr, fmt, source_pos, j)` (:1006-1038), the unused pairs
  zero. The slot is reserved at init after the background DMA program
  (`eot_data_local` / `eot_data_off`, backend.rs:2724-2725) and its heap
  offset goes into the 3D command as `event_pixel_pds_data` (backend.rs:8360;
  render.rs:1414, dword 15).
- **read, the info word:** `event_pixel_pds_info_n[n]` is
  `pvr_cr_event_pixel_pds_info(PVR_RENDER_MAX_RT)` for every n
  (driver/src/pvr_render.c:481-483, `out->event_pixel_pds_info_n[n]`;
  driver/src/pvr_render_regs.c:22-35). The data size and the 6 shared
  words are always the 3-pair ones, because the code at the fixed offset
  is. The frame sets it from the table (backend.rs:8350-8359), dword 13.
- **read, the PBE register words** are `pbe_reg_words(fmt, alpha_one, tw,
  th, pitch, downscale, twiddle)` (backend.rs:888-922) per emitted state,
  written to the 3D command at dwords 30 + i x `PBE_STRIDE_DW` (4)
  (render.rs:1427-1433). Only the first `num_rt` slots are written
  (render.rs:1428-1429).

## 3. The word-by-word difference

For our target (one RGBA8 colour output, 1 sample, no tile buffer; the PR's
`dword_count` = the render's output registers, 1 here).

### 3.1 USC program: none

- **Mesa:** a new program with immediates (pvr_usc.c:163-165).
- **Ours:** none. The `eot1` blob already is "one `emitpix.freep` that reads
  sh0, sh1" (usc_eot_blob.h:12-20), and the PR has exactly one emit
  (see 3.2). The DOUTU `src0` is today's `eot_doutu_src0_n[1]`.
- **Instruction difference against Mesa's PR EOT:** Mesa's immediate form
  has two moves of the state words before the same `emitpix`; ours reads
  the same two words from the shared registers the PDS fills. Both end in
  the same `emitpix` with `freep`. **Inferred:** the two are equivalent on
  the hardware, since Mesa uses the shared form for its transfer queue
  (pvr_arch_job_context.c:1242-1255, `rt_pbe_regs[i] = i * PVR_STATE_PBE_DWORDS`)
  and the immediate form for its 3D jobs, and we use the shared form for
  both of ours. Our `eot1` is measured on the AM62P with every normal
  frame.
- **No `wop` and no tile-buffer flush:** one emit, none from a tile buffer
  (pvr_usc.c:100-103, 106-148 not taken).
- **Output registers > 4:** not reachable here (`PVR_BACKEND_MAX_OUT_REGS`
  4, backend.rs:484). Mesa's 8-register split into two U32U32U32U32 emits
  with source start BIT128 (pvr_arch_spm.c:468-528) is not needed.
- **Tile buffer frames** (`tb_emit`, backend.rs:8069, 8127): Mesa adds one
  more emit per tile buffer to the scratch (pvr_arch_spm.c:549-575) and its
  program flushes the tile buffer first. We refuse the PR for such a
  frame (no `frag_pr` block), counted as "no PR command" in the design's
  fallback. A first version covers `num_rt` = 1..3 colour outputs that fit
  the 4 output registers.

### 3.2 PBE STATE words (the pair the PDS puts in sh0, sh1)

One pair, `emit[0..2]`, from spm.rs's PR PBE words (spm-calc), which
transcribe `pvr_spm_setup_pbe_state` (pvr_arch_spm.c:285-327). Against the
normal frame's emit 0 (`pbe_emit_words`, backend.rs:1006-1038):

| word / field | normal EOT (backend.rs) | PR EOT (Mesa, pvr_arch_spm.c) |
|---|---|---|
| STATE_WORD0 `address_low` | the target's address (`da`, :8076) | the scratch address (low), pvr_arch_spm.c:308 |
| STATE_WORD1 `address_high` | target's | scratch's |
| `source_format` | `8_PER_CHANNEL` (8-bit), `F16_PER_CHANNEL` for float/sRGB (:1029-1033) | always `8_PER_CHANNEL` (pvr_arch_spm.c:307), raw dwords |
| `packmode` | by the format: `U8U8U8U8` for RGBA8 (:1016-1022) | by dword count: `U32` for 1, `U32U32` 2, `U32U32U32` 3, `U32U32U32U32` 4 (pvr_arch_spm.c:258-272) |
| `norm` | `!float && int.is_none()`, so 1 for RGBA8 (:1036) | 0 (unset, pvr_arch_spm.c:299-316) |
| `source_pos` | `spos`, 0 for the first (backend.rs:8115-8140) | 0 (`PVR_PBE_STARTPOS_BIT0`, pvr_arch_spm.c:531-541) |
| `mrt_index` | `j & 7` | 0 |

- **Number of dwords for the PR packmode:** the sum of the output
  registers the frame's attachments take, `out_regs` in
  `kick` (backend.rs:8089-8143; `surf_out_dwords`, :986-996). One RGBA8
  target: 1 (`U32`). Three RGBA8 targets: 3 (`U32U32U32`). An RGBA16F
  target: 2 (`U32U32`). The packmode switch rejects 0 and 5+
  (pvr_arch_spm.c:271).
- **Inferred, the load must agree:** the background-load shader reads the
  same count (pr-bgload-read). The two use the one number
  `output_reg_count` = min(out_regs, 4).

### 3.3 PBE REG words (3D command dwords 30..)

The PR command replaces the whole array (pvr_arch_job_render.c:1388-1393).
One REG pair for emit 0, from spm.rs's PR words; the other slots are zero.
Against our `pbe_reg_words` (backend.rs:888-922):

| word / field | normal | PR (pvr_arch_job_common.c:123-267 with the SPM params) |
|---|---|---|
| `tilerelative` | 1 | 1 |
| `memlayout` | LINEAR, or TWIDDLE_2D with the knob | LINEAR always (`PVR_MEMLAYOUT_LINEAR`) |
| `linestride` | `(pitch - 1) / 2` of the target's row pitch | `(align(width, unit) - 1) / unit`, the scratch's own stride |
| swizzles | `pbe_swizzle(fmt, alpha_one)`: BGRA for some formats, alpha forced to one for X8 (:852-870) | identity (X Y Z W map to chan 0 1 2 3) |
| `gamma` | sRGB targets | 0 |
| `downscale` | set on a multisampled frame (`b.samples > 1`) | 0 |
| REG_WORD1 `maxclip_x/y` | target (or chain-level clip) size - 1 | frame width - 1, height - 1 |
| REG_WORD1 `size_x/y` | log2 size when twiddled | 0 |
| REG_WORD2 | not written by us (`[[u64; 2]; MAX_RT]`, render.rs:134) | 0 |

- **The unused slots must be written as zero in the PR command.** The 3D
  command we build writes slot i from `pbe_word[i]` only for the first
  `num_rt` (render.rs:1428-1429). The PR copy has one emit, so slots 1 and
  2 of the copied normal command are still the normal frame's real targets
  unless the 3D_PR builder zeroes them. Mesa's array copy
  (pvr_arch_job_render.c:1393) replaces them with the scratch state's
  zeros. The kick-abi block `frag_pr.pbe_word[3]` must be written whole.
- **A multisampled frame:** the normal words have `downscale`, so the
  stored target is the resolved one. The PR words never have it, and the
  scratch holds `sample_count` dwords-groups (pvr_arch_spm.c:324-326, the
  `sample_count` factor). Whether the 4x frame is a PR candidate in
  version one is pr-frame-colour's decision. This note covers the 1x frame
  only; at 4x the emit's `source_pos` and the sample layout are not read
  here.

### 3.4 PDS DATA words

Our DATA segment for the PR is the same generator
(`pvr_pds_generate_pixel_event_tq`, pvr_pds_pixel_event_gen.c:34-60), one
slot, unchanged layout: DOUTU `src0`, `emit_words`, the control words. Words that
differ from today's frame:
- **DOUTU `src0`:** the same `eot_doutu_src0_n[nrt_pr]`. The PR has one emit,
  so `nrt_pr` = 1, whatever the frame's `nrt`. This is the one place the PR
  and the normal EOT can use different programs: a 3-target frame uses
  `eot3` normally and `eot1` for its PR.
- **Emit pairs:** pair 0 = 3.2's STATE words. Pairs 1 and 2 zero, as for a
  one-target frame today (backend.rs:8092-8094 comment: "the unused ones stay
  zero").
- **Size and info word:** unchanged. `event_pixel_pds_info` is the
  3-pair value for every program (driver/src/pvr_render.c:481-483).
  So the PR EOT "fits the frame's value" (the rule in pvr_arch_job_render.c:1187-1195, the
  design's part 3) by construction. Mesa's is the 0-pair size for both.
  Ours is the 3-pair size for both, because our code segment is.

### 3.5 What the 3D_PR command carries

Against the normal 3D command (the pr-frame-colour item writes it into
`frag_pr`):
- dword 15 (`event_pixel_pds_data`): the PR slot's heap offset, not the
  normal slot's (3.6);
- dwords 30.. (`pbe_word`): 3.3's, whole array;
- dword 13 (`event_pixel_pds_info`): unchanged;
- everything else (ZLS, ISP, background words, flags except the one noted
  in pvr-render-pool.md "The PR frame side", part 3) as the design says.

### 3.6 Where the PR EOT lives

- **USC:** nowhere new. `eot1`'s existing allocation, built at init
  (pvr_render.c:479-488). No address of its own.
- **PDS CODE:** the existing one at `PVR_RENDER_EOT_PDS_CODE_OFF` (shared,
  as Mesa's static code is).
- **PDS DATA:** a second slot in `pds_static`, reserved at init beside
  today's: `eot_pr_data_local = eot_data_local + PDS_ALIGN`, and
  `eot_pr_data_off = eot_data_off + PDS_ALIGN`. Both are `PDS_ALIGN`-aligned
  (128, backend.rs:348), after today's slot (backend.rs:2722-2725), and
  inside the page (`PDS_STATIC_BYTES` 4096, :452). The two addresses differ
  by construction. The contents are written per frame with the normal
  slot's (backend.rs:8229), because the packmode and the number of
  dwords can change with the frame's outputs; the scratch address is the
  render pool's for the framebuffer size (the design's part 2).
  The cache flush after the write already covers the whole page (:8230).
- **Why not at init only:** unlike Mesa (one program per framebuffer),
  nothing here is compiled per frame, so rewriting a few words per frame
  costs one more `pvr_pds_generate_pixel_event_tq` call and a `mem_write`.

## 4. Which generator builds each piece

| piece | generator | parameters it needs | changed? |
|---|---|---|---|
| USC program | `pvr_usc_eot_n_words[1]` (offline `eot1`), `eot_doutu_src0_n[1]` | none | no |
| PDS code | the one at 0x80 | none | no |
| PDS data | `pvr_pds_generate_pixel_event_tq` (driver/src/pvr_pds_pixel_event_gen.c:34) | `doutu_src0` of `eot1`; `emit_words` (3.2) padded to 6 words; `num_rt` = `PVR_BACKEND_MAX_RT` | no |
| `event_pixel_pds_info` | the table's `event_pixel_pds_info_n[1]` | none | no |
| STATE pair | spm.rs PR PBE words (transcribes pvr_arch_spm.c:285-327). Not `pbe_emit_words`: it sets `norm`, a format packmode and a format source | scratch address, `dword_count`, none else | **new, spm-calc's** |
| REG pair | spm.rs PR PBE words (same). Not `pbe_reg_words`: it swizzles, sets `downscale`, `gamma` and a target stride | scratch stride (`align(width, 2)`), width, height | **new, spm-calc's** |
| PDS data slot address | `frame_set_static_init` (backend.rs:2603-2725) | one more `PDS_ALIGN` slot | **new, small** |

- `pbe_emit_words` and `pbe_reg_words` cannot be reused "as they are" with
  a different format: no `GLES_SURF_*` format gives `U32` packmode with
  `norm` 0 and identity swizzle. Their `match` arms are format-driven
  (backend.rs:1010-1022, :852-870). The PR words come from the raw
  transcription in spm.rs, not from a format.
- **PDS_ALIGN note:** the existing slot size is only what
  `pvr_pds_generate_pixel_event_tq` writes (dsz <= 3 x 3 + 2 dwords by its
  buffer `ed`, backend.rs:8074); the 128-byte step leaves room.

## 5. Host test list

Where each lives is for the item that writes the code; none is written
here.

1. **PR STATE words, golden.** For an 800x600, 1-dword scratch at a known
   address, `pvr_spm_setup_pbe_state`'s two STATE words equal values
   packed by hand from the csbgen fields in 1 (`packmode` U32,
   `8_PER_CHANNEL`, `norm` 0, `source_pos` 0, `mrt_index` 0, address low/high).
   (spm.rs unit test, spm-calc.)
2. **PR REG words, golden.** The REG_WORD0/1 pair of the same case: identity
   swizzle, LINEAR, `linestride` = (800 - 1) / 2, `downscale` 0, `gamma` 0,
   `maxclip` 799/599, REG_WORD2 0. And an odd width (801): stride 802.
3. **Packmode by dword count.** 1, 2, 3, 4 give U32, U32U32, U32U32U32,
   U32U32U32U32. 0 and 5 are refused (pvr_arch_spm.c:271).
4. **The scratch advance.** The bytes stored equal `stride x height x
   samples x 4 x dwords` (pvr_arch_spm.c:324-326), and agree with the
   pool's scratch size (`pvr_arch_spm_scratch_buffer_calc_required_size`,
   pvr_arch_spm.c:39-75) for 1 target and 1 sample: 1,920,000 B at 800x600.
5. **PR EOT data slot (the mock platform of src/pvr/backend_tests.rs).**
   After a frame that has a `frag_pr` block:
   - the PR slot is `PDS_ALIGN` after the normal slot and inside
     `PDS_STATIC_BYTES`;
   - its DOUTU is `eot_doutu_src0_n[1]` (use `eot_emits`,
     backend_tests.rs:826, for the emitted words);
   - its pair 0 is test 1's pair, pairs 1 and 2 are zero;
   - the normal slot is unchanged by the PR slot's write.
6. **Distinct addresses.** `event_pixel_pds_data` of `frag_pr` differs from
   `frag`'s, and both are slot addresses.
7. **A 3-target frame.** `frag.event_pixel_pds_info` and
   `frag_pr.event_pixel_pds_info` are equal (the 3-pair value); the normal
   DOUTU is `eot3`'s and the PR one is `eot1`'s; the PR REG array slots 1
   and 2 are zero while `frag.pbe_word[1..3]` are the real targets'.
   (Extends `three_colour_attachments_get_a_pbe_state_and_an_eot_program_each`,
   backend_tests.rs:3169.)
8. **Refusals.** A tile-buffer frame (`tb_emit`) and an out-register count
   of 0 or above 4 produce no `frag_pr` (the frame kicks without a PR).
9. **A z-only frame.** No PR colour state (`pbe_emit_words_z_only`,
   backend.rs:938-945, is not used for the PR).
10. **Normal EOT unchanged.** The existing EOT tests
    (backend_tests.rs:3169) pass with the PR block off.

For the board, after the host tests: the first PR with a scratch store
shows `partial renders >= 1` and a byte-exact read-back (forcedpr-case).
Neither the PR STATE words nor the `source_pos` / `norm` choices can be
checked off the board.

## 6. Open points (for the planner)

- **Inferred, the shared-register form of the EOT for a PR.** Mesa builds
  the PR's with immediates, ours reads shared registers. The firmware runs
  the PDS code for a PR the same way as for a render
  (pvr_arch_job_render.c:1388-1406: only the data address differs), so it
  should work. If the board shows the PR's `emitpix` storing zeros, the
  fallback is a per-size immediate program from `gen_usc_blobs.py`.
- **`pbe_reg_words`' `downscale`:** the PR words never downscale. A
  multisampled frame's PR is not covered above (3.3).
- **Which 3D-command flag a scratch PR needs** (`SPMSCRATCHBUFFER`): in
  pvr-render-pool.md "The PR frame side", part 3.

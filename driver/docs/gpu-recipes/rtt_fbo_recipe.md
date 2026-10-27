<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Render-to-texture (offscreen FBO) + MRT on PowerVR Rogue TBDR (BXS-4-64, BVNC 36.53.104.796)

Read-only analysis. Sources:
- Mesa PVR Vulkan driver: Mesa 26.2.1 `src/imagination/` (paths below abbreviated `M/`)
- DDK KM tree: the `ti-img-rogue-driver` 25.2 kernel tree (abbreviated `D/`)

Bottom line: **RTT is just "chain two kicks."** Both halves already work in our driver. The
only new plumbing is (a) point pass-2's texture base at pass-1's PBE output VA, (b) serialize
the two kicks so pass-2's TA/3D starts after pass-1's 3D finishes. On Rogue the SLC (L2) is a
unified coherent cache shared by PBE (writes) and TPU (reads), so **no CPU-style data cache
flush is required between the two GPU renders** — ordering is the requirement, not a flush.

---

## 1. Chaining two renders: what happens between pass-1 (write T) and pass-2 (sample T)

### 1a. Cache/coherency — the important finding

On Rogue/BXS the **PBE writes and the TPU reads both go through the same unified SLC (L2)**. The
DDK does **not** issue any per-render data-cache flush/invalidate for a framebuffer-then-texture
dependency. Evidence:

- The only SLC flush/invalidate command, `RGXFWIF_KCCB_CMD_SLCFLUSHINVAL`
  (`D/include/rogue/rgx_fwif_km.h:681-692`, struct `RGXFWIF_SLCFLUSHINVALDATA`), is a *host-driven*
  KCCB command used for power/teardown and explicit CPU<->GPU sync — it is **not** emitted between
  dependent render kicks.
- The per-kick cache op the DDK *does* do is **MMU** cache invalidation only, and only when page
  tables change (buffer alloc/free/remap): `RGXFWIF_KCCB_CMD_MMUCACHE`
  (`D/services/server/devices/rgxmem.c:198-201`, `pfnMMUCacheInvalidate` wired in
  `D/services/server/devices/rgxinit_common.c:314-315`). This is **not** data coherency; do it
  once when T is first mapped, not between passes.
- Ordering/coherency between dependent renders in the DDK is handled by **sync checkpoints/fences**
  (the KM sync framework), i.e. pure *ordering*, not cache maintenance.

Why this works: the TBDR 3D phase writes each tile's PBE output to T through the SLC; by the time
the 3D render's completion fence signals, T is coherent in SLC/DRAM. The small per-USC **TPU/TCU
L1 texture cache is invalidated at render (kick) boundaries by the FW/HW**, so pass-2's first
sample of T is fresh.

**Requirement between passes = pipeline sync (ordering), not a cache op:**
- A second full geom+3D kick is sufficient *provided it is ordered after pass-1's 3D*.
- For a bare-metal driver that does not use the DDK sync framework: **wait for pass-1's 3D
  completion (fence/IRQ) before kicking pass-2**, or submit both on the same FW context so the FW
  serializes them. Do not start pass-2's TA while pass-1's 3D may still be writing T.

Mesa's own render-pass code confirms the model: the driver tracks pipeline-stage barriers
(`pvr_cmd_buffer_update_barriers`, `M/vulkan/pvr_arch_cmd_buffer.c:321-355`; GRAPHICS sets
`GEOM_BIT|FRAG_BIT`) and, when a subpass must *sample* a just-written attachment via general
access (our case, not the on-chip same-tile input-attachment fast path), it **splits the work into
a separate HW render = a separate geom+3D kick** (`M/vulkan/pvr_arch_hw_pass.c`, input-attachment
"from memory" path ~:868 and new-render allocation ~:1992-2011). So "two kicks, ordered" is
exactly the hardware-intended mechanism.

### 1b. Bring-up safety margin

The structural mechanism is certain, but our driver bypasses the DDK sync/cache framework. For the
*first* on-silicon bring-up I recommend inserting a belt-and-braces SLC flush+invalidate between
the two kicks (either the host `SLCFLUSHINVAL` path or a direct SLC-flush CR poke), confirm the
sample is correct, then remove it and re-confirm the automatic SLC-coherency + ordering path is
enough. The DDK not doing it says it is unnecessary; verifying on BXS-4-64 costs one kick.

---

## 2. The render target as a texture — is PBE output linear or twiddled?

**PBE output layout is explicitly selected per surface; force LINEAR and the exact write VA is the
texture base — no twiddle conversion.**

- PBE layout comes from `surface_params->mem_layout`, packed into `PBESTATE_REG_WORD0.memlayout`
  in `pvr_arch_pbe_pack_state` (`M/vulkan/pvr_arch_job_common.c:144-157`):
  - `PVR_MEMLAYOUT_LINEAR` -> `ROGUE_PBESTATE_MEMLAYOUT_LINEAR`
  - `PVR_MEMLAYOUT_TWIDDLED` -> `TWIDDLE_2D`, `PVR_MEMLAYOUT_3DTWIDDLED` -> `TWIDDLE_3D`
  There is **no implicit twiddle**: whatever you set in `mem_layout` is what the PBE writes.
- For LINEAR, the PBE line stride is `PBESTATE_REG_WORD0.linestride = (stride - 1) /
  ROGUE_PBESTATE_REG_WORD0_LINESTRIDE_UNIT_SIZE` (`:174-175`); `size_x/size_y` (log2 dims) are only
  written for the twiddled cases (`:286-290`), i.e. linear needs only the stride.
- The destination byte address is `PBESTATE_STATE_WORD0.address_low` + `STATE_WORD1.address_high`
  = `surface_params->addr` (`:115-121`) — the raw device VA of T.

The TPU side matches this exactly for a linear RGBA8888 sample (`pvr_arch_pack_tex_state`,
`M/vulkan/pvr_arch_tex_state.c`):
- `mem_layout == LINEAR` -> `TEXSTATE_IMAGE_WORD0.textype = ROGUE_TEXSTATE_TEXTYPE_STRIDE` (:140-144)
- `TEXSTATE_STRIDE_IMAGE_WORD1`: `stride = info->stride - 1`, `texaddr = base + offset` (:244-251)

This is precisely our already-working linear-sample image state (IMAGE_WORD0=0x003F00FC60000A64 +
STRIDE_IMAGE_WORD1 with `stride-1` and `((base>>2)<<16)`). **So: write T with PBE
mem_layout=LINEAR and `linestride`=`(row_bytes/unit)-1`; then sample T with the same VA as
`texaddr`, `textype=STRIDE`, `stride`=`row_bytes-1`. Same bytes, same VA, no conversion.** (Match
the PBE `linestride` unit and the TPU `stride` unit to the same 64*4=256-byte row for the 64x64
carveout.)

Note: linear PBE output is fully supported but is the slow/simple path (twiddled is the default
for perf and has different alignment); for RTT bring-up linear is the right choice because it makes
write-VA == sample-VA trivial.

---

## 3. MRT (multiple render targets): how 2+ RTs are described

### 3a. Output-register allocation (`M/vulkan/pvr_arch_mrt.c`)
- `pvr_arch_init_usc_mrt_setup` (:231-272) loops attachments, calling `pvr_alloc_mrt` (:116-198)
  per RT.
- Per RT, `pixel_size = DIV_ROUND_UP(accum_format_bits, 32)` dwords (:124-125; RGBA8888 => 1 dword).
- Each RT is packed into the **pixel-output-register pool** first
  (`pvr_mrt_alloc_from_buffer` :57-80, bit-allocator over `max_out_regs`); if the regs are
  exhausted it spills to a **tile buffer** (`USC_MRT_RESOURCE_TYPE_MEMORY`, :138-190).
- Result per RT is a `usc_mrt_resource`: type OUTPUT_REG with `reg.output_reg` = base dword index,
  or MEMORY with `mem.tile_buffer` + `mem.offset_dw`. `setup->num_render_targets` counts them;
  `num_output_regs` is rounded to a power of two (:266).

### 3b. The EOT ("end of tile") emit program — the actual MRT emit (`M/vulkan/pvr_usc.c` `pvr_usc_eot`)
One EOT USC program emits all RTs in a loop of length `emit_count` (= number of RTs):
```
for (u = 0; u < props->emit_count; ++u) {
    if (u > 0) nir_wop_pco(&b);              // ordering between emits
    // (if RT u is a tile buffer: flush_tile_buffer_pco burst first)
    state0 = state_words[u*ROGUE_NUM_PBESTATE_STATE_WORDS + 0];   // PBESTATE_STATE_WORD0
    state1 = state_words[u*ROGUE_NUM_PBESTATE_STATE_WORDS + 1];   // PBESTATE_STATE_WORD1
    last_emit = nir_emitpix_pco(&b, state0, state1);              // EMITPIX for RT u
}
nir_intrinsic_set_freep(last_emit, true);    // free partition on final emit
```
`struct pvr_eot_props { unsigned emit_count; bool shared_words; const uint32_t *state_words; ... }`
(`M/vulkan/pvr_usc.h:25`). `capture_eot_usc.c` shows the single-RT case (`emit_count=1`). So MRT =
bump `emit_count` and provide two state words per RT.

### 3c. Per-RT PBE state fields (`pvr_arch_pbe_pack_state`, `M/vulkan/pvr_arch_job_common.c:82-297`)
Build one set of words **per RT**:
- `pbe_cs_words[2]` (STATE words, consumed by the EOT USC EMITPIX above):
  - `STATE_WORD0.address_low` / `STATE_WORD1.address_high` = that RT's dest VA (:115-121)
  - `STATE_WORD1.source_format`, `.packmode`, `.norm` (format) (:122-138)
  - `STATE_WORD1.source_pos = pvr_pbestate_source_pos(render_params->source_start)` — **which
    output-register dword this RT reads from** (from 3a's `reg.output_reg`), plus
    `source_pos_offset_128` for regs >=128b (:124-130)
  - `STATE_WORD1.mrt_index = render_params->mrt_index` — **the RT index; 0 for a single RT** (:132-133)
- `pbe_reg_words[3]` (REG words, go to the fragment KM stream / PBE registers):
  - `REG_WORD0`: `memlayout`, `linestride`, swizzle chan0-3, gamma, downscale (:141-283)
  - `REG_WORD1`: clip rect, `size_x/size_y` (twiddled only), `zslice` (:285-296)
  - `REG_WORD2 = 0` (:100)

Input structs: `struct pvr_pbe_surf_params` (addr, mem_layout, stride, swizzle[4], source_format,
pbe_packmode, width/height, ...) and `struct pvr_pbe_render_params` (clip, `source_start`,
`mrt_index`, `slice`) — both in `M/vulkan/pvr_job_common.h`.

### 3d. Where the per-RT words land in the frag/render command (`M/vulkan/pvr_arch_job_render.c`)
- `job->pbe_reg_words[PVR_MAX_COLOR_ATTACHMENTS][ROGUE_NUM_PBESTATE_REG_WORDS]` (REG words are 3,
  64-bit each: STATIC_ASSERTs at :1046-1049) are `memcpy`'d into the fragment KM stream at
  `pvr_frag_km_stream_pbe_reg_words_offset(dev_info)` (:1044-1050). One row per RT.
- The STATE words feed the EOT USC (3b).
- The FS writes each RT to its allocated output registers (po0, po1, ...) at the `reg.output_reg`
  dword offsets; EMITPIX's `source_pos` pulls each RT's pixel from those regs.

So: **N render targets = N rows of pbe_reg_words in the KM stream + N (state0,state1) pairs in the
EOT USC (emit_count=N) + the FS writing po0..po(N-1); each RT's `source_start` = its output-reg
dword, `mrt_index` = its index.**

---

## 4. Minimal concrete recipe (bare-metal, no Vulkan runtime)

Reuse the structures you already have; add only the second kick + VA plumbing.

**Pass-1 — render into T (your existing render-to-carveout):**
1. TA(geom)+3D exactly as today, PBE dest = T at VA_T.
2. In the PBE emit state for T, set `mem_layout = LINEAR`
   (`REG_WORD0.memlayout = ROGUE_PBESTATE_MEMLAYOUT_LINEAR`), `linestride = (256/unit) - 1` for a
   64x64 RGBA8888 row (256 B), `STATE_WORD0/1 address = VA_T`, `mrt_index = 0`,
   `source_start = BIT0` (single RT in output reg 0). This is your working "dst carveout" render.

**Between passes — sync only, no data cache op:**
3. Ensure T's mapping MMU-invalidate was done once at map time (not per pass).
4. **Wait for pass-1's 3D completion** (fence/IRQ) before kicking pass-2, or chain on the same FW
   context. No SLC/CPU data flush needed (SLC is coherent; TPU L1 auto-invalidated at kick).
   [Bring-up: optionally add one SLC flush+invalidate here, verify, then drop it — see 1b.]

**Pass-2 — sample T (your existing linear-texture sample):**
5. TA(geom)+3D that samples a linear texture. Set the image state `texaddr = VA_T` (same VA as
   pass-1's PBE dest), `textype = STRIDE`, `stride = 256 - 1`, IMAGE_WORD0 format bits =
   your working `0x003F00FC60000A64`. Sampler + PDS DOUTD loading image+sampler into the USC common
   store + SMP in the FS — all unchanged from today.
6. Pass-2's PBE targets the final destination (panel scanout or another buffer).

**What is reused vs changed:**
- Frag command / PBE emit state (pbe_cs_words + pbe_reg_words): *same code*, used in pass-1 to
  describe T as the PBE **dest** (mem_layout=LINEAR).
- Image state (IMAGE_WORD0 + STRIDE_IMAGE_WORD1): *same code*, used in pass-2 to describe T as the
  TPU **source**, with `texaddr = VA_T`.
- New: a second kick, ordered after the first; and passing VA_T from pass-1's PBE addr into
  pass-2's `texaddr`. Nothing else.

For MRT in pass-1 (e.g. write color + a data buffer), replicate step 2 per RT: N pbe_reg_words
rows, EOT `emit_count=N`, FS writing po0..po(N-1), each RT's `source_start` = its output-reg dword
and `mrt_index` = its index (section 3).

---

## Confidence and on-silicon verification

**High confidence** on the register/structure mechanism — it is read directly from the Mesa PVR
driver that targets this exact Rogue/BXS arch, cross-checked against the DDK 25.2 FWIF. LINEAR
PBE-out == LINEAR/STRIDE TPU-in at the same VA is unambiguous in the source.

**Needs on-silicon verification on BXS-4-64 (AM62P):**
1. That **ordering alone** (no explicit SLC flush) is sufficient between the two kicks in our
   bare-metal driver — bring up with a flush, then remove it and confirm (section 1b). This is the
   single real risk.
2. Linear PBE-output alignment: `LINESTRIDE_UNIT_SIZE` and base-VA alignment constraints for the
   256-byte row (grep the csbgen/hwdefs headers for the exact `ROGUE_PBESTATE_REG_WORD0_LINESTRIDE_UNIT_SIZE`
   value and round the carveout stride/base to it).
3. That the FS actually routes each MRT output to the expected output register matching
   `source_start` (verify with the 2-RT case before relying on it).

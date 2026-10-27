<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Alpha blending on PowerVR Rogue/Volcanic TBDR (BXS-4-64, BVNC 36.53.104.796)

Scope: how standard GL-ES over-blending (`src_alpha`, `one_minus_src_alpha`) is realized on
this arch, derived from the Mesa PCO/PVR driver and DDK 25.2. **Read-only analysis.**

> **Verified on silicon (2026-09-14):** `yakogl` implements exactly this. `pco_shc`
> (`PCO_SHC_BLEND=...`) lowers the blend state into fragment-shader variants
> (the `blend` lines of the shader list `tools/glsl2usc.sh` reads), the GL core picks the variant for `glBlendFunc`, every object
> is already submitted TRANSLUCENT, and `driver/tools/pvrlab/gl_test.c` case 8 reads back
> over (0xbf008080), additive (0xff00ffff) and premultiplied (0xff00ff80) results that
> match the GL arithmetic exactly. No PBE/PPP change was needed. Blend constants
> (`PCO_SHC` allocates `blend_consts` shared registers after the push constants, the GL
> core appends `glBlendColor` to the uniform DMA) and `glColorMask` (`PCO_SHC_COLORMASK`,
> `nir_lower_blend`'s colormask → fb-fetch of the kept channels) are verified the same
> way (case 8 frames 5..7: 0x800080bf, 0x8000ffff, 0xff008080).

Path prefix used below:
`MB = <Mesa 26.2.1>/src/imagination`

---

## TL;DR — the answer to the central uncertainty

**Blending is NOT fixed-function on this arch. It is lowered into the fragment shader as an
on-chip read-modify-write of the pixel-output (PIXOUT) registers.** Option **(c)** in the task.

- There is no blend hardware unit and no blend fields in the PBE — `pbestate.xml` is right, and
  the Vulkan driver has no "blend lowering" of its own because it reuses Mesa's **common**
  `nir_lower_blend` pass (`MB/pco/pco_nir.c:970`).
- The "framebuffer fetch" that blending needs is just **reading the on-chip PIXOUT hardware
  register** for that render target. On a TBDR the destination color for the current pixel is
  resident on-chip in the tile, so FB-fetch is a register move, not a memory load.
- The FS computes `src*Fs + dst*Fd` in floats and writes the result back to the **same** PIXOUT
  register with `store_output`. The PBE later emits PIXOUT → memory in a separate end-of-tile
  (EOT) program — unchanged by blending.
- The only global state that changes for a blended draw is the **ISP object pass type**, set to
  `TRANSLUCENT`, which is what tells the ISP/USC to make the current on-chip destination color
  available (background load) and to process the object in submission order instead of doing
  opaque hidden-surface elimination.

Mesa/DDK **do** implement blending for this part — fully, in the PCO compiler. This is
authoritative for the architecture.

---

## Q1 — The actual blend mechanism, with citations

### It is lowered in NIR by the common pass

`MB/pco/pco_nir.c:970`
```
NIR_PASS(_, nir, nir_lower_blend, &data->fs.blend_opts);   // FRAGMENT stage only
```
Runs right before interpolation lowering (`pco_nir_lower_interpolation`, `:977`) and the
per-fragment output pass `pco_nir_pfo` (`:978`). Order matters (see Q3/Q4).

`nir_lower_blend` (Mesa common, header `MB/../compiler/nir/nir_lower_blend.h`) rewrites every
`store_output` to a color RT into: read current color via **framebuffer-fetch**, compute
`nir_color_blend()` (`func`, `src_factor`, `dst_factor` per channel), apply the color write mask,
store the blended result. Its options struct is filled from the Vulkan color-blend state — see
Q3.

### Framebuffer-fetch = `nir_load_output` with `io_semantics.fb_fetch_output = true`

The driver's own color-write-enable lowering shows the exact intrinsic shape the blend pass also
uses (`MB/pco/pco_nir_pvfio.c:784-809`):
```
io_semantics.fb_fetch_output = true;
nir_def *prev_input =
   nir_load_output(b, num_components, bit_size, offset,
                   .base = ..., .range = ..., .component = ...,
                   .dest_type = ..., .io_semantics = io_semantics);
```
`uses_fbfetch_output` is recorded on the shader (`MB/vulkan/pvr_usc.c:433`,
`MB/pco/pco_nir.c:287`) and drives both the pass-type selection and resource setup.

### FB-fetch lowers to a PIXOUT register read (the "load tile color" op)

`trans_load_output_fs()` (`MB/pco/pco_trans_nir.c:848`). For the normal single/multi-RT case the
output lives in **output registers**, not a tile buffer, so FB-fetch is literally a `mov` from
the pixel-output register class (`MB/pco/pco_trans_nir.c:879-883`):
```
if (!tile_buffer) {
   pco_ref src = pco_ref_hwreg(range->start + component, PCO_REG_CLASS_PIXOUT);
   return pco_mov(&tctx->b, dest, src, .olchk = tctx->olchk);
}
```
- `PCO_REG_CLASS_PIXOUT` = the on-chip pixel output registers that hold this pixel's current RT
  color inside the tile. **This is the "framebuffer fetch / load tile" primitive on Rogue.**
- `.olchk` = **output-load check** (`MB/pco/pco_ops.py:302` `OM_OLCHK`; printed as `.olchk` in
  `MB/pco/pco_print.c:893`). It is ON by default for FS PIXOUT accesses
  (`MB/pco/pco_trans_nir.c:4087` `tctx.olchk = !data->fs.uses.olchk_skip`; only the internal EOT
  shader skips it, `MB/vulkan/pvr_usc.c:178`). It flags the backend so the hardware guarantees the
  on-chip output value is loaded/coherent before the shader reads it — i.e. it is the coherency
  bit that makes read-before-write blending correct.

### Blend result written back to the same PIXOUT register

`trans_store_output_fs()` (`MB/pco/pco_trans_nir.c:629`, non-tile-buffer branch `:649-653`):
```
pco_ref dest = pco_ref_hwreg(range->start + component, PCO_REG_CLASS_PIXOUT);
return pco_mov(&tctx->b, dest, src, .olchk = tctx->olchk);
```
So a blended FS is: `interpolate/sample → src color; mov dst = PIXOUT; blend = src*Fs + dst*Fd;
mov PIXOUT = blend`. Pure on-chip RMW; no memory traffic in the per-draw FS.

### PBE emit is unchanged and separate (EOT)

The PBE store to memory is done by the **end-of-tile** USC program via the `emitpix` backend op,
generated in `MB/vulkan/pvr_usc.c:168` (`nir_emitpix_pco`), op defined at
`MB/pco/pco_ops.py:439` (`O_EMITPIX`). `emitpix` never appears in a normal draw FS — it belongs to
the EOT/PBE-emit shader that flushes PIXOUT → framebuffer. **Your existing `pbe_word` emit state /
EOT path does not change for blending.**

### Fixed-function? No.
No blend equation, factor, or logic-op field exists in `pbestate.xml` or the ISP state
(`MB/csbgen/rogue/ppp.xml` `STATE_ISPA`/`STATE_ISPB`/`STATE_ISPCTL`, lines 131-169, carry only
depth/stencil/passtype/sref — no blend). Logic-op is also FS-lowered
(`MB/vulkan/pvr_arch_pipeline.c:2236`, `nir_color_logicop`). Blend constants are pushed as shared
registers (`PVR_BUFFER_TYPE_BLEND_CONSTS`, `MB/vulkan/pvr_arch_pipeline.c:548-552`), read in-shader
via `load_blend_const_color_rgba` (`MB/pco/pco_nir.c:251`).

---

## Q2 — ISP/PPP state that selects blended objects, and tile availability

### Pass type = TRANSLUCENT, chosen automatically from fbfetch

`MB/vulkan/pvr_arch_pipeline.c:1349-1357`:
```
if (depth_feedback && !early_frag)  pass_type = DEPTH_FEEDBACK;
else if (uses.discard)              pass_type = PUNCH_THROUGH;
else if (uses.fbfetch)              pass_type = TRANSLUCENT;   // <-- blending lands here
else                                pass_type = OPAQUE;
```
`uses.fbfetch` is set whenever the FS reads the framebuffer, which blending always does. So
**enabling blend ⇒ FS gets fbfetch ⇒ object is submitted as TRANSLUCENT.**

The corroborating statement that TRANSLUCENT specifically means "read the destination/background":
`MB/vulkan/pvr_arch_job_transfer.c:3465-3466`
```
ispa.passtype = read_bgnd ? ROGUE_TA_PASSTYPE_TRANSLUCENT
                          : ROGUE_TA_PASSTYPE_OPAQUE;
```
and the clear/blit template uses TRANSLUCENT when it must preserve/read existing tile content
(`MB/vulkan/rogue/pvr_clear.c:116`).

### Where the pass type physically goes

`pass_type` (`ROGUE_TA_PASSTYPE_*`) is written into the ISP object control word A,
`STATE_ISPA.passtype`, a 3-bit field at bits [24:26] (`MB/csbgen/rogue/ppp.xml:147`), enum
`PASSTYPE` (`ppp.xml:89-97`): `OPAQUE=0, TRANSLUCENT=1, PUNCH_THROUGH=2, ...`. `pvr_clear.c:116`
assigns `ispa.passtype = ROGUE_TA_PASSTYPE_TRANSLUCENT` directly, i.e. the `ROGUE_TA_PASSTYPE_*`
enum values are the `ppp.xml` `PASSTYPE` values.

Other relevant ISPA fields for a translucent over-draw (`ppp.xml:145-160`): `objtype` (triangle),
`dcmpmode`/`dwritedisable`/`dfbztestenable` (keep depth **test** on for correctness, but for
typical translucent draw **disable depth write** `dwritedisable=1` so translucent fragments don't
occlude each other), `sref`.

### What makes the destination color available on-chip

Two things, both already covered:
1. **passtype = TRANSLUCENT** tells the tiling/ISP pipeline this object reads the background, so
   the current tile color is kept resident and made visible to the USC (rather than the opaque
   fast path that would let later geometry overwrite it). Objects are rasterized in **submission
   order** for the translucent pass (no opaque HSR reordering), which is required for correct
   ordered over-blending.
2. **`.olchk` (output-load check)** on the PIXOUT read/write instructions is the per-instruction
   coherency guarantee that the on-chip output register holds the loaded destination before the
   shader reads it.

### Tile load/store setup
- **Single RT in output registers (the common case, incl. RGBA8888):** *no* extra tile-buffer
  setup. FB-fetch is a PIXOUT register read; PBE EOT emit is unchanged. This is your case.
- **Tile buffers** (`USC_MRT_RESOURCE_TYPE` ≠ `OUTPUT_REG`, e.g. many/large MRTs or when outputs
  are spilled to the on-chip tile buffer) use a different lowering: address = shared-reg base +
  per-component tiled offset special register (`PCO_SR_TILED_LD_COMP*`), read via `pco_ld_regbl`,
  written via `pco_st_tiled` (`MB/pco/pco_trans_nir.c:885-923`, `:655-723`). You do **not** need
  this for a single RGBA8 target.

---

## Q3 — Minimal recipe: translucent triangle over opaque background (`src_alpha`/`one_minus_src_alpha`)

Assumes your existing working TA(geom)+3D path with a hand-packed frag command, an FS that outputs
RGBA to po0, an ISPA passtype that is TRANSLUCENT-capable, and a working PBE emit (`pbe_word`).

**What changes vs. your current opaque draw:**

1. **ISP object state (per-object, in the geometry/TA stream):**
   - `STATE_ISPA.passtype = TRANSLUCENT (1)` (`ppp.xml:147`). This is the single most important
     change — it enables ordered rendering + background read.
   - `STATE_ISPA.dwritedisable = 1` (bit 21) for a standard translucent overlay (keep `dcmpmode`
     depth *test* as desired). Leave `dfbztestenable` per your depth setup.
   - Draw order: submit the translucent triangle **after** the opaque background, in the same
     render pass/tile. Translucent objects are processed in submission order.

2. **PPP/PBE:** *no change.* No blend fields exist there. PBE emit state (`pbe_word`) and the EOT
   emit program stay exactly as they are — blending is finished on-chip before EOT.

3. **Fragment shader: you need a blend-aware FS.** Reuse your existing texture/interp FS to
   produce the source RGBA, then append the blend as an on-chip RMW of po0:
   ```
   src   = <your existing FS result, RGBA float, po0>
   dst   = load_output(po0, fb_fetch_output=true)     // reads PIXOUT (on-chip dst)
   a     = src.a
   out.rgb = src.rgb * a + dst.rgb * (1 - a)          // src_alpha / one_minus_src_alpha
   out.a   = src.a  * a + dst.a  * (1 - a)            // GL default: same factors on alpha
   store_output(po0, out)                             // writes PIXOUT
   ```
   Emit this through the shader toolchain by enabling the driver's own blend lowering rather than
   hand-writing the arithmetic: set `data->fs.blend_opts` and let `nir_lower_blend` do it
   (`MB/pco/pco_nir.c:970`). Blend option fields to set (mirror
   `MB/vulkan/pvr_arch_pipeline.c:2247-2270`):
   ```
   rt[0].format          = <your RT pipe_format, e.g. PIPE_FORMAT_R8G8B8A8_UNORM>
   rt[0].rgb.func        = PIPE_BLEND_ADD
   rt[0].rgb.src_factor  = PIPE_BLENDFACTOR_SRC_ALPHA
   rt[0].rgb.dst_factor  = PIPE_BLENDFACTOR_INV_SRC_ALPHA
   rt[0].alpha.func      = PIPE_BLEND_ADD
   rt[0].alpha.src_factor= PIPE_BLENDFACTOR_SRC_ALPHA         // GL-ES default; use ONE/INV_SRC_ALPHA
   rt[0].alpha.dst_factor= PIPE_BLENDFACTOR_INV_SRC_ALPHA     //   for premultiplied-correct alpha
   rt[0].colormask       = 0xF
   logicop_enable        = false
   ```
   The lowering must run **before** `pco_nir_pfo` (`:978`) so the pack/unpack around the PIXOUT
   register is applied by pfo to both the injected `load_output` and the final `store_output`.

4. **Shader `uses.fbfetch` must be set** (it is, automatically, once `load_output(fb_fetch)` is
   present — `MB/pco/pco_nir.c:287`, `pvr_usc.c:433`). Your driver must then map that to
   `passtype=TRANSLUCENT` when packing the object's ISPA (step 1), exactly as
   `pvr_arch_pipeline.c:1355` does.

**Nothing else** (PDS, coefficient loading, PBE, EOT) changes for the single-RGBA8 over-blend
case.

---

## Q4 — Exact NIR/PCO construct for the FB-fetch (for a shader-compile harness)

Emit, in the fragment shader, immediately before the color `store_output`:

- **Intrinsic:** `nir_load_output`
- **Required semantic:** `nir_io_semantics.fb_fetch_output = true` and
  `location = FRAG_RESULT_DATA0 + rt_index` (use `FRAG_RESULT_COLOR` only for the special on-chip
  input-attachment case, `MB/pco/pco_trans_nir.c:868-872`).
- **Operands / indices** (from `MB/pco/pco_nir_pvfio.c:800-809`):
  - `num_components` = 4, `bit_size` = 32 (logical float pre-pfo)
  - `src[0]` = offset (must be constant 0; asserted `pco_trans_nir.c:856-857`)
  - `.base = 0`, `.range`, `.component = 0`, `.dest_type = nir_type_float32`
  - `.io_semantics = { .location = FRAG_RESULT_DATA0, .fb_fetch_output = true }`
- **Blend + writeback:** compute `nir_color_blend()` (or hand-build `src*a + dst*(1-a)`), then the
  ordinary `nir_store_output` to the same `FRAG_RESULT_DATA0` (`pco_nir_pvfio.c:357`,
  `pco_trans_nir.c:630`).

The compiler then lowers both to `mov` on `PCO_REG_CLASS_PIXOUT` with `.olchk` set
(`pco_trans_nir.c:882` / `:652`). No new op is required in the ISA — reuse `mov`/`pco_mov`.

**On-chip framebuffer format/layout assumptions:**
- The PIXOUT registers hold the RT value **packed in the render-target format** (e.g. one 32-bit
  reg for RGBA8888). Format pack/unpack is inserted by `pco_nir_pfo` (`pack_to_format`,
  `MB/pco/pco_nir_pvfio.c:344-373`; unpack around `load_output` at `:404-458`), which runs
  **after** blend. So: blend is expressed on **logical float** colors; pfo converts float↔packed
  around the PIXOUT register.
- If you hand-write the FS **without** pfo (raw PCO/ISA), you must do this yourself: read PIXOUT
  (packed) → unpack per RT format → blend in float → repack → write PIXOUT. Simplest robust path
  for a bring-up harness: run your color through the toolchain with `nir_lower_blend` + `pfo` so
  packing is handled for you.
- One PIXOUT register per RT component group per RT (index = `outputs[location].start +
  component`). For a single RGBA8888 RT this is the base `po0` register you already emit.

---

## Minimal implementation path

**Fixed-function is not available — blending is FS-lowered on this arch. Use the FS-fetch path:**

1. **State (per translucent object):** set `STATE_ISPA.passtype = TRANSLUCENT (1)` (bits 24:26,
   `ppp.xml:147`); set `dwritedisable=1`; submit after the opaque background, in order.
   **No PBE/PPP blend state exists or is needed.**
2. **Shader:** build a blend-aware FS = your current texture/interp FS **plus** one
   `nir_load_output(fb_fetch_output=true, FRAG_RESULT_DATA0)` for the destination, a float blend
   `src.rgb*src.a + dst.rgb*(1-src.a)` (and the matching alpha), and the normal `store_output`.
   Easiest: fill `nir_lower_blend_options` (SRC_ALPHA / INV_SRC_ALPHA, ADD) and run
   `nir_lower_blend` **before** `pco_nir_pfo`. This compiles to on-chip PIXOUT `mov`s with
   `.olchk`.
3. **EOT/PBE:** unchanged — your existing `pbe_word` emit stays as-is.
4. **Driver glue:** when the FS uses fbfetch, force the object's ISPA passtype to TRANSLUCENT
   (mirror `pvr_arch_pipeline.c:1355`). Push blend constants as shared regs only if a factor needs
   them (not needed for SRC_ALPHA/INV_SRC_ALPHA).

**Confidence:** High on the mechanism and the exact NIR/PCO constructs — the PCO/PVR driver
implements blending precisely this way and the code paths are unambiguous (FS-lowered RMW of
PIXOUT; TRANSLUCENT passtype gated on fbfetch; `.olchk` coherency; PBE/PPP carry no blend fields).
This is the reference Mesa driver for BXS-4-64 (bvnc 36.53.104.796, arch ROGUE/Volcanic), so it is
authoritative for the silicon's capability.

**Still needs on-silicon verification in your hand-written driver:**
- That `passtype=TRANSLUCENT` alone (with your TA/ISP stream) causes the HW to preserve/load the
  opaque background into PIXOUT for the translucent object — i.e. that the on-chip dst read returns
  the previously-shaded color, not stale/zero. This is the one behavior that depends on the tiling
  pipeline being configured as Mesa configures it (region headers, object emission order, HSR
  disabled for the translucent pass).
- That your FS's `.olchk` (output-load) encoding is set on the PIXOUT read; without it the read may
  race the on-chip load.
- The exact pack/unpack for your RT format if you bypass `pco_nir_pfo` (verify RGBA8888 packing in
  po0 matches what PBE emits).
- Interaction with depth: confirm `dwritedisable`/`dcmpmode` give the ordering you expect for
  overlapping translucent triangles.

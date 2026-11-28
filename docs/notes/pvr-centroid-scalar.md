<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: the 7 `qualification_order.variables.valid` fails (cause S-E)

Plan #400, item `cause-centroid`. A host investigation only: nothing under
`src/`, `tests/` or `tools/` changed, and no board was used. Integration
tip `ad2b6cc9`, 2026-11-28. The experiments were scratch host tests, not
committed (§3).

**Short answer.** The cause is **not** in the program yakogl builds, and
**not** "scalar centroid varyings". The host shows the 7 programs are right:
same layout as their twins, the same iterator words as Mesa's pco, white in
the byte model. On the board, at the same build, a lone scalar centroid
varying passes and so do two scalar smooth varyings. What is left is a fault
that only the board shows, in the one combination these 7 alone have: a
centroid `ditrp` followed by a second varying read, judged on exact white.
§5 gives the board experiment that tells the remaining candidates apart. The
cause stays **open** until it runs.

## 1. The cases

`tools/cts/am62p-notpass-causes.py` files these 7 Fail lines of
`docs/es3/am62p-gles3-notpass.tsv` under S-E (rule
`^shaders\.qualification_order\.variables\.valid`, `backend-caps.md` 11.8).
Each says "Image comparison failed":

```text
dEQP-GLES3.functional.shaders.qualification_order.variables.valid.interp_storage
dEQP-GLES3.functional.shaders.qualification_order.variables.valid.interp_storage_precision
dEQP-GLES3.functional.shaders.qualification_order.variables.valid.invariant_interp_storage
dEQP-GLES3.functional.shaders.qualification_order.variables.valid.invariant_interp_storage_precision
dEQP-GLES3.functional.shaders.qualification_order.variables.valid.invariant_storage
dEQP-GLES3.functional.shaders.qualification_order.variables.valid.invariant_storage_precision
dEQP-GLES3.functional.shaders.qualification_order.variables.valid.storage_precision
```

They make up the whole `variables.valid` group. Tree row 17 holds them,
counting non-comment lines of `tools/cts/am62p-gles3-tree.txt` from 0. The
TSV header says rows 11-21 are `build b25997e4, run 2026-11-27`.

**What dEQP does.** The source is VK-GL-CTS `93bca01`,
`data/gles3/shaders/qualification_order.test`, group `valid`, and
`modules/glshared/glsShaderLibraryCase.cpp`.

- Each case has `values {}` and no attribute besides `dEQP_Position`.
- The vertex shader writes `x0 = 1.0` and `x1 = 2.0` to two scalar outputs.
- The fragment shader writes `(x0 + x1 + x2) / 3.0` to R, G and B. `x2` is a
  `uniform float` that is never set, so it is 0.
- The case passes only if every pixel of the viewport reads exactly 255 in R,
  G and B (`checkPixels`, line 967: all white, with no tolerance).
- `x0` is always `centroid`, with `smooth`, `invariant`, both or neither, and
  with `lowp` or no precision.
- `x1` is `flat` in the three `*interp_storage*` cases and smooth in the
  other four.

Position w is 1 and both varyings are constant over the quad. So the place
where a plane is evaluated (pixel centre or centroid) cannot move the value. A
pixel other than white means a register or coefficient was read wrong, or a
fragment was not shaded.

## 2. What the board already shows at the same build (b25997e4, row 11)

| case (mustpass, not in the TSV, so Pass) | varyings in the FS | what it rules out |
|---|---|---|
| `linkage.varying.interpolation.centroid` | 1 x `centroid in vec4` | centroid iteration on a 1x target |
| `linkage.varying.rules.differing_interpolation_2` | 1 x `centroid in mediump float` (the VS writes it `smooth`) | **scalar** centroid iteration (`ditrp mode=centroid count=1`) |
| `linkage.varying.rules.readback_1` | 2 x `in float` (smooth) | two scalar varyings in adjacent coefficient sets |

So the premise in `backend-caps.md` 11.8, "a scalar centroid varying beside
another scalar varying", still separates the passes from the fails. Neither
half fails on its own.

## 3. Host evidence (tip `ad2b6cc9`)

A scratch test compiled and linked the 7 programs verbatim (`dEQP_Position`
renamed) with the online back end. It used
`--no-default-features --features host-test,gpu-powervr,shaderc-online` and
ran the programs in the byte model `tests/usc_exec` (`run_binary`).

1. **Varying packing.** In all 7, the vertex record has
   `[(loc 0, 1 comp, vtxout 4), (loc 1, 1 comp, vtxout 5)]` and
   `vs_output_size` 6. After the link, the fragment record has the same
   `vtxout_off` values and `uses_w` 1. The vertex program in the byte model
   writes 1.0 to output 4 and 2.0 to output 5.
2. **Iterator (USC).**
   - `x0`: `ditrp dest=0 coff=4 woff=0 mode=centroid count=1`.
   - `x1` smooth: `ditrp dest=1 coff=8 woff=0 mode=pixel count=1`.
   - `x1` flat: the move from its plane's C coefficient (`fragment.rs`
     `flat_group`, `coff + 2`).

   The coefficient sets are W at 0, then 4 and 8, as
   `fragment::coefficient_offsets` gives.
3. **Twin comparison.** Each program was also built with every `centroid `
   removed and the two fragment listings compared line by line. The only
   difference is that one `ditrp` field: `mode=centroid` against
   `mode=pixel`. The listings have the same length (145 lines for flat `x1`,
   182 for smooth `x1`), with the same registers and the same coefficients.
4. **Against Mesa's pco.** I wrote Vulkan-GLSL transcriptions of the two
   fragment-shader shapes (`smooth centroid` + `flat`, and `centroid` +
   smooth). The uniform was dropped, because pco_shc asserts on a descriptor
   it was not given. I compiled them with `glslangValidator -V` and the
   recorded `pco_shc` (Mesa 26.2.1), and decoded the words with our
   `decode_group`. pco gives the same coefficient layout (`fs_in` VAR0 at
   cf4, VAR1 at cf8) and the same iterator words:
   `ditrp dest=0 coff=4 woff=0 mode=centroid count=1`, then
   `ditrp dest=1 coff=8 woff=0 mode=pixel count=1` for a smooth `x1`.
   Mesa does nothing else for centroid, either in the coefficient program or
   in the USC kick. In `src/imagination/vulkan/pvr_arch_pipeline.c`, the
   DOUTI per varying carries only shademodel, perspective, size and offset,
   and every DOUTU uses `SAMPLE_RATE_INSTANCE`.
5. **The PDS side.** `x0`'s record `interp` is 3 (`ir::INTERP_CENTROID`,
   `reflect.rs:555-559`, where `flat` wins over `centroid`). In
   `src/pvr/backend.rs`, the coefficient (DOUTI) block maps 3 to its default
   arm, `PVR_PDS_COEFF_SMOOTH`. That is the twin's DOUTI, as in Mesa.
6. **Uniform and arithmetic.**
   - `x2` is read from `shared s0`. The GL core `calloc`s the uniform block
     (`src/gl/program.rs`, `p.fs_uniforms`). `uniform_sh_dwords` 0 means the
     whole block, so `uniform_split` DMAs all of it, and `x2` is 0 on the
     board too.
   - `/ 3.0` is `fmul` by hardware constant 152 = `0x3eaaaaab`, the correctly
     rounded 1/3 (`prepare.rs` `CONST_REGS`). In f32, 3.0 times that is
     exactly 1.0.
7. **Pixel result.** In the byte model the setup was: W = 1.0 at coefficient
   0, `x0 = 1.0` and `x1 = 2.0` at their planes, and zero shared registers.
   All 7 write pixel output 0 = `0xffffffff`, which is white.

**Ruled out on the host:** varying packing, coefficient offsets, the iterator
words (identical to pco's), the centroid flag (the USC mode only, as in Mesa),
the DOUTI class, the unset uniform, and the 1/3 constant.

## 4. Candidates left (board only), in order

1. **The iteration after a centroid `ditrp`.** These 7 are the only cases
   where a centroid `ditrp` is followed by a second iterator: another `ditrp`
   at the next coefficient set, or a flat move from it. In
   `differing_interpolation_2` the centroid read is the program's only
   varying. If the hardware's centroid iteration overruns into, or is
   reordered against, the next iteration's destination or coefficients, `x1`
   or `x0` comes out wrong. pco would emit the same words, so this would be a
   hardware hazard that Mesa shares or that Mesa avoids by scheduling. The
   fence groups between iterations (`flush_p0`, `br.next`, `wdf`) are copied
   from pco's form.
2. **Edge pixels of the quad.** Centroid is evaluated at a covered sample. If
   the ISP's sample positions or coverage state for a 1x draw leave some
   pixels on the quad's diagonal with no centroid, those pixels read
   something other than the plane's value. The linkage cases compare each
   pixel against a tolerance and write a boolean colour, so a few wrong
   pixels might be hidden there but not here. That is unlikely, because a
   boolean that is wrong also fails, but it is not excluded.
3. **Not centroid at all.** The `values {}` path and the exact-white check
   reach no other shader-library case that the board passes with varying
   arithmetic. A plain `smooth` twin of these programs has never run on the
   board.

## 5. What settles it, and the fix's paths

**Board experiment.** It goes in a later item on the AM62P, one session with
the console.

- (a) Get row 17's dEQP log at the build that board-b runs (rows 11-21 at
  `472630df`) and read the result image's pixel values. 170 grey (2/3)
  means `x0` read 0, and 85 (1/3) means `x1` read 0. A pattern only on the
  diagonal points at candidate 2.
- (b) Run four gl_test or dEQP-style programs on the same quad:
  - the case as is;
  - with `centroid` removed (the twin);
  - with the varyings declared in the other order (`x1` first);
  - with `x1` removed (`x0 * 3.0`).

  If only the original and none of the others fails, candidate 1 is
  confirmed. If the twin fails too, candidate 3 is confirmed and centroid is
  not the cause.

**Fix paths, by candidate.** None of them needs a `src/gl` change.

- Candidate 1:
  - Option one: iterate the centroid inputs **last** in
    `src/shaderc/online/powervr/chain.rs`, in the `iterated` list built at
    lines 920-933 (keep the coefficient layout and change only the order the
    `ditrp`s are issued in).
  - Option two: put a `wdf` fence after a centroid `ditrp` in
    `src/shaderc/online/powervr/fragment.rs` `iterate_group`.

  Pin it with a host test beside `tests/es3_pvr_centroid.rs`, for example
  `qualification_order_valid_programs_iterate_both_scalars`, which asserts
  the order or fence and that the byte model writes white.
- Candidate 2: the 1x ISP sample and coverage state the PowerVR back end
  writes for a draw, in `src/pvr/backend.rs` and `driver/src/pvr/`.
- Candidate 3: re-triage as a non-centroid cause, with no change to the
  centroid path.

## 6. Code locations

| what | where |
|---|---|
| centroid qualifier to `INTERP_CENTROID` (flat wins) | `src/shaderc/online/reflect.rs:555-559`, `:578-582` |
| varying list, coefficient offsets, `IterMode::Centroid`, iteration order | `src/shaderc/online/powervr/chain.rs:863-933` |
| `coefficient_offsets` (W's 4, then 4 per component) | `src/shaderc/online/powervr/fragment.rs:241` |
| `ditrp` with `iter_mode` centroid / pixel, and its fences | `src/shaderc/online/powervr/fragment.rs:192-237` (`iterate_group`) |
| flat varying from coefficient C (`coff + 2`) | `src/shaderc/online/powervr/fragment.rs:178` (`flat_group`) |
| DOUTI class per varying (3 to `PVR_PDS_COEFF_SMOOTH`) | `src/pvr/backend.rs`, the "coefficient (DOUTI) PDS for the FS varyings" block |
| uniform DMA prefix | `src/pvr/backend.rs` `uniform_split` |
| layout pin (centroid equals its twin) | `tests/es3_pvr_centroid.rs` |
| Mesa's centroid: the USC mode only | Mesa 26.2.1 `src/imagination/pco/pco_trans_nir.c:443-447`; `vulkan/pvr_arch_pipeline.c` (FS coefficient setup) |

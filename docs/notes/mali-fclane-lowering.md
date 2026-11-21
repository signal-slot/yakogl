<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): lowering the `gl_FragColor` lane refusal to Mesa's words: design

**HOST-ONLY design.** Plan #387 item `fclane-design`, for the items
`fclane-lower-1` and `fclane-lower-2` (issue #7, ROADMAP stage 5). No change
under `src/` or `tests/` is part of it. It builds on
[`mali-deqp-fclane.md`](mali-deqp-fclane.md) (the 20 references, the shapes,
the case counts of its section 2, `MaliError::ColourForm`); read that note's
sections 2 and 3 first. Counts here are that note's first-refusal cases at
`9ac31f30` (HOST-ONLY), never a render verdict. The G310 needs no separate
work ([`mali-g310-vs-g610.md`](mali-g310-vs-g610.md)).

What this note adds to the earlier one:

1. the exact refusal site of each of the 20 shapes (the earlier note's
   section 4 named only "likely" sites), measured once by instrumenting
   `Err(MaliError::ColourForm)` with `line!()` and printing the operands at
   the three sites that fire, then reverting (the recipe is in section 3);
2. the Mesa instruction sequence of each shape, read from the `text` of each
   reference's `fs.fragment` region in `tools/mali/testdata/deqp_fclane_*.expect.json`
   (the Mesa 26.2.1 disassembly `pan_ref_diff.py extract --draw` stores, the
   same words `tools/mali/va_disasm_ref.sh` / `isa_ref.py` print);
3. the mechanisms, and a two-step implementation design.

## 1. The common frame of all twenty (G610 fragment, mediump `gl_FragColor`)

Every reference ends the same way, and none of it needs a change:

```
  ...colour words in r0:r1 (two fp16 pairs: (x,y) and (z,w))...
  ATEST.wait0 @r60, r60^, <alpha source>, atest_datum.w0
  BLEND.slotN.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
  IADD_IMM.i32 r48, 0x0, #0x0
  BRANCHZI.eq.absolute 0x0, blend_descriptor_0.w1
```

- `work_reg_count` 32, `preload` `0x3000000000000000` in all twenty.
- The ATEST alpha source is `0x3F800000` (1.0, an opaque constant alpha:
  the mat shapes, `vec3_local`, the bcast shapes), `rN.h1` (the alpha half of
  the last word, computed: the `op_pair_*` with a `FMA.v2f16` scale-and-bias),
  or `rN.h0` (`struct_fields`, `tex_coord_ramp`, `varying_merged`: the word
  whose half is the alpha, placed between the lane code and the `MKVEC`).
  The compiler already picks it (`select` and the ATEST code after the
  colour); a lowering only has to leave the node in the colour word it names.
- The `BLEND` slot number varies (`slot0`/`slot1`/`slot2`): the scheduler's
  slot rotation, not part of the lowering.
- `LD_VAR_BUF_IMM.f32...v4/v3/v2` loads the *lanes read*, in one or two
  loads (`mat_cols_sum`: `.v2` at index 0x20, `.v4` at 0x0, `.v4` at 0x10).
  The compiler's load layout (`layout`, `index`, `FsVarying`) is not touched.

## 2. The 20 Mesa sequences, per shape

`v` is the load's registers (`r0..`), words are `MKVEC.v2i16` pairs of two
fp16 halves. "Narrow alone" is `FADD.f32 rX.h0, rY^, 0x0.neg` (one fp32
value converted into the low half; Mesa's `f2f16` of a scalar,
`bi_lower_*`). "Pair convert" is `V2F32_TO_V2F16 rD, a, b` (two fp32 values
into one word). The first column is the shape's reference name
(`deqp_fclane_<name>`), then cases; mechanism letters are section 4's.

| Shape (cases) | Mesa words (fragment, in order) | Mech |
|---|---|---|
| `op_pair_yz` (112) | `LD v4 r0:r3`; `FMA.f32 r0, r3^, u2.w0, 0x0.neg` (w * 57.2958), `FMA.f32 r1, r1^, u2.w0, 0x0.neg` (y); narrow alone r0, r1; **`MKVEC.v2i16 r0, 0x0.h0, r0^.h0`** (word 0 = (0, res.x'): the constant 0.0 *first*); `FMA.v2f16 r0, r0^, u2.w1, 0x38000000.h11` (`*0.04 + 0.5`, scale in the FAU, 0.5 inline); `MKVEC.v2i16 r1, r1^.h0, u3.w0.h0` (word 1 = (lane, alpha 1.0 from the FAU)); `FMA.v2f16 r1, r1^, u2.w1, 0x38000000.h11`; `ATEST ... r1.h1` | A |
| `op_pair_xyz` (49) | `LD v4`; three `FMA.f32 ..., u2.w0, 0x0.neg` in the order z, x, y (`r2`, `r0`, `r1`); three narrows in the same order; `MKVEC.v2i16 r0, r2^.h0, r0^.h0` (z,x); `FMA.v2f16 r0, r0^, u2.w1, 0x38000000.h11`; `MKVEC.v2i16 r1, r1^.h0, u3.w0.h0` (y, alpha); `FMA.v2f16 r1`; `ATEST ... r1.h1` | A |
| `op_pair_xyzw` (115) | `LD v4`; four `FMA.f32` in the order y, z, w, x; four narrows; `MKVEC r1, r1^.h0, r2^.h0` (y,z), `FMA.v2f16 r1`, `MKVEC r0, r3^.h0, r0^.h0` (w,x), `FMA.v2f16 r0`; `ATEST ... r0.h1`; `MOV.i32 r2, r0^; MOV.i32 r0, r1^; MOV.i32 r1, r2^` (the words swapped into `r0:r1` for the BLEND source: the first-built word is colour word 0 but sits in `r1`) | A |
| `op_pair_x` (8) | `LD`; `MOV.i32 r1, u2.w0`; `FMA_RSCALE.f32 r0, r0^, 0x3F800000, 0x0.neg, r1^`; `F32_TO_S32 r1, r0`; `FEXP.f32 r0, r1^, r0^` (exp: the arith family's lowering of the lane); narrow alone r0; **`U16_TO_U32 r0, r0^.h0`** (word 0 = (lane, 0): `MKVEC` with 0 second is the `lower_mkvec_swz` form); `FMA.v2f16 r0, r0^, 0x38000000.h11, 0x0.neg` (scale 0.5 only); `ATEST ... 0x38000000.h1` (alpha 0.5, a constant of the second word); `MOV.i32 r1, 0x38000000` (word 1 = (0, 0.5) as one constant) | A (+ arith-lower-1's exp) |
| `op_pair_other` (8) | `sequence.side_effects`: a bool compare as a mask (`FCMP_OR.v2f16.lt.m1 r0, 0x0, r0^.h00, 0x0`, `S16_TO_S32`, `CSEL.u32.eq r0, r0^, 0x0, 0x0, 0x3F800000`), `FADD.f32` of the bool float to two lanes, `F32_TO_S32.rtz`, `S32_TO_F32` round trips, each lane narrowed alone, `MKVEC r1, 0x0.h0, r1^.h0`, `FMA.v2f16 .. 0x38000000.h11, 0x0.neg` per word; `ATEST ... r0.h1`; three `MOV.i32` to swap the words | E (needs intval) |
| `mat_cols_sum` (51) | three `LD_VAR_BUF_IMM` (`.v2` index 0x20 = `v_coords` and the scalar; `.v4` x2 = the matrix columns); `FADD.f32 rN, rN^, r1` adds the scalar `v_coords` to each of the nine matrix lanes (`res = m + s`), then the colour sums as fp32 `FADD.f32 r1, r2^, r5^` ... (`res[0]+res[1]+res[2]` per lane x,y,z); each sum narrowed alone (three `FADD.f32 rN.h0, ..`); `MKVEC.v2i16 r1, r1^.h0, r2^.h0` (x,y); `MKVEC.v2i16 r2, r0^.h0, u2.w0.h0` (z, alpha 1.0 from the FAU, `3c003c00`); `ATEST ... 0x3F800000`; `MOV.i32 r0, r1^; MOV.i32 r1, r2^` | A |
| `mat_elem_pairs` (42) | `LD .v4` and `LD` (scalar); `FADD.f32` x4 (`m[c] + s` as needed: the four lanes of `res[0][0], res[0][1], res[1][0], res[1][1]`), `FADD.f32 r0, r2^, r0^` (the sum lane); three narrows alone; `MKVEC r1, r1^.h0, r2^.h0` (x,y), `MKVEC r2, r0^.h0, u2.w0.h0` (z, 1.0); `ATEST 0x3F800000`; two `MOV.i32` | A |
| `mat4_swz_sum` (23) | `LD` (the scalar `v_coords`); constant matrix + scalar: `FADD_IMM.f32 rN, r0, #bits` for the non-trivial columns (`1.0`, `0.2`, `-0.3`: one per constant), `FMA.f32 r0, r0^, 0x40000000, r0^` (`s + s`, the 2.0 product's late rule `fadd(a, a)`), `FMA.f32 r1, r1^, 0x44000000.h1, 0x0.neg`; `FADD.f32` sums; three narrows; the same two `MKVEC` (x,y), (z, alpha 1.0 FAU); `ATEST 0x3F800000`; two `MOV.i32` | A |
| `mat_vec_lanes` (6) | `LD .v2` (v_coords) and `LD .v4` (mat2 rows); `FMA.f32` chains: `FMA.f32 r3, r1, r3^, 0x0.neg` then `FMA.f32 r2, r0, r2^, r3^` (`v * m` lane x) and the lane y alike; two narrows alone; `MKVEC.v2i16 r0, r1.h0, r0^.h0` (word 0 = (x', y') where the lane `x` is read twice: `res.x` appears in colour lane 0 and 2: the *same* narrowed register `r1.h0` is the low half of word 0 **and** (`r1^`, last read) of word 1); `MKVEC.v2i16.wait r1, r1^.h0, u2.w0.h0` (word 1 = (x', alpha)); `ATEST 0x3F800000` | A |
| `rnd_vec4` (66) | `LD .v3`; `V2F32_TO_V2F16 r0, r1^, r0^` (the pair convert of two lanes **in swapped order**: r1 low, r0 high), narrow alone r2 (z); `MKVEC r2, r0.h0, u2.w0.h0`; `FADD.v2f16 r0, r2^, r0^.h10` (the swizzled operand `.h10`); `MKVEC r2, r1.h0, u2.w1.h0`; `MKVEC r1, r1^.h0, u3.w0.h0`; `FADD.v2f16 r1, r1^, r2^` (mixed fp16 pair arithmetic: a per-case arrangement) | D |
| `vec3_local` (52) | `LD .v4`; two `V2F32_TO_V2F16` (x,y) and (z,w); `FMA.v2f16 r2, r0.h10, 0x38000000.h11, 0x0.neg` (`* 0.5` of the swapped pair), `FMA.v2f16 r3, r1, ...`; `MKVEC.v2i16 r2, r2^.h0, r3.h0`; `FADD.v2f16` sums with `.h00` / `.h11` swizzles; `FMA.v2f16 r1, 0x34000000.h11, r1^, r2^` (`* 0.25`); alpha `MKVEC.v2i16 r2, r0^.h0, u2.w0.h0`; `ATEST 0x3F800000`; two `MOV.i32` | D |
| `tex_coord_ramp` (20) | `LD .v2`; one `V2F32_TO_V2F16 r0, r0^, r1^` (x,y); `FADD.v2f16 r1, r0.h00, r0.h11` (x+y: **one lane read of each half of the same word**); `FADD.v2f16 r0, r0^.h00, r0^.neg.h11` (x-y); `FMA.v2f16` x3 (`*0.5`, `+0.5`); `MKVEC r2, r1.h0, r2^.h0`; `FADD.v2f16 r1, 0x3C000000.h11, r1^.neg.h00` (1-f0); `MKVEC r1, r1^.h0, r0.h0`; `ATEST ... r0^.h0`; `MOV.i32 r0, r2^` | D |
| `varying_merged` (18) | `LD .v3` (the link merged the varying's w lane with x: three lanes loaded); `V2F32_TO_V2F16 r0, r0^, r1^` (x,y); narrow alone r2 (z) into `r1.h0`; `ATEST ... r0.h0`; **`MKVEC.v2i16 r1, r1^.h0, r0.h0`** (word 1 = (z, x): the merged `w` is the pair-converted `x`'s `.h0`); `BLEND @r0:r1` | B |
| `struct_fields` (14) | `LD .v3`; `V2F32_TO_V2F16 r0, r0^, r1^`; narrow alone r2 (z) to `r1.h0`; **`S32_TO_F32 r2, u2.w0`** (the `int c` uniform member) and `FADD.f32 r2.h0, r2^, 0x0.neg`; `ATEST ... r2.h0`; `MKVEC r1, r1^.h0, r2^.h0` (word 1 = (z, float(c))) | B |
| `tex_zw_lane` (4) | `MOV.i32 r0..r3` (coordinates (0,0), the sampler word `u3.w0`), `TEX_SINGLE` ... `@r0, @r0:r1` (the texel in `r0`: only z,w are read); `S32_TO_F32 r1, u2.w0`, `S32_TO_F32 r2, u2.w1`; **`V2F32_TO_V2F16 r1, r1, r2^`** (two integer-uniform floats pair-converted, word 0 = `fTest.xy`); `ATEST ... r0.h1`; `MOV.i32 r2, r0^; MOV r0, r1^; MOV r1, r2^` (texel word is colour word 1) | B |
| `bcast_compare_int` (11) | no varying (the vertex shader's `1.0` is constant-linked away); `MOV.i32 r0, 0x0`; `ICMP_OR.u32.eq.m1 r0, u2.w0, u2.w1, r0^` (compare as a mask, uniform and literal both FAU words); `MUX.v2i16 r0, 0x0, 0x3C000000.h11, r0^.h00` (mask to 0.0/1.0); `ATEST ... 0x3F800000`; **`IADD.v2u16 r1, r0.h00, 0x0`** (the lane in both halves); `MKVEC.v2i16 r2, r0^.h0, u3.w0.h0` (lane, alpha 1.0); `MOV.i32 r0, r1^; MOV.i32 r1, r2^` | C |
| `bcast_compare_bool` (10) | `MOV.i32 r0, u2.w0` (the bool word), the same `MUX.v2i16`, `ATEST`, `IADD.v2u16 .h00`, `MKVEC ... u2.w1.h0`, two `MOV.i32` | C |
| `bcast_compare_float` (39) | `multiple_basic`: fp16 compares of float uniforms (`FADD_IMM.v2f16`, `FMAX.v2f16 .abs.h00/.h11`, `FCMP_OR.v2f16.lt.m1`, `S16_TO_S32`), int/bool uniform masks (`ICMP_OR`/`ICMP_AND.u32.eq.m1`, `CSEL.u32.gt`), the product of 0/1 floats as `LSHIFT_AND.i32 r0, r0^, 0x0.b00, r1^` (AND of masks), the `MUX.v2i16`, `MKVEC.v2i16 r1, r0.h0, u11.w0.h0` and `IADD.v2u16.wait0126 r0, r0^.h00, 0x0` (**the broadcast here is in a different order**: the `MKVEC` before the `IADD`, `ATEST` after both, and no `MOV.i32` pair: the colour words are already in `r0:r1`) | C |
| `bcast_compare_array` (26) | `FADD_IMM.v2f16` pairs of the uniform array's elements against literals, `FMAX.v2f16 .abs.h00,.abs.h11` reductions, `FCMP_OR.v2f16.lt.f1 r0, r0^.h00, u5.w0, 0x0` (compare with the 0.05 FAU word), `ATEST`, `IADD.v2u16 r1, r0.h00, 0x0`, `MKVEC r2, r0^.h0, u5.w1.h0`, two `MOV.i32` | C |
| `bcast_const` (4) | `MOV.i32 r0, 0x0`; `ICMP_OR.u32.eq.m1 r0, 0x7060504.b3, u2.w0, r0^` (a constant-folded literal byte compared with a uniform); `MUX`, `ATEST`, `IADD.v2u16 .h00`, `MKVEC ... u2.w1.h0`, two `MOV.i32` | C |

The `Mech` column's letters (section 4): **A** fp32 lanes narrowed alone and
packed (`MKVEC`), **B** two varying/convert halves of different sources
packed, **C** the broadcast of one computed bool-float lane (the compare
lowering is another family's), **D** fp16 pair trees with `.hN` swizzle
reads, **E** needs the integer/boolean family first.

### 2.1 What every `A` word has in common

For the 8 `A` shapes the colour is `highp` fp32 arithmetic, **narrowed one
lane at a time and then packed**, never converted as an fp16 pair tree:

1. each fp32 lane op is a scalar `FMA.f32` / `FADD.f32` / `FADD_IMM.f32`
   (`alu32` of `nir_fs.rs`; no `.v2f16`), in NIR's per-lane order;
2. each lane is narrowed alone: `FADD.f32 rX.h0, rX^, 0x0.neg` (the same
   word `narrow_stored` makes for the integer cases, `F2f16` of width 1);
3. the packing is `MKVEC.v2i16 word, lo.h0, hi.h0` (lane, lane / lane, FAU
   constant alpha / `0x0.h0`, lane) or `U16_TO_U32` (lane, 0.0);
4. when the shader does `color = color * s + b` (the operator harness, 292
   cases) the **post-step runs on the packed fp16 words** (`FMA.v2f16 word,
   word^, <FAU scale>, 0x38000000.h11` (bias 0.5 inline), or the bare scale
   `.. 0x0.neg`) and the alpha half of the last word feeds ATEST;
5. the FAU holds the fp32 scale (`u2.w0`), the fp16 pair of the scale/bias
   (`u2.w1`, `0x291f291f` = 0.04 in both halves), and the alpha constant
   word (`3c003c00`): the compiler's FAU builder (`fau.rs`) already
   builds such words for the Pack/Lane forms (not re-checked for this note; confirm when step 1 starts).

Sub-shapes the five operator references (`op_pair_*`) do not separate: the
no-scale form (`color = res` without `* s + b`) drops the `FMA.v2f16` and
the ATEST reads the word's half directly; counts of the three post-steps in
the earlier note's section 2 (`scale-and-bias` / `scale only` / `none`).

## 3. Where the compiler raises it (measured)

All sites are in `src/shaderc/online/mali/nir_fs.rs` (line numbers at
`3497799a`). Method, to repeat in 5 minutes and **to run again at the start
of each lowering item** (the sites move as other items land in `nir_fs.rs`):

```sh
sed -i 's/Err(MaliError::ColourForm)/{ eprintln!("CF@{}", line!()); Err(MaliError::ColourForm) }/' \
    src/shaderc/online/mali/nir_fs.rs
# print the case name in refusals_pinned's loop: eprintln!("NAME {name}");
cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table \
    --test mali_deqp_fclane refusals_pinned -- --nocapture 2>&1 | grep -E "NAME|CF@"
git checkout src tests          # never commit the instrument
```

Three sites fire for the twenty (all inside `select`'s `vectorise` /
`lane_tree` calls, the colour's pair graph; `layout` is not reached):

| Site | What it is | Shapes (operands seen) |
|---|---|---|
| `vectorise`, the `else { return Err(ColourForm) }` at **8735** | two lanes of one load that are neither an aligned pair, a swapped pair, one lane twice, nor two lanes of a `whole` load: `i/2 != j/2` and `!whole` (e.g. y,z of a v4) | `op_pair_xyz` (a=(0,2), b=(0,0)), `op_pair_xyzw` ((0,1),(0,2)), `mat_elem_pairs` ((0,0),(1,0)), `mat_vec_lanes` ((1,0),(2,0)), `varying_merged` ((0,2),(0,3)), `vec3_local` ((0,1),(0,2)) |
| `vectorise`, the final `_ => return Err(ColourForm)` at **8799** | a pair whose halves are no load lanes: a computed lane, or two lanes of a node of width 2 | `op_pair_yz` (a = Const 0.0, b = `Fmul(load.w, 57.2958)`), `op_pair_x` (a = `Fexp2`, b = 0), `mat_cols_sum` ((1,0),(1,1): a two-lane node), `mat4_swz_sum` (`Fmul(l, 2.0)`, `Fadd`), `rnd_vec4` (`Fmul`, `Fadd(Const, Const)`), `tex_coord_ramp` (`Fmul`, `Fadd`), `struct_fields` (a = load lane z, b = `I2f(uniform)`), `tex_zw_lane` (`I2f`, `I2f`), `op_pair_other` (a two-lane node), `bcast_compare_array` (`Bcsel(..)`, 1.0) |
| `lane_tree`'s last arm, **9525** | a one-lane operation the tree has no arm for: here `B2f` (a bool as a float, `a ? 1.0 : 0.0` of a compare) | `bcast_compare_bool`, `_float`, `_int`, `bcast_const` |

How the first column reads, from the code:

- **8735 is mostly a symptom of mechanism A.** The `op_pair_*` and `mat_*`
  colours are `highp` fp32 `Fmul`/`Fadd`/`Ffma` lanes *stored to a mediump
  output/local*. Nothing narrows them, so `vectorise` meets `(Fmul, Fmul)` /
  `(Fadd, Fadd)` of the two colour lanes and takes the **fp16 pair arm**
  (`P::Op(Alu::Fmul, [vectorise(sa[0], sb[0]), vectorise(sa[1], sb[1])])`):
  it recurses into the *operand* pair (z, x) of the loads and dies at 8735.
  The pair arm is right for a mediump tree (Mesa's `f2fmp` lowered the whole
  tree to fp16) and wrong for these: a `highp` operand stops lower_precision
  (`handle_precision` `CANT_LOWER`, `lower_precision.cpp:304-305`), the tree
  stays fp32 and only its result is narrowed. That is exactly the mechanism
  the compiler already has for the integer and frag-coord cases (below).
- 8799 with `a = Const(0)` (op_pair_yz): the existing `(Some((op,_)), None)`
  arm handles `(computed lane, constant)`; the mirrored `(constant, computed
  lane)` has no arm.
- 9525: no `Alu::B2f` arm in `lane_tree`; `bcast_*` need the compare's mask
  first.

### 3.1 The mechanism the compiler already has (and where to generalise it)

`Fs::narrow_stored` (`nir_fs.rs` about 2763) makes the `F2f16` node, width 1,
of an fp32 value that is stored to a declared mediump local; it is called at
about **5577**, under

```rust
if !prec.highp && !temporary && !highp_local {
    ... if f.fc32_float(*t) || f.int_float(*t) { *t = f.narrow_stored(*t, at)?; }
}
```

with `fc32_float` (about 2553) true only for fp32 arithmetic reaching the
highp `gl_FragCoord`'s island (`reaches_fc`), and `int_float` for `i2f`/`u2f`.
`Fs::narrowed` (2581) then recognises the `F2f16` node as a fp16 leaf, and the
pair graph already packs such lanes: `Leaf::Zext` (`U16_TO_U32`), `Leaf::Lane
{ x, half }` (`MKVEC x.h0, <const>.h0`), `Leaf::Pack { x, y }` (`MKVEC x.h0,
y.h0`) through `vectorise`'s arms for `(computed, Const)`, `mixed_word`,
`a != b with lane ops` and `lane_word`. The pinned references of that path
are `deqp_intval_opfs_*_mediump` (`S32_TO_F32` then `FADD.f32 r0.h0`, then
`U16_TO_U32` and `FMA.v2f16`, `tests/mali_deqp_intval.rs`) and
`fsarith_218_shadowcmp`.

The same must happen for **two more places an fp32 lane is narrowed**:

1. the store of a computed fp32 value (a `highp` local's, or a direct
   expression) **to a mediump local** (`color.yz = res`: the operator harness);
2. the store **to the colour output itself** (`gl_FragColor = vec4(res...)`,
   the `mat_*` and `mat_vec_lanes` shapes): `nir_lower_mediump_io` converts
   the colour at its store (`bifrost_nir.c:914`, `nir_lower_mediump.c:102-104`),
   after everything else (`conversion_order`'s note), so the narrowing nodes
   of the colour are created where the colour is taken at `StoreOutput`
   (about 5598, `ir::Op::StoreOutput`).

## 4. Mechanisms

| | Mechanism | Shapes | Cases |
|---|---|---|---:|
| **A** | An fp32 computed lane (any `alu32` chain over varying lanes, uniforms, constants) narrowed alone and packed with `MKVEC`/`U16_TO_U32`, the mirrored `(0.0, lane)` word, and the `FMA.v2f16` post-step | `op_pair_yz` 112, `op_pair_xyzw` 115, `op_pair_xyz` 49, `op_pair_x` 8, `mat_cols_sum` 51, `mat_elem_pairs` 42, `mat4_swz_sum` 23, `mat_vec_lanes` 6 | **406** |
| **B** | Two halves from different sources, each alone: a varying lane converted alone then the merged/other half (`varying_merged`: (z, x-of-pair)), an int uniform's float (`struct_fields`: (z, `S32_TO_F32` narrowed)), a pair conversion of two int-uniform floats (`tex_zw_lane`: `V2F32_TO_V2F16 r1, r1, r2^`) | `varying_merged` 18, `struct_fields` 14, `tex_zw_lane` 4 | 36 |
| **C** | The broadcast of one computed bool-float lane: `IADD.v2u16 r1, r0.h00, 0x0` / `MKVEC (lane, alpha)` / `MOV.i32` pair, and the `B2f` arm in `lane_tree`; the compare producing the mask is the intval/arith families' | `bcast_compare_{int,bool,float,array}` 229, `bcast_const` 4 | 233 |
| **D** | fp16 vector trees over converted pairs with `.hN` operand swizzles and mixed word arrangements | `rnd_vec4` 66, `vec3_local` 52, `tex_coord_ramp` 20 | 138 |
| **E** | The integer/boolean machinery first (a bvec2/ivec2 side-effect sequence) | `op_pair_other` | 8 |
| | Total | 20 references | **821** |

406 + 36 + 233 + 138 + 8 = 821 (the earlier note's total).

## 5. The two-step design

The split follows case count per mechanism and what one two-hour agent
session can hold. Mechanism A is the single largest and is self-contained
(no other family's lowering is needed to reach the packed word); it is
step 1. Everything else is step 2: C is the next largest but needs the
compare/integer lowering to exist first (see "Order and dependencies").

### 5.1 Step 1 (`fclane-lower-1`): mechanism A, 406 cases

**Goal.** `highp` fp32 lanes that reach the mediump colour compile to
Mesa's fp32 lane code, narrowed alone, packed by `MKVEC`, with the operator
harness's `FMA.v2f16` post-step.

**Changes (all `nir_fs.rs`, plus `mod.rs` for the error text only if a
refusal moves):**

1. **The narrowing predicate** (`Fs::fc32_float`, `Fs::fp32_lane`, new
   `Fs::wide_float`): add a predicate for an fp32 `Fadd`/`Fmul`/`Ffma`/
   `Fneg`/`Fabs`/`Fmin`/`Fmax`/`Ffloor`/... node (`bits == 32`) of a **mediump
   shader whose operands read a varying lane, a uniform or a constant** (the
   existing `fp32_lane` has `I2f | U2f | Fmul | Fadd | Ffma`, but only the
   integer's; it is also keyed on `!self.prec.highp`). Keep `fc32_float`
   for the frag-coord island (its tests pin that case) and OR the new
   predicate in the two calls below. **Do not** widen it by guessing: Mesa
   narrows `highp` values only where they meet a mediump destination; a
   `highp` operation whose all readers are `highp` is no colour lane.
2. **Narrow at the mediump local's store** (about 5577): `if f.fc32_float(*t)
   || f.int_float(*t) || f.wide_float(*t)` (the operator harness:
   `color.yz = res`; `mediump vec4 color` is a *declared* mediump local, so
   `!highp_local` holds). The temporaries rule (`temporary`) stays: the
   constructor's unnamed local keeps fp32 lanes until it is copied.
3. **Narrow at the colour output** (the `StoreOutput` arm about 5598 and
   `Fs::colour`): each lane of the output vec4 that is an fp32 node
   (`wide_float`) is wrapped by `narrow_stored` right at the store, in
   lane order (`nir_lower_mediump_io`: the converts follow all other code;
   `fsconv_cpq`/`fsconv_discw` pin that order in `conversion_order`'s
   comment). Only when `!prec.highp` (a `highp` colour output stays fp32:
   `mali_fs_arith`'s highp references pin that).
4. **`vectorise`, mirrored `(Const, computed lane)`** (the `(Some(..), None)`
   arm's twin around line 8527 and the `(None, None, Const, load lane)` arm
   at 8780): `(Const(0.0), F2f16 lane)` is `Leaf::Lane { x, half, first: true }`
   (new `first` field, as `Leaf::Mk` has: `MKVEC.v2i16 0x0.h0, x.h0`, the
   `op_pair_yz` word 0). `Form`/`form()` (8320) and `prod()`/`ADMITTED`:
   the new leaf's form is `Form::Mk` as `Lane`'s; if `f.prods` takes a
   production the operator tree's `FMA.v2f16` over it does not allow, add
   it to `ADMITTED` with the reference it comes from.
5. **`lane_tree`** (9338): the `F2f16` node of an fp32 chain is a one-lane
   operation: add `Alu::F2f16` as a leaf (`Leaf::Narrow(node)`) whose source
   is emitted by the existing fp32 emission (`alu32_lanes`/`op32`, the
   `FMA.f32 r, a, u.w0, 0x0.neg` forms already pinned by the `S32_TO_F32`
   cases) and which prints `FADD.f32 rX.h0, rX^, 0x0.neg`; this is what the
   integer path already produces through `narrowed()` (re-use, do not add
   a second narrowing). `vectorise`'s `(F2f16, F2f16)` pair must reach the
   `Pack` arm (the arm `a != b && alu_of(a).is_some() && ... has_lane_op`
   takes only lane ops: extend `has_lane_op` (9002) to an `F2f16` of an fp32
   chain) instead of the `(Fmul, Fmul)` pair arm, which sees `Alu::F2f16`
   (not `Fmul`) and so no longer recurses into the operands once the lanes
   are narrowed.
6. **Emission (`select`, 9830, and the `Word`/`word_of` code, 10457/10625):** a
   `Pack { x, y }` of two narrowed lanes is already emitted (cflow_74's
   `MKVEC.v2i16 r0, r1^.h0, r0^.h0`); `Lane { x, half, first: true }` prints
   `MKVEC.v2i16 r, 0x0.h0, r0^.h0` (copy `Mk { first: true }`'s emitter).
7. **Order (`conversion_order` 8829, `move_loads` 10551):** Mesa narrows the
   lanes in the **order the fp32 lane ops were made** (not the colour's lane
   order: `op_pair_xyz` makes z, x, y; `op_pair_xyzw` y, z, w, x: the
   shape's own swizzle), then packs. The narrowing nodes are made at the
   store in lane order (item 2/3), the lane ops before them by `alu32`'s
   order: that gives the references' order as long as the `F2f16` of a lane
   stays **directly after its source in `nir.order`** (as
   `narrow_stored`'s integer branch does: `insert_before(after)`; copy that).

**Refusal sites removed:** 8735 for `op_pair_xyz`, `op_pair_xyzw`,
`mat_elem_pairs`, `mat_vec_lanes` (the pair arm no longer sees fp32
`Fmul`/`Fadd`), 8799 for `op_pair_yz`, `mat_cols_sum`, `mat4_swz_sum`
(`(Const, lane)`, the narrowed pair) and `op_pair_x` up to the exp.

**Un-ignores** (move to `LOWERED` in `tests/mali_deqp_fclane.rs`, drop
`#[ignore]`, keep `refusals_pinned`'s list in step): `op_pair_yz`,
`op_pair_xyz`, `op_pair_xyzw`, `mat_cols_sum`, `mat_elem_pairs`,
`mat4_swz_sum`, `mat_vec_lanes` (7 references, 398 of the 406 cases).
`op_pair_x` (8) moves to its next refusal's text (`exp`'s fp32 form
`FMA_RSCALE.f32` / `F32_TO_S32` / `FEXP.f32` is `arith-lower-1`'s family) and
stays in `REFUSED` with that text, or `LOWERED` when `arith-lower-1` has
landed: check the order of the two items. `degrees` itself (a plain
`* 57.2958` in the three vector references) is the compiler's own `Fmul`
and needs no arith lowering.

**Host forecast.** About 398 to 406 cases of this note's section 2, all
`shaders.operator.*` (`binary` 63+57+24 and the others of the three vector
shapes: only the cases whose *lane operation* the compiler already has:
`degrees`, `radians`, `+`, `-`, `*`, `abs`, `min`, `max`, `floor`...) and
`shaders.matrix.*` (`mat_cols_sum` 51, `mat_elem_pairs` 42, `mat4_swz_sum`
23, `mat_vec_lanes` 6). The operator cases whose lane op is another family's
(`exp`, `sin`, ..., `ARITH`) turn into `ARITH` refusals, `arith-lower-1`'s:
measure with `tools/mali/deqp-host-measure.sh --filter shaders.operator` and
`--filter shaders.matrix` before and after (the earlier note's method, 5
minutes), and state the shift `FRAGCOLOR_LANE -> ARITH` in the commit.

**Pinned tests at risk** (run each after the change; the CI gate runs all):

- `tests/mali_deqp_intval.rs` (`opfs_*_mediump`: `narrow_stored` and
  `Leaf::Zext` of an `I2f`; the new `wide_float` must not change an
  integer's narrowing or its position);
- `tests/mali_deqp_mixprec.rs`, `tests/mali_fragcoord16.rs` and
  `fsarith_218_shadowcmp` (the frag-coord island; `fc32_float`'s users);
- `tests/mali_deqp_colourpair.rs` (every `Leaf::Mk`/`Lane`/`Pack` word and
  `ColourZeroHalf`: adding `first` to `Lane` and the `(Const, lane)` arm
  must keep `ck_lane_highpcolour`'s `MKVEC r1, 0x0.h0, r0.h0`),
  `tests/mali_glmark2_corpus.rs` and `tests/mali_fragment_random.rs`
  (glmark2's fragment shaders: any `highp` fp32 lane stored to a mediump
  output now narrows one by one: the refs that already compile must stay
  byte-equal; a new narrowing in a pinned shader means `wide_float` is too
  wide), `tests/mali_fs_arith.rs` (the `gl_FragColor lane other than`
  pinned refusals at 252 and 260: they must keep refusing or move with their
  commit), `tests/mali_deqp_uniarray.rs` (99, 170), `tests/mali_deqp_unitype.rs`
  (228, 234: `vec4(ivec4).xy`), `tests/mali_deqp_constop.rs`,
  `tests/mali_deqp_arith.rs`, `tests/mali_const_fold.rs`,
  `tests/mali_arith_const.rs`, `tests/mali_locals.rs` and the in-tree
  `src/shaderc/online/mali/tests.rs` (line 1314 `v.xzyw` refusal and 3640
  `fstex_cubeface5`: both are `ColourForm` refusals pinned *as refusals*; a
  broader narrowing must not make them compile with other words) and
  `fs_transc_tests.rs` (147).
- Generated/pinned files: no pinned reference changes (the lowering only
  makes refused shaders compile); `tests/reference/*.rs` and
  `tools/mali/testdata/` stay as they are, except `mali_deqp_fclane.rs`'s
  lists. If a pinned shader's words change, stop: the new predicate is too
  wide.

**Deliverable and verification.** `cargo test --test mali_deqp_fclane` with
the 7 references byte-equal (`mali_ref::compare`), the intval and colourpair
tests above, `cargo fmt --check` and `cargo clippy` on the crate with the
feature set of the test header; the CI gate does the rest (CLAUDE.md: no
`tools/check.sh` locally).

### 5.2 Step 2 (`fclane-lower-2`): mechanisms B, C, D and E, 415 cases

Order inside the step, by cost and by what each needs. B and D need
nothing from other items; C needs the compare lowering; E the integer one.

1. **B, 36 cases** (`varying_merged`, `struct_fields`, `tex_zw_lane`).
   - `varying_merged` (18): 8735 for the pair `(z, w)` of a compacted load
     whose `w` the link merged with `x`. The `index()` map (8142) gives the
     merged lane's slot; the pair is `(Splat z alone, Half-lane h0 of the
     pair (x,y))`, `MKVEC.v2i16 r1, r1^.h0, r0.h0`: use `mixed_word`'s
     `Leaf::Pack { x: Splat, y: Half-low }` with the ATEST read of `r0.h0`
     (the pair's own conversion, not a new one: `pair_read` must see the pair
     (x,y) as read because `w` aliases `x`). The vertex-side
     `VaryingDuplicate` path (`mali-deqp-varylink.md`) exists; this is the
     fragment side.
   - `struct_fields` (14): 8799 `(load lane z, I2f(uniform))`: the mirror of
     `mixed_word`'s `(computed, load lane)` (that arm is `(a computed, b
     lane)`; this is `(a lane, b computed)`): `Pack { Splat z, narrowed I2f }`,
     the `S32_TO_F32 r2, u2.w0` + narrow of step 1's integer path.
   - `tex_zw_lane` (4): `(I2f, I2f)` pair of `ivec4` uniform floats:
     `V2F32_TO_V2F16 r1, r1, r2^` (two fp32 values in one pair convert, the
     `Leaf::Wide` shape for computed fp32 values: a new `Leaf::Narrow2 { x, y }`)
     and the texel word as colour word 1; the texture-coordinate refusal
     may come first (`TEX_COORD`).
   - Un-ignores: `varying_merged`, `struct_fields`, `tex_zw_lane`.
2. **D, 138 cases** (`rnd_vec4`, `vec3_local`, `tex_coord_ramp`): the fp16
   pair-tree family, `vectorise` 8735/8799. Each needs `.hN` read of a
   converted word as an *operand* (`r0.h00`, `r0.h11`, `r0.h10`, `neg.h11`),
   i.e. `Swz` on `Leaf::Half` reads by lone-lane operations
   (`conv_form` / `pair_read`: "a varying lane converted both alone and as
   half of a pair"). `tex_coord_ramp` (20): allow a lane both as half of a
   pair and alone when Mesa does (`FADD.v2f16 r1, r0.h00, r0.h11`), the
   smallest of the three: do it first. `vec3_local` (52): the (y,z) pair of a
   v4 straddles two converted words (`FMA.v2f16 r2, r0.h10, ...` then
   `MKVEC r2, r2^.h0, r3.h0`): a new `Leaf::Straddle { k, c, e, d }` reading
   one half of each of two converts; the mixprec family's `alu16`/`vec16`
   (`vec16.rs`) must not change (`tests/mali_deqp_mixprec.rs`). `rnd_vec4`
   (66) is the residue of these forms plus `conversion_order`'s swapped
   `V2F32_TO_V2F16 r0, r1^, r0^`: **pin the residue after the other two are in**
   (do not promise all 66; the references are one of many arrangements).
   - Un-ignores: `tex_coord_ramp`, `vec3_local`, `rnd_vec4` (the last only
     if its word arrangement lands; else `DIFFERS`/`REFUSED` with the
     new text).
3. **C, 233 cases** (`bcast_compare_*`, `bcast_const`): `lane_tree` gets the
   `B2f` arm (9525) of a compare, and `select` (the `a == b` splat branch at
   9846) the three-word broadcast `IADD.v2u16 r1, r0.h00, 0x0`, `MKVEC.v2i16
   r2, r0^.h0, <alpha>.h0`, two `MOV.i32`. The mask lowering of the
   compare (`ICMP_OR.u32.eq.m1`, `FCMP_OR.v2f16.lt.m1`/`.f1`, `CSEL.u32`,
   `MUX.v2i16 0x0, 0x3C000000.h11, mask.h00`, `FMAX.v2f16 .abs`
   reductions, `LSHIFT_AND.i32 .., 0x0.b00, ..` for the product of masks) is
   `intval-rest` and `arith-lower-1`'s: schedule C **after both**, or limit
   C to the references both items have produced (`bcast_compare_int` and
   `_bool` first: a single `ICMP_OR`/`MOV.i32` and the `MUX`). The reference
   `bcast_compare_float` differs in order (`MKVEC` before `IADD.v2u16`, no
   `MOV` pair); `bcast_compare_array` has `FCMP_OR.v2f16.lt.f1`. 94 of the
   229 read a `structType` uniform: no reference and not drawable
   (`gl_ref.py` cannot set it): a separate change outside this item.
   - Un-ignores: `bcast_compare_int`, `bcast_compare_bool`,
     `bcast_compare_float`, `bcast_compare_array`, `bcast_const`.
4. **E, 8 cases** (`op_pair_other`): after `intval-rest` (the bvec2 round
   trips, `F32_TO_S32.rtz`/`S32_TO_F32`); reuses step 1's narrowing.

### 5.3 Order and dependencies

- Step 1 first: it needs nothing from the plan's other lowerings (the
  compiler already has `degrees`, `+`, `*`, narrowing).
- Step 2's B and D after step 1 (same functions, `Leaf::Lane`/`Pack`); C
  after `intval-rest` and `arith-lower-1`; E last.
- The plan's board rerun (`board-rerun-1`) comes after step 1 and the intval
  items: a re-rank of the table follows, and C or D may be re-ordered by
  their measured counts (`tools/mali/deqp-host-measure.sh --filter`).

## 6. Risks (both steps)

1. **A too-wide narrowing changes a pinned shader.** `wide_float` must be
   `false` for every fp32 lane of the existing references' fragment shaders
   whose lanes Mesa keeps wide (`mali_fs_arith` highp references,
   `mali_deqp_colourpair`'s highp colour). Test: compile each pinned fragment
   with and without the predicate and diff the words before running the
   byte-compare tests (a 10-line harness over `tools/mali/testdata/*.frag`).
2. **Pairs vs lone lanes in the pair graph.** `has_lane_op` and `lone_tree`
   decide "lane alone": an `F2f16` of an fp32 chain must count as a lone
   lane there or `vectorise` recurses again (the 8735 symptom). The
   `Pairs`/`prod`/`ADMITTED` tables (`f.prods & !ADMITTED`, `select`) refuse
   forms nothing drew: a new production may need a reference; the 20
   references are the references.
3. **`ColourZeroHalf` and `ColourPair`** (refusal texts of the existing
   `(Const, lane)`/`(lane, Const)` arms, 8780): the new `first` word must be
   limited to the colour's own words (`word(c)`) as `Leaf::Mk first` is.
4. **Order of narrowing in `nir.order`** decides the register allocation and
   so the words: `narrow_stored`'s `insert_before` placement, `Fs::origin`
   and `block` must be copied for the new nodes (the integer branch of
   `narrow_stored` does; the other branch uses `node16` at the end).
5. **ARITH shift.** Lowering the lane sources moves many operator cases to
   `ARITH`/`ARITH_CONST` refusals (`mali-deqp-fclane.md` end of section 2):
   a smaller pass count than the 406 is expected before `arith-lower-1`.
   Forecast both numbers in the commit.
6. **Pinned generated files.** `tests/reference/*.rs` and
   `tools/mali/testdata/deqp_fclane_*` are inputs; `mali_deqp_fclane.rs`'s
   `LOWERED`/`REFUSED`/`DIFFERS` lists are the only test edit (regenerate in the
   same commit as the change that moves them: CLAUDE.md).
7. **Line numbers drift.** The sites quoted are those of `3497799a`; the
   plan's other items (`constop`, intval) edit `nir_fs.rs` and `select.rs`.
   Re-run the section 3 instrument first.

## 7. Files

- This note: `docs/notes/mali-fclane-lowering.md`.
- Shapes, references, counts: [`mali-deqp-fclane.md`](mali-deqp-fclane.md),
  `tools/mali/testdata/deqp_fclane_*`, `tests/mali_deqp_fclane.rs`.
- Related: `mali-fragment-outputs.md`, `mali-varying-constants.md`,
  `mali-partial-varyings.md`, `mali-deqp-varylink.md`, `mali-deqp-intval.md`,
  `mali-deqp-families.md` section 18.
- Run: `cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_fclane`.

## 8. Measured sites at a7260420

Section 3's sites were quoted at `3497799a` and its section 7 warns they
drift. Measured again at the tip `a7260420` (plan #387, item fclane-sites):
every `deqp_fclane_*` and `deqp_fclane2_*` reference was compiled through
`mali_ref::compile_pair` in a scratch test (features `host-test,gpu-mali,
shaderc-online,no-shader-table`), with every `Err(MaliError::ColourForm)` in
`nir_fs.rs` temporarily routed through a helper that printed `line!()`. The
instrument and the scratch test were removed; nothing under `src/` or
`tests/` changed. No `ColourForm` site other than the three of section 3
fired. All 26 references are refused with the one text `Valhall: a
gl_FragColor lane other than the varying's own lane or a constant is not
lowered yet`.

`nir_fs.rs` has no commit since `3497799a` in `git log 3497799a..HEAD`, which
is why the line numbers still hold: **8735** (`vectorise`, the `else` of two
load lanes that are no pair), **8799** (`vectorise`, the final `_` arm),
**9525** (`lane_tree`'s last arm, no `B2f`). The mechanism column is the
section 4 letter of the note (for `fclane2_*`, the letter of the shape
`bcast_compare` they extend, by site: not read word by word).

| Reference | Site | Mechanism | Differs from the note? |
|---|---:|:-:|---|
| `fclane_bcast_compare_array` | 8799 | C | no |
| `fclane_bcast_compare_bool` | 9525 | C | no |
| `fclane_bcast_compare_float` | 9525 | C | no |
| `fclane_bcast_compare_int` | 9525 | C | no |
| `fclane_bcast_const` | 9525 | C | no |
| `fclane_mat4_swz_sum` | 8799 | A | no |
| `fclane_mat_cols_sum` | 8799 | A | no |
| `fclane_mat_elem_pairs` | 8735 | A | no |
| `fclane_mat_vec_lanes` | 8735 | A | no |
| `fclane_op_pair_other` | 8799 | E | no |
| `fclane_op_pair_x` | 8799 | A | no |
| `fclane_op_pair_xyz` | 8735 | A | no |
| `fclane_op_pair_xyzw` | 8735 | A | no |
| `fclane_op_pair_yz` | 8799 | A | no |
| `fclane_rnd_vec4` | 8799 | D | no |
| `fclane_struct_fields` | 8799 | B | no |
| `fclane_tex_coord_ramp` | 8799 | D | no |
| `fclane_tex_zw_lane` | 8799 | B | no |
| `fclane_varying_merged` | **8799** | B | **yes: the note says 8735** |
| `fclane_vec3_local` | 8735 | D | no |
| `fclane2_struct_array_float` | 8799 | C | new row (like `bcast_compare_array`) |
| `fclane2_struct_arrmember_float` | 8799 | C | new row (like `bcast_compare_array`) |
| `fclane2_struct_basic_float` | 8799 | C | new row (like `bcast_compare_array`) |
| `fclane2_struct_nested_float` | 8799 | C | new row (like `bcast_compare_array`) |
| `fclane2_struct_basic_bool` | 9525 | C | new row (like `bcast_compare_bool`) |
| `fclane2_struct_basic_int` | 9525 | C | new row (like `bcast_compare_bool`) |

Per site: 8735 five references (`mat_elem_pairs`, `mat_vec_lanes`,
`op_pair_xyz`, `op_pair_xyzw`, `vec3_local`); 8799 fifteen (eleven
`fclane_*`: `bcast_compare_array`, `mat4_swz_sum`, `mat_cols_sum`,
`op_pair_other`, `op_pair_x`, `op_pair_yz`, `rnd_vec4`, `struct_fields`,
`tex_coord_ramp`, `tex_zw_lane`, `varying_merged`; and four `fclane2_*`);
9525 six (`bcast_compare_bool`, `_float`, `_int`, `bcast_const`, two
`fclane2_*`). 5 + 15 + 6 = 26.

### 8.1 Corrected step

- **`varying_merged`** (section 5.2, step 2, mechanism B, first bullet): the
  refusal is at 8799, not 8735. The colour `v_color` is a plain `gl_FragColor
  = v_color` of a vec4 whose w lane the link merged with x, so the pair
  `(z, w)` is not two lanes of one load: it reaches `vectorise`'s final arm.
  The step the note gives (`Leaf::Pack { x: Splat, y: Half-low }`, the
  `index()` map for the merged lane) is unchanged; only the arm that gets the
  new case is the final `_` arm, not the 8735 `else`. The operands of the
  pair at 8799 were not printed (only the line), so which of the two
  halves is the merged lane is still read from the note, not measured.
- Section 3's table row at 8735 lists `varying_merged ((0,2),(0,3))`: drop it
  from that row and add it to the 8799 row.
- The six `fclane2_*` references are not in section 3 or 4. By site they
  belong to mechanism C: 9525 for `struct_basic_bool` and `struct_basic_int`
  (the `B2f` of a struct-member compare), 8799 for the four float ones (the
  `Bcsel`, 1.0 pair of `bcast_compare_array`). The per-mechanism counts of
  section 4 (A 406, B 36, C 233, D 138, E 8, total 821) are unchanged: the
  94 must-pass cases of `fclane2_*` (`tests/mali_deqp_fclane2.rs`) are a
  separate count (`mali-deqp-fclane.md` marks them HOST-ONLY and the
  per-reference split is not measured here), so mechanism C would be 327
  with them if they are not already inside the 229 of `bcast_compare_*`.

<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the `!=` comparisons (`CompareForm`)

**Status: design** (plan #336, issue #33; row 20 of `mali-corpus-families.md`).
This note changes no code. It says which comparison `tools/shaders/es100/sharedint.frag`
is refused on, where, what Mesa 26.2.1 emits for `!=` of floats and of ints at
mediump and highp (measured, quoted), what the tip already does with it, the
lowering and the tests. Anything not measured is marked **inferred**. State
read: `f5bc25f4`.

Read first: `mali-vs-compare-clamp.md` (the vertex half of the compares, which
has `Feq`, `Flt`, `Fle` and no `!=`), `mali-alu-ops.md` (the `bool(float)` and
`f2b` rows, which also wait on `Fne`), `mali-int-uniforms.md` (names this
refusal for `sharedint`).

## How the words were made

Mesa 26.2.1 drm-shim, built and driven as in `mali-tex-coords.md` ("How the
words were made": `$CONNECTED_DEPS_DIR/mesa-shim`, `tools/mali/drm_shim_ref.sh
run`, `pan_ref_diff.py extract --draw`). `sharedint` is drawn with its own
vertex shader and without `--uniform-values` (a struct uniform); the small
shaders with `tc.vert` (`attribute vec4 a_pos; attribute vec4 a_v; varying
vec4 v; void main() { gl_Position = a_pos; v = a_v; }`) and
`--uniform-values`. Each small fragment shader is `precision <p> float;
precision <p> int; uniform <p> int m; varying vec4 v; void main() { <body> }`:

| name (`_mp`, `_hp`) | body |
|---|---|
| `fne` | `gl_FragColor = vec4(float(v.x != v.y), 0.0, 0.0, 1.0);` |
| `feq` | `gl_FragColor = vec4(float(v.x == v.y), 0.0, 0.0, 1.0);` (the lowered reference) |
| `ine` | `gl_FragColor = vec4(float(m != 3), 0.0, 0.0, 1.0);` |
| `ine0` | `gl_FragColor = vec4(float(m != 0), 0.0, 0.0, 1.0);` |
| `ineif` | `vec4 c = v; if (m != 0) c.rgb *= 0.5; gl_FragColor = c;` |
| `ieq` | `gl_FragColor = vec4(float(m == 3), 0.0, 0.0, 1.0);` (the lowered reference) |

All 23 draws of this note and `mali-tex-coords.md` printed `glGetError 0x0`.
Every `fs.fragment` has `work_reg_count` 32 and preload `0x3000000000000000`.
"Ours" below was measured with a scratch test (not committed), the shader
compiled alone by `compile(SHC_STAGE_FRAGMENT, ..)` and compared by
`tests.rs::check_fs_stage` with the draw's `expect.json`; the scratch test and
the two-line experiment of step 1 were reverted.

| draw | words | sha256 of the words (16 hex digits and a newline each) |
|---|---|---|
| `sharedint` | 31 | `79519b3ca0b12779fa6669b884b7e4002be03b338cae2b8bd403fd2cd99e77f9` |
| `fne_hp` | 9 | `79b6d6b83a9ad579a4c8046de061d54b0dcbc3be5dbd076947be351e249011cb` |
| `fne_mp` | 9 | `126b94226d1245e59bc5d5882da104c7c861caf4bb0298b9c740caa0c8c86987` |
| `feq_hp` | 9 | `ebcabf7f2d697c719245c2bfec261a966e5c263bb82438b5167b878f4bc99de2` |
| `ine_hp`, `ine_mp` | 9 | `0a5c140001c03757fbf240bf3e1cc90bc74a250cb275cc65ea349d612d225781` |
| `ine0_hp` | 8 | `fdbe94b26010fcfd42565f308c8bf9254f0b7706a943b2ddf2a1b759a9faba3d` |
| `ineif_hp` | 19 | `4e8295008928df1168374acf007b2b6d0d1ce14024c0771b85fa475278292ce4` |
| `ineif_mp` | 13 | `1b5048971b1d444141345d17fd9a13e1dd38a805ff69a3fac00d4f4329cc8330` |
| `ieq_hp`, `ieq_mp` | 9 | `1def13aed0ae3f58d053049d3168dd5f922cd1e8849e32de57f7ff29c15e0732` |

(`ine_mp` is `ine_hp` and `ieq_mp` is `ieq_hp`, word for word: an int is 32-bit
at either precision.)

## The source and the refusal on the tip

`sharedint.frag` (highp float and int), struct uniform `Params` (`mat4`,
`mat4`, `vec4 color`, `float scale`, `int mode`) and `varying vec2 v_xy`:

    vec4 c = u_p.color;
    if (u_p.mode != 0)
        c.rgb *= 0.5 + 0.5 * sin(v_xy.x + v_xy.y);
    gl_FragColor = c;

The comparison it meets is **`Ine`: `ine(u_p.mode, 0)`**, an int struct member
uniform against the constant 0. It reaches the highp walk's `alu32`, whose
last arms (`nir_fs.rs:2139-2141`) are

    /* `!=`: no reference that compiles draws it ... */
    AluOp::Fne | AluOp::Ine => Err(MaliError::CompareForm),
    _ => Err(MaliError::Arithmetic),

(`CompareForm`, `mod.rs:286`, message at `mod.rs:512`). It is the first
refusal: the `ine` comes before the `sin` in program order, so the verdict
is `CompareForm`, not `Arithmetic`. The corpus verdict of `sharedint.vert` is
a different refusal (`UniformType`, the int member in the struct; row 8).

Where else `!=` stops on the tip (measured, `compile` of each small shader):

| shader | verdict on the tip | site |
|---|---|---|
| `fne_hp`, `ine_hp`, `ine0_hp`, `ineif_hp` | `CompareForm` | `nir_fs.rs:2141` |
| `fne_mp` | `Arithmetic` | the mediump compare arm lists `Flt`/`Fle`/`Feq` only (`nir_fs.rs:4086`; the narrow-operand arm `:1778` likewise) |
| `ine_mp`, `ieq_mp` (**sic**) | `IntValue` | `int_lane`'s last arm (`nir_fs.rs:1500-1504`): a mediump shader's int value, `==` too |
| `ineif_mp` | `control flow (if, a loop, a return before the end)` | the mediump `if` |
| `feq_mp` | `IntValue` | `float(bool)` of a mediump compare |
| `feq_hp`, `ieq_hp` | compile, **byte-equal** | (`Fcmp(Eq)`, `Icmp(Eq)`) |

The unit test that pins the refusal is `tests.rs:1195`
(`precision highp float; varying vec4 v; ... float(v.x != v.y)`, expects
`MaliError::CompareForm`); `tests.rs:1053` puts the variant in the `None` arm of
`unreachable_from_es100` ("a gap a test below reaches"), which is that test. `mali-alu-ops.md:187` and `:300` name the same site for
`bool(float)` (`es100/f2b`, `fsarith_f2b`): `Fne` unblocks their compare half.

## What Mesa emits (measured)

### The ops

`bi_translate_cmpf` (`bifrost_compile.c:2582-2605`) maps `ine_pan` and `fneu_pan`
to `BI_CMPF_NE`; `bi_emit_alu` (`:3252-3272`) builds `bi_icmp_to(.., nir_type_int,
.., BI_RESULT_TYPE_M1)` for `ieq_pan/ine_pan/ilt_pan/ige_pan` and `bi_fcmp_to(..,
BI_RESULT_TYPE_M1)` for `feq_pan/fneu_pan/flt_pan/fge_pan`. So a `!=` is the
`==` instruction with the compare function changed; the shim words show it:

| form | instruction | result type | operand shape |
|---|---|---|---|
| `float != float` (`fneu`) | `FCMP_OR.f32.ne.m1` | `m1` (0 / ~0 mask) | two registers, third source `0x0` |
| `float == float` (`feq`), for reference | `FCMP_OR.f32.eq.m1` | `m1` | the same, `eq` |
| `int != 0` (`ine`) | `ICMP_OR.u32.ne.m1` | `m1` | `u2.w0, 0x0, 0x0` (the zero is both the constant and the `_OR` source) |
| `int != 3` (`ine`) | `ICMP_OR.u32.ne.m1` | `m1` | `u2.w0, 0x3020100.b3, r0` with `r0` zeroed first |
| `int == 3` (`ieq`), reference | `ICMP_OR.u32.eq.m1` | `m1` | the same, `eq` |

The disassembly says `u32` for `ne` and `eq` alike (an equality is sign
independent, `bi_icmp_to` is `ICMP.i32` before `va_lower_isel`, and the `_OR`
form is what is encoded). Mesa never uses `.f1`/`.s32` for these, because the
consumer is a `MUX` (the `b2f`), not a `fsat`-style fold: the result type
stays `m1`. A `bool` consumer such as `if` is the `CSEL` below.

### The small shaders

`fne_hp` (9 words; the load is `.v2`, only `x` and `y` are read):

```
LD_VAR_BUF_IMM.f32.slot0.v2.src_f32.center.store.wait0 @r0:r1, r61^, index:0x0
FCMP_OR.f32.ne.m1.discard r0, r0^, r1^, 0x0
MUX.v2i16.wait0126 r0, 0x0, 0x3C000000.h11, r0^.h00        ; float(bool) -> fp16 1.0 / 0
ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
U16_TO_U32 r0, r0^.h0
MOV.i32.wait r1, 0x3C000000
BLEND.slot1.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
IADD_IMM.i32 r48, 0x0, #0x0
BRANCHZI.eq.absolute 0x0, blend_descriptor_0.w1
```

`fne_mp` and `feq_mp` are the same words with a `.v4` load
(`@r0:r1:r2:r3`); `feq_hp` has `.eq` for `.ne`. **The mediump compare of two
varyings is a 32-bit `FCMP_OR.f32`** on the raw lanes, not a half compare.

`ine_hp` (and `ine_mp`), `m != 3`:

```
MOV.i32.discard r0, 0x0
ICMP_OR.u32.ne.m1 r0, u2.w0, 0x3020100.b3, r0^
MUX.v2i16.wait0126 r0, 0x0, 0x3C000000.h11, r0^.h00
ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
U16_TO_U32 r0, r0^.h0
MOV.i32.wait r1, 0x3C000000
BLEND.slot0.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
...
```

`ine0_hp`, `m != 0`: `ICMP_OR.u32.ne.m1.discard r0, u2.w0, 0x0, 0x0`, then
the same `MUX`/`ATEST`/`U16_TO_U32`/`MOV`/`BLEND` (8 words, no leading `MOV`).
`ieq_hp` is `ine_hp` with `.eq`.

`ineif_hp`, `if (m != 0) c.rgb *= 0.5` (19 words): the `if` is **flattened**
(no branch), one compare and one `CSEL` per lane:

```
LD_VAR_BUF_IMM.f32.slot0.v4.src_f32.center.store.wait0 @r0:r1:r2:r3, r61^, index:0x0
FMA.f32.discard r4, r1, 0x38000000.h1, 0x0.neg             ; the then side, speculated
ICMP_OR.u32.ne.m1 r5, u2.w0, 0x0, 0x0
CSEL.u32.eq r1, r5, 0x0, r1^, r4^                          ; mask == 0 ? original : scaled
FMA.f32 r4, r0, 0x38000000.h1, 0x0.neg
CSEL.u32.eq r0, r5, 0x0, r0^, r4^
FMA.f32 r4, r2, 0x38000000.h1, 0x0.neg
CSEL.u32.eq r2, r5^, 0x0, r2^, r4^
...                                                        ; narrowing and blend
```

`ineif_mp` is the same shape on half pairs (`V2F32_TO_V2F16`, `FMA.v2f16`,
`MUX.v2i16` with the mask `r3.h00`, 13 words); the compare is still the 32-bit
`ICMP_OR.u32.ne.m1 r3, u2.w0, 0x0, 0x0`.

### `sharedint.frag`

31 words (`sharedint.expect.json`, `fs.fragment`), without the three-word
blend tail:

```
LD_VAR_BUF_IMM.f32.slot0.src_f32.center.store.discard @r0, r61^, index:0x0   ; v_xy.x ... (a vec2 read as one lane pair)
MOV.i32.wait0 r1, u5.w0
FMA.f32 r1, r0, u4.w1, r1^
FADD_IMM.f32 r2, r1, #0xC9400000
FMA.f32 r0, r2^, u5.w1, r0^
FSIN_TABLE.u6 r2, r1
FCOS_TABLE.u6 r1, r1^
FMA_RSCALE.f32 r3, r0, r0, 0x0.neg, 0xFFFFFFFF
FMA.f32 r3, r3^.neg, r2, 0x0.neg
FMA.f32.clamp_m1_1 r0, r0^, r1^, r3^
FADD.f32 r0, r0^, r2^                                     ; sin(x + y), the transcendental chain
FMA.f32 r0, 0x38000000.h1, r0^, 0x38000000.h1             ; 0.5 + 0.5 * sin
FMA.f32 r1, u2.w0, r0, 0x0.neg                            ; color.xyz * that
FMA.f32 r2, u2.w1, r0, 0x0.neg
FMA.f32 r0, u3.w0, r0^, 0x0.neg
ICMP_OR.u32.ne.m1 r3, u4.w0, 0x0, 0x0                     ; mode != 0
CSEL.u32.eq r1, r3, 0x0, u2.w0, r1^                       ; mask == 0 ? color.x : scaled
CSEL.u32.eq r2, r3, 0x0, u2.w1, r2^
CSEL.u32.eq r0, r3^, 0x0, u3.w0, r0^
FADD.f32 r1.h0, r1^, 0x0.neg                              ; narrow to the fp16 output
FADD.f32 r2.h0, r2^, 0x0.neg
FADD.f32 r0.h0, r0^, 0x0.neg
FADD.f32.wait0126 r3.h0, u3.w1, 0x0.neg                   ; color.w
ATEST.wait0 @r60, r60^, r3.h0, atest_datum.w0
MKVEC.v2i16 r1, r1^.h0, r2^.h0
MKVEC.v2i16 r2, r0^.h0, r3^.h0
MOV.i32 r0, r1^
MOV.i32.wait r1, r2^
```

FAU (12 words): four blend constants, `u2` = `color.x, color.y`, `u3` =
`color.z, color.w`, `u4.w0` = `mode`, then `3f22f98c`, `49400000`, `bfc90fd0`
(the three constants of the `sin` sequence). Observations:

- **The `if` is flattened and the compare sits next to its selects**, after
  the `sin` work, which is evaluated unconditionally. **Inferred:** the
  peephole took the `sin` chain because `nir_opt_peephole_select` runs with
  `expensive_alu_ok` (the pass `Nir::peephole_select` transcribes, limit 64,
  `mali-control-flow.md`); that the whole `then` side is within the
  limit is shown by the listing, not computed here.
- The else side of each lane is the **uniform itself** (`CSEL ... u2.w0, r1^`),
  the `then` value the scaled one; the selects come in the order y, z... of the
  listing (`r1`, `r2`, `r0`), the registers' allocation, not the lane order.
- Pushed uniforms: only `color` and `mode` of the struct are pushed (the two
  matrices and `scale` are not read in this stage).
- The compare is `ICMP_OR.u32.ne.m1 r3, u4.w0, 0x0, 0x0` as in `ine0_hp`
  (a uniform word against the constant zero, no leading `MOV`).

## What the tip does with the `!=` forms (measured)

Experiment (reverted): `nir_fs.rs:2141` replaced by

    AluOp::Fne => self.op32(Alu::Fcmp(Cmpf::Ne), [x, y, u], at),
    AluOp::Ine => self.op32(Alu::Icmp(Cmpf::Ne), [x, y, u], at),

| shader | after the edit |
|---|---|
| `fne_hp` | **byte-equal** (words, registers, preload, FAU) |
| `ine_hp`, `ine0_hp`, `ineif_hp` | **words byte-equal**; the scratch comparison stops at the FAU check only because the existing test helper `uniform_value` models every uniform as a float (reference `0x10`, helper `0x41800000`); the same happens to the already-lowered `ieq_hp` |
| `sharedint.frag` | `Arithmetic`: the `sin` (`AluOp::Fsin`), which is the next refusal |
| `fne_mp` | `Arithmetic`; `ine_mp` `IntValue`; `ineif_mp` control flow (unchanged: mediump arms) |

So the back end already encodes everything: `emit.rs:259` (`Alu::Icmp(Cmpf::Eq
| Cmpf::Ne)` at 32 bits is `IcmpOrU32`), `emit.rs:252` (`Fcmp` at 32 bits is
`FcmpOrF32` for any `cmpf`), and `pack.rs:225` fill `cmpf` generically. The
flattening of the `if` into `CSEL` is also already right (`ineif_hp`).

## The lowering

1. **`nir_fs.rs:2141`, the two arms above** (highp, `alu32`), no operand
   rewriting: `fneu_pan` and `ine_pan` take their GLSL operands in order, as
   `Feq` and `Ieq` do. Check that a `Fne` whose operand is a constant keeps the
   constant in the second source as `Ieq` does (the fp32 path has no
   16-bit constants). Measured: `fne_hp`, `ine_hp`, `ine0_hp`, `ineif_hp`.
2. **Related folds on the `Ine` result (inferred, unmeasured):** NIR folds
   `ine(b, 0)` of a bool, `ine(ineg(..))` and `inot(ine(a, b))` into `ieq`
   (`nir_opt_algebraic.py`). The mask folds of `nir_fs.rs:1472-1483`
   (`Ieq|Ine` of a mask lane against `0`/`~0`) already exist for the mediump int
   path; `alu32` has none, and none is needed for the corpus (the compared
   value is a uniform). Add a reference before adding a fold.
3. **`emit.rs` table (documentation rows at `:62-64`, `:82`):** add
   `Fcmp(Cmpf::Ne), 32, 1` ("nir_op_fneu_pan: FCMP.f32.ne.m1") and
   `Icmp(Cmpf::Ne), 32, 1` ("nir_op_ine_pan: ICMP_OR.u32.ne.m1") if the table
   is checked against the lowering (it lists what a reference measured;
   `emit.rs:165` already lists the 16-bit `Icmp(Ne)`).
4. **Mediump (not needed by the corpus; a separate item):** Mesa compares two
   mediump varyings at **32 bits** (`fne_mp`, `feq_mp` above), and a
   mediump int at 32 bits (`ine_mp`), while the tip's mediump arms narrow
   compare operands to fp16 (`:4086`, `:1778`) and refuse an int value
   (`:1504`) or an `if`. The `Fne` mediump arm therefore needs the operand
   rule of the existing `Feq` one plus the same 32-bit/16-bit choice the
   references of `Feq` make (**inferred:** a compare whose operands are
   unconverted varying lanes is fp32, a compare against a constant narrowed by
   `lower_precision` is fp16, as `:1778`'s comment says for `Flt`/`Fle`/`Feq`).
   `ieq_mp`'s `IntValue` is the same family (`mali-int-and-size.md`), not
   this one.
5. **`sharedint.frag` as a whole** needs, after step 1, `sin` (`FSIN_TABLE`,
   `FCOS_TABLE`, `FMA_RSCALE`, `mali-transcendentals.md`; plan item
   `fs-transc`, case 77) and then the struct-uniform layout of `color` and
   `mode` (the words above push `color` at `u2:u3` and `mode` at `u4.w0`; the
   existing struct-uniform handling reads `Params` as a vec4 and an int,
   **inferred** to give the same placement, not run past the `sin`).
   `sharedint.vert` is row 8 (`UniformType`) plus a vertex `if` and stays
   refused (`nir.rs` refuses vertex control flow): the compare there is
   `ICMP_OR.u32.ne.m1 r7, u17.w1, 0x0, 0x0` with four `CSEL.u32.eq` (measured in
   `sharedint.expect.json`, `vs.position`), the same instruction.

## Files, test, what it moves

- Code: `src/shaderc/online/mali/nir_fs.rs:2141` (step 1);
  `src/shaderc/online/mali/emit.rs` (rows, step 3). Nothing in `bir.rs`,
  `pack.rs`, `ra.rs`.
- Testdata to add **by the item that lands the change** (not this note):
  `tools/mali/testdata/cmp_fne.{vert,frag,expect.json}`, `cmp_ine`,
  `cmp_ine0`, `cmp_ineif` (the highp shaders above with `tc.vert`), drawn
  **without `--uniform-values`** so the int uniform stays 0 and the FAU check
  needs no integer model (or teach `uniform_value` the declared type);
  `sharedint.{vert,frag,expect.json}` when `sin` has landed.
- Host test, `src/shaderc/online/mali/tests.rs`: byte-equal checks of the four
  (`check_fs_stage`, like the `ieq` one in `fvg_texture_copy_unclipped`'s
  neighbours); **move** `tests.rs:1195` (`float(v.x != v.y)` highp, now
  `CompareForm`) from the refusal table to a byte test or give it the next
  refusal (it will compile: `fne_hp` is equal); keep a refusal pin for the
  mediump `Fne` (`fne_mp`, `Arithmetic`) while step 4 is open.
  `MaliError::CompareForm` has no other raiser, so after step 1 the gap is
  unreachable: remove it from `GAPS` (`mod.rs:361`) and the match arms
  (`mod.rs:286`, `:512`, `tests.rs:1053`), or leave it as the mediump `Fne`'s
  refusal if step 4 reuses it.
- `tools/mali/corpus-verdicts.sh --write`: `sharedint.frag` moves from
  `CompareForm` to `Arithmetic` (its `sin`), then to `ok` with `fs-transc`;
  `es100/f2b.frag` is `Arithmetic` before its compare (the `Ffloor`, mediump
  arm) and is unchanged by this step.
- Cases moved: corpus `sharedint.frag` (with `sin`). No gl_test case and no
  dEQP-GLES2 or glmark2 shader is attributed to `CompareForm` by the corpus
  tally (one source). Any dEQP-GLES2 shader with `!=` on a highp float or an
  int uniform reaches the same site (**inferred**: dEQP's
  `operator.not_equal.*` and `bool(float)` shaders; not counted here).

## Not covered / open

- Mediump `!=`, mediump int compares (step 4, `IntValue`).
- Vector `!=`: `notEqual(vec2, vec2)`/`any`/`all` forms
  (`mali-vs-compare-clamp.md` has the vertex `Ball`/`Bany`); this note
  measured scalar compares only.
- The vertex `!=` (`sharedint.vert`): needs the vertex `if` and `UniformType`
  first.

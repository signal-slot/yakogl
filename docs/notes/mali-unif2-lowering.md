<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali dEQP-GLES2: lowering the uniform array, struct and type refusals to Mesa's words (unif2-design)

Plan #387, item unif2-design: a design, no code. It takes the 15 shapes of
`docs/notes/mali-deqp-unif2.md` (`tools/mali/testdata/deqp_unif2_*`,
`tests/mali_deqp_unif2.rs`) and says how Mesa 26.2.1 lays out and loads each
uniform, where the compiler refuses it, which mechanism lifts the refusal, and
in which one-session steps. The vertex int and mediump uniform shapes are the
design of intval step 3 (item vs16-design,
`docs/notes/mali-intval-vs16-lowering.md`, not merged when this was written);
here they are only named, as *covered by intval step 3*.

**Counts.** Every case count is the one `mali-deqp-unif2.md` has at its commit
`314fdc9c` (integration tip `0819a074`): the `unitype` and `uniarray` rows of
`deqp_shapes/index.tsv`, texts of `37305608`, the run at `abcc9013`, first
refusal of each case, 944 cases in the 15 shapes (1,092 with the seven shapes
that already have a `deqp_unitype_*` or `deqp_uniarray_*` reference). They were
not re-measured here; unif2-recount may have moved them. A step's forecast is
the sum of its shapes' counts, an upper bound until `tools/mali/deqp-host-measure.sh
--filter uniform` (note section 1) is run on a VK-GL-CTS `93bca01` checkout.

**What was read, what was not.** Mesa's side is the pandecode text stored in
each reference's `.expect.json` (`regions.*.text`, the instructions of the
draw, and `regions.*.fau`, the push words), read with `python3 -I`; the
sequences below are copied from it. No Mesa was built and no board used.
The compiler's side is the source at `0819a074`. Where a statement is a
prediction (the refusal that follows the one now raised) it says so, and the
first action of the step that depends on it is to run the pair and read it.

## 1. How Mesa lays out and loads each uniform

### 1.1 The rules (from `mali-deqp-unitype.md` section 3, `mali-deqp-uniarray.md`
section 3, and checked against the references)

- **Packed block, FAU push.** Mesa packs the default uniform block, one dword
  per component, nothing aligned (`glsl_count_dword_slots`): a vec3 is 3, a
  mat3 9, an array element times length, a struct the sum of its members. Every
  load of a constant offset is *pushed*: the words the program reads become FAU
  words in block order, after the viewport words (and, in a fragment shader,
  the blend constants), each constant the program needs that no instruction
  encodes as an immediate after them. An unread word, element or member gets no
  FAU word (`arr_mat_array`: element 2 of five is six words; `arr_struct_array`:
  12 of 34 member words). So **no reference of these 15 shapes uses a UBO row
  loaded from memory**: that needs a run-time index (`mali-uniform-index.md`,
  `mali-ubo-rows.md`), which none of them has. Their lowering is the *layout*
  (`select::UboLayout`, which words, in which order) plus the *operations* that
  read the words.
- **Precision.** `lower_precision` retypes to fp16 a uniform that is a plain
  (non-struct) float, vec, mat or an array of one, declared mediump or lowp: the
  block holds it as halves, two a word, a column or an array element starting a
  word (`vs_mat_uniform`, `vs_mediump_array`, `arr_mat_array`). A **struct
  member stays fp32 whatever its precision**; a mediump read narrows it
  (`FADD.f32 rX.h0, uN, 0x0.neg`, two reads `V2F32_TO_V2F16 rX, uA, uB`). An
  **int is never narrowed** (`caps->int16` false): one raw 32-bit word a
  component, at every precision (`fs_int_uniform`: a `mediump int u_var[3]` is
  three plain words).
- **Bool.** One 32-bit word a component, true is `~0` in Mesa
  (`UniformBooleanTrue`), read as a mask: `LSHIFT_AND.i32` of two bools is the
  AND, `x == false` is `LSHIFT_OR.i32 r, 0x0, 0x0.b00, uN.not` (the `.not` source
  modifier), the final select `CSEL.u32.eq` (vertex) or `MUX.v2i16` (fragment).
  The GL core stores a bool as **1** (`src/gl/program.rs:2787`); see section 5.
- **Compare of ints.** `a == K` is `ICMP_OR.u32.eq.m1 rX, uN, K, 0x0` for the
  first term and `ICMP_AND.u32.eq.m1 rX, uN, K, rY` for the next, `K` an 8-bit
  byte-lane immediate (`0x3020100.b2`, `0x7060504.b0`), a register
  (`MOV.i32`/`IADD_IMM.i32`) or a FAU constant; compares are joined by
  `LSHIFT_AND.i32 rX, rA, 0x0.b00, rB`.
- **`--uniform-values` does not set arrays or struct members.** The FAU words
  of the array and struct references are 0 (`gl_ref.py` looks up `u_var` of an
  array, location -1, and stops on a struct; note unif2 section 4). The
  instruction words are Mesa's; the word-to-member order of a struct is not
  recoverable from them (section 6, step U0).

### 1.2 The shapes

FAU indexes count the 32-bit words of the stage's FAU as in `expect.json`
(`uN.wM` is word `2N+M`); the uniform words start at 11 in the vertex varying
program, 4 in the fragment program (the viewport words, the blend constants).

| Shape | Uniform in Mesa | Mesa's sequence (key instructions) |
|---|---|---|
| `arr_bool_array` VS | `bool u_var[3]`, 3 words at FAU 11-13 (`u5.w1`, `u6.w0`, `u6.w1`), one load each | `LSHIFT_OR.i32 r0, 0x0, 0x0.b00, u5.w1.not` (`[0] == false`); `LSHIFT_AND.i32 r0, u6.w0, 0x0.b00, r0^`; `LSHIFT_AND.i32 r0, u6.w1, 0x0.b00, r0^`; `CSEL.u32.eq r0, r0^, 0x0, 0x0, 0x3F800000`; `FMA.f32 r0, r0^, 0x0, r0^` (the tail of the existing `deqp_intval_uniapi_bool_vs`) |
| `arr_bool_array` FS | 3 words at FAU 4-6 (`u2.w0`, `u2.w1`, `u3.w0`) | `LSHIFT_AND.i32 r0, u2.w1, r0^.b00, u2.w0`; `LSHIFT_OR.i32 r1, 0x0, 0x0.b00, u3.w0.not`; `LSHIFT_AND.i32 r0, r0^, 0x0.b00, r1^`; `MUX.v2i16 r0, 0x0, r1^.h00, r0^.h00` (the 1.0 / 0 select of `v_vtxOut`); the colour `MKVEC.v2i16 r1, r0.h0, u3.w1.h0` (u3.w1 the `1.0` half pair) |
| `arr_int_array` VS | `mediump int u_var[3]`, 3 plain words at FAU 11-13; the constant -2 as FAU word 14 (`u7.w0`, `fffffffe`), 4 and 0 immediates | `MOV.i32 r0, u7.w0`; `ICMP_OR.u32.eq.m1 r0, u5.w1, r0^, 0x0`; `ICMP_AND.u32.eq.m1 r0, u6.w0, 0x7060504.b0, r0^`; `ICMP_AND.u32.eq.m1 r0, u6.w1, 0x0, r0^`; `CSEL.u32.eq` as above |
| `arr_int_array` FS | 3 words at FAU 4-6, -2 at 7 (`u3.w1`) | `ICMP_OR.u32.eq.m1 r0, u2.w1, 0x7060504.b0, r0^`; `MOV.i32 r1, u3.w1`; `ICMP_OR.u32.eq.m1 r1, u2.w0, r1^, 0x0`; `ICMP_AND.u32.eq.m1 r0, u3.w0, 0x0, r0^`; `LSHIFT_AND.i32 r0, r0^, 0x0.b00, r1^`; `MUX.v2i16 r0, 0x0, r1.h00, r0^.h00` |
| `fs_int_uniform` | as `arr_int_array` FS, constants -9 (`u4.w0`, `fffffff7`), 2 (`0x3020100.b2`), -2 (`u3.w1`) | `ICMP_OR.u32.eq.m1 r0, u2.w1, 0x3020100.b2, r0^`; `ICMP_AND.u32.eq.m1 r0, u3.w0, u3.w1, r0^`; `MOV.i32 r1, u4.w0`; `ICMP_OR.u32.eq.m1 r1, u2.w0, r1^, 0x0`; `LSHIFT_AND`; `MUX.v2i16.wait0126 r0, 0x0, 0x3C000000.h11, r0^.h00` (the vertex shader is `v_vtxOut = 1.0`, so the select is of a constant) |
| `fs_bool_uniform` | 3 words at FAU 4-6; `compare_bool(x, true)` is the word itself | `LSHIFT_AND.i32 r0, u2.w1, r0^.b00, u2.w0`; `LSHIFT_AND.i32 r0, r0^, 0x0.b00, u3.w0`; `MUX.v2i16.wait0126 r0, 0x0, 0x3C000000.h11, r0^.h00` |
| `vs_bool_uniform` VS | `bvec2 u_var`, 2 words at FAU 11-12 (drawn `ffffffff`, 0) | `LSHIFT_OR.i32 r0, 0x0, 0x0.b00, u5.w1.not` (x == false); `LSHIFT_AND.i32 r0, u6.w0, 0x0.b00, r0^` (y == true); `CSEL.u32.eq r0, r0^, 0x0, 0x0, 0x3F800000`; `FMA.f32 r0, r0^, 0x0, r0^` |
| `vs_bool_uniform` FS | 2 words at FAU 4-5 | `LSHIFT_OR.i32 r1, 0x0, 0x0.b00, u2.w0.not`; `LSHIFT_AND.i32 r1, u2.w1, 0x0.b00, r1^`; `MUX.v2i16 r0, 0x0, r0^.h00, r1^.h00` after `FADD.f32 r0.h0, r0^, 0x0.neg` (the `v_vtxOut` narrowed) |
| `vs_int_uniform` | three plain `mediump int` words, the loop bound compared in place | covered by intval step 3 (`ICMP_OR.s32.lt.m1 r4, u5.w1, 0x1000000.b3, r3`, `MUX.v2i16` selects of the `tmp[ui_zero]` subscript, `F16_TO_F32` of the result) |
| `vs_struct_uniform` | `struct { mediump int m0; ivec4 m1; int m2[3]; ivec4 m3[3]; }`: 20 words (m0 1, m1 4, m2 3, m3 12) at FAU 11-30, fp32, no padding, member array an element a word; the constants that fit no byte immediate (-9, -2, -10 and the like) as FAU words after them or `MOV.i32`/`IADD_IMM.i32` (`u15.w1`, `u16.w0`, `u16.w1` are read as constants) | per scalar `ICMP_OR.u32.eq.m1 rX, uN, K, r3` (r3 = 0), per ivec4 an `ICMP_OR`/`ICMP_AND` chain of its four lanes, constants `MOV.i32`/`IADD_IMM.i32 #0xFFFFFFF6`; the eight compares joined by `LSHIFT_AND.i32`; `CSEL.u32.eq`; `FMA.f32` |
| `fs_struct_uniform` | `struct { bool m0; bvec4 m1; bool m2[3]; bvec4 m3[3]; }`: 20 words at FAU 4-23 | `LSHIFT_AND.i32`/`LSHIFT_OR.i32 ... .not` per lane (a `false` lane is the `.not` form, a `true` lane the word), `LSHIFT_OR.i32 r2, u3.w1, r0.b00, u3.w0` then `.not` for an all-false pair; joined by `LSHIFT_AND.i32`; `MUX.v2i16.wait0126 r0, 0x0, 0x3C000000.h11, r0^.h00` |
| `arr_struct_array` | `uniform S s[2]` (S{float a; T b[3]; int c}, T{float a; vec2 b[2]}): 12 used members, 12 words at FAU 11-22, fp32; `s[0].c` is an int | each mediump float read `FADD.f32 rX.h0, uN, 0x0.neg`; a pair `MOV.i32 r1, u5.w1` + `V2F32_TO_V2F16 r1, u6.w0, r1^`; the sums `FADD.v2f16`/`FMA.v2f16`; `F16_TO_F32 r2, r2^.h0` before the varying store; the int member `S32_TO_F32 r2, u8.w1`, `FADD.f32 r2, r2^, u10.w1`, `FADD.f32 r2, r2^, u10.w0.neg` (`float(s[0].c) + s[1].b[2].a - s[1].b[1].a`) |
| `arr_mat_array` VS | `mediump mat3 u_var0[5]`: element 2 only, 6 words at FAU 11-16, fp16, a column two words (x,y; z + zero pad); the compare constants are FAU words 17-20 (`c60dc60d`, ...); `ivec4 u_var1` unused, no word | `FADD_IMM.v2f16 r0, u6.w1, #0xC801C5EC` (an xy pair against a literal), `FADD.v2f16 r1, u7.w0.h00, r1^` (a z alone), `FMAX.v2f16 rX, rX.abs.h00, rX.abs.h11` chains, one `FCMP_OR.v2f16.lt.m1 r0, r0^.h00, u10.w0, 0x0`, `S16_TO_S32 r0, r0^.h0`, `CSEL.u32.eq` |
| `fs_mat_uniform` | `highp mat2 u_in1`: 4 fp32 words at FAU 4-7 (columns in order, drawn 16..19) | `LD_VAR_BUF_IMM.f32.v4 @r0:r3` (the `v_mat2` varying whole), `FADD.f32 r0, r0^, u2.w0`, `r1 ... u2.w1`, `r2 ... u3.w0`, `r3 ... u3.w1`, `FADD.f32 r1, r1^, r3^`, then `FADD.f32 rX.h0, rX, 0x0.neg` per lane and `MKVEC.v2i16` |
| `vs_mediump_array` | `mediump vec4 u_var[3]`: 6 fp16 words at FAU 11-16, a vec4 two words | per element two `FADD_IMM.v2f16 rX, uN, #imm`, `FMAX.v2f16 .abs` chains, one `FCMP_OR.v2f16.lt.m1`, `S16_TO_S32`, `CSEL.u32.eq` (VS); FS the same into `MUX.v2i16 r0, 0x0, r1^.h00, r0^.h00` |
| `vs_mediump_struct` | `struct { mediump float m0; mediump vec4 m1; }`: 5 fp32 words at FAU 11-15, fp32 (struct), narrowed at the read | `MOV.i32 r0, u7.w0`; `V2F32_TO_V2F16 r0, u6.w1, r0^`; `V2F32_TO_V2F16 r1, u5.w1, r1^`; `FADD.f32 r2.h0, u7.w1, 0x0.neg`; `MKVEC.v2i16` repacks; `FADD_IMM.v2f16`/`FMAX.v2f16`/`FCMP_OR.v2f16.lt.m1`/`S16_TO_S32`/`CSEL.u32.eq`; FS the same from FAU 4-8 |
| `fs_other` | two `mediump float` uniforms `d`, `e`: **each its own word** (FAU 4, 5; the fp16 in the low half, drawn `4c00`, `5000`), the colour constants as FAU pairs after (6-8) | `MKVEC.v2i16 r1, u2.w0.h0, u2.w1.h0` (the pair `(d, e)` built from two words), `FMA.v2f16 r1, u3.w0, r1^, 0x0.neg`; `FMA.v2f16 r0, u3.w1, r0^.h00, 0x0.neg`; `MKVEC.v2i16 r2, r0^.h0, u4.w1.h0` |
| `vs_mat_uniform` | mat2/mat3 columns as fp32 words | compiles byte-equal now (`LOWERED`) |

Observations that matter for the lowering:

1. In every array and struct shape the *uniform words are consecutive and in
   declaration order at the front of the stage's uniform block*; the compiler's
   layout already produces that for float arrays (`UboLayout::push_uniform`).
2. Nothing here needs a UBO row from memory (`MAX_UBO_ROWS`, `LD_PKA`).
3. The four mediump struct float members and the `s[0].c` int in
   `arr_struct_array` are fp32 words read through f2f16 / i2f; this is the same
   rule `vsstruct_partial` and `vsstruct_rip` already pin.
4. Constant ints and bools: a value that fits a byte immediate (`0x3020100.bK`)
   is not a FAU word; others (-2, -9, -10) are FAU words after the uniform
   block (`fs_int_uniform`: `u3.w1`, `u4.w0`) or `MOV.i32`/`IADD_IMM.i32`.

## 2. Where the compiler refuses, and what it lacks

Sites at `0819a074`. "Err" is the `MaliError` (`mod.rs:417-427` gives the text).

| Site | Err | Lacks |
|---|---|---|
| `nir::uniform_of` (`nir.rs:1046-1089`): `array_len > 1` and `array_element(u)` is None | `UniformArray` | `array_element` (`nir.rs:1103`) takes float, vec2, vec3, vec4 only (stride 1, 2, 4, 4); an int, bool, ivec, bvec, mat array has no element rule. This is the vertex refusal of `arr_bool_array`, `arr_int_array`, `arr_mat_array` and of the member arrays `u_var.m2[3]`, `u_var.m3[3]` of `vs_struct_uniform` once its int compares are admitted |
| `nir::uniform_of`: `gl_type` not in the table | `UniformType` | the table has every int, uint, bool, bvec, float, vec, mat2-4 scalar type; a struct has no row of its own (the front end flattens it to leaf rows named `s.b[1].b`, `reflect.rs` `push_struct_members`), so a refusal here is a *leaf* type, not a struct. `arr_struct_array` and `vs_struct_uniform` reach it through the next site |
| `nir.rs:4455` (`IrOp::LoadUniform`): a `GL_INT` that is neither `index_only` nor `int_compare_only` | `UniformType` | the vertex walk reads an int uniform only as a run-time index or a loop's head-test operand. A compare in a product (`compare_int(u, K)`), a `float(int)` (`arr_struct_array`'s `s[0].c`) and a select by it are not read: **intval step 3** |
| `mod.rs:1352-1366`, the vertex layout check | `UniformPrecision` | a mediump or lowp int uniform that is not a struct member and not an fp16 uniform: `arr_int_array` (after the array rule), `vs_int_uniform`: **intval step 3**. A bool and a struct member are exempt |
| `UboLayout::fs16` (`select.rs:579-668`) | `FsUniform` | `GL_INT`/`GL_BOOL`/`*_VEC*` arms need `array_len <= 1`; an int, bool, ivec or bvec array (`fs_int_uniform`, `fs_bool_uniform`, `arr_*` FS, the `m2`/`m3` members of `fs_struct_uniform`) falls to `_ => FsUniform`. The row model already holds an array (`push_uniform`, split rows) |
| `nir_fs::uniform_extent` (`nir_fs.rs:7436`) = `nir::uniform_of` | `UniformArray` | as the first row, for the highp fragment walk |
| `nir_fs::vectorise`, the `_ =>` arm (`nir_fs.rs:8764-8786`) | `FsUniform` | two uniform lanes become one fp16 pair only when they are halves of one word (`H01`) or one lane twice (`H00`, `H11`); `fs_other`'s `(d, e)` are the low halves of two words, Mesa's `MKVEC.v2i16 r1, u2.w0.h0, u2.w1.h0` |
| `nir::vs_reduce` (`nir.rs:3612`): `a == Ball && op == Ieq && n == 3` only | `Arithmetic` | `vs_bool_uniform`'s `bvec2 == bvec2(false, true)` (n = 2, one constant `false` and one `true`); the vertex chain of three scalar bool compares of `arr_bool_array` (the first `[0] == false` has the `.not` form) |
| the fragment colour: `result = v_vtxOut * select(...)`, `gl_FragColor = vec4(result, result, result, 1.0)` | `FragColorLane` | not a uniform change; `fs_mat_uniform` (a lane `res[0][1] + res[1][1]`), `vs_mediump_array`, `vs_mediump_struct` fail here **once their uniforms load**, and the fragment shaders of the int and bool shapes will meet it after `FsUniform` is lifted (plan item fclane-lower) |

**Driver side.** The Mali draw reads uniforms only through the record's
`FauWord::Uniform { dword }` / `UniformHalves { lo, hi }` (pushed words,
`src/mali/program.rs:108-110`, `frame.rs:3059-3060`) and `UboRow`
(`frame.rs:552`, rows from memory). Both copy the GL core's dwords as they are
(`uniforms[r.dword + k]`). Arrays, struct members, ints and the mat3 array need
nothing new there: the dwords are already the front end's, and the compiler
names which it pushes. **One change is needed: the bool word** (section 5).

## 3. Mechanisms, ranked by must-pass cases

Every one of the 15 shapes is assigned once; the cases are those of
`mali-deqp-unif2.md` (section 2 of that note).

| # | Mechanism | Shapes | Cases | Status |
|---:|---|---|---:|---|
| M1 | Int, bool, ivec, bvec array element rule, top-level arrays, both stages | `arr_bool_array` 95, `arr_int_array` 46, `fs_int_uniform` 52, `fs_bool_uniform` 33 | **226** | to do (steps 3, 4); the vertex int ones also need intval step 3 |
| M2 | A fragment colour lane from a loaded uniform (not a uniform change) | `fs_mat_uniform` 119, `vs_mediump_struct` 53, `vs_mediump_array` 25 | **197** | plan item fclane-lower; this note only lists the uniform words each expects (section 6, step 9) |
| M3 | Struct uniforms: leaf rows, member arrays of int, bool, ivec4, bvec4, the nested `s[k].b[j]` | `vs_struct_uniform` 134, `fs_struct_uniform` 55, `arr_struct_array` 2 | **191** | to do (step 5), after M1 (member arrays) and, for the vertex int members, intval step 3 |
| - | Vertex int and mediump int uniforms | `vs_int_uniform` 117 | 117 | **covered by intval step 3** |
| M4 | Vertex bvec / bool compare chain (the `.not` first lane) | `vs_bool_uniform` 67 | **67** | to do (step 6) |
| M5 | Two mediump uniform words as one fp16 pair | `fs_other` 7 | **7** | to do (step 7) |
| M6 | A mat3 array element, fp16 | `arr_mat_array` 1 | **1** | to do (step 8) |
| - | Already byte-equal | `vs_mat_uniform` 138 | 138 | `LOWERED`; the cases leave the table at the re-measure |

226 + 197 + 191 + 117 + 67 + 7 + 1 + 138 = 944, the 15 shapes. The three
largest mechanisms by cases (M1, M2, M3: 614) are all behind a prerequisite
that is not a uniform change: M1's vertex halves and M3's vertex int members
behind intval step 3, M1's and M3's fragment halves and all of M2 behind the
fragment colour lane (fclane-lower) and intval's fragment compare. The steps
below therefore produce a *reference that compiles through the uniform*, and
un-ignore a test the moment the prerequisite is also in; the table in each step
says which.

## 4. Prerequisites this design assumes (not designed here)

- **intval step 3** (vs16-design): a mediump int and ivec uniform read as a
  value in the vertex walk (`nir.rs:4455`'s `index_only || int_compare_only`
  guard, `mod.rs:1364`'s `UniformPrecision`), and the vertex int compare chain
  of the sequences in section 1.2 (`ICMP_OR/AND.u32.eq.m1`, `LSHIFT_AND.i32`,
  `CSEL.u32.eq`). If step 3 admits only compares, `float(int uniform)`
  (`arr_struct_array`: `S32_TO_F32` of the FAU word) is one more operation to
  add in step 5.
- **fclane-lower** (fclane-design): the computed colour lane, with the select
  `MUX.v2i16 r0, 0x0, <v_vtxOut half>, mask` of the int and bool fragments.
- **intval fragment** (intval-rest): the fragment `ICMP_*.m1` / `LSHIFT_AND`
  compare of ints and bvecs (`fsivec4`, `fsmulti` are refused for the colour
  lane today).

## 5. Driver side: the bool word

Mesa stores a true bool as `~0` and every bool op above reads the word as a
mask. The GL core stores 1 (`src/gl/program.rs:2787`, `truth() as u32`), which
the other back ends need. With 1, `x` and `LSHIFT_AND` forms and `CSEL`/`MUX`
tests (against 0) are right, but the `.not` form is not: `~1` is
`0xFFFFFFFE`, non-zero, so `x == false` reads true for `x = 1`. The pinned
bool references compare only against `true` (`deqp_intval_uniapi_bool_vs`) and
no reference test runs a draw, so nothing catches it; the board would
(`uniform_api.value.*.bool` cases set 1).

Design: the compile record says which words are bools, the draw expands them.
`FauWord` gains `UniformBool { dword }` (the compiler pushes it for a
`GL_BOOL*` dword where it pushes `Uniform`), `UboRow` gains `bool: bool`;
`src/mali/program.rs:108` maps it to `PushWord::UniformBool { dword }` and
`frame.rs:3059` writes `if w != 0 { u32::MAX } else { 0 }` (and the same for a
row's words at `frame.rs:3010-3020`). A host test in `src/mali/frame_tests.rs`
(`PushWord::UniformBool` of 1 gives `0xFFFFFFFF`) and one in the compiler's
tests (the `FauWord` of `uniapi_bool_vs`). It changes the record of every shader
that reads a bool, so `tests/mali_deqp_intval.rs`'s two pins and
`tests/mali_deqp_unitype.rs`'s `vsbvec3` compare *program words* (unchanged) and
a record round trip only.

## 6. Steps, one session each

Order is by cases and by what must come first. Each step moves its shapes
from `REFUSED` to `LOWERED` in `tests/mali_deqp_unif2.rs` in the same commit,
removes the `#[ignore]`, and pins the refusal that remains where a shape is
not yet byte-equal.

**Step 1 (U0): tools, no compiler.** `tools/mali/gl_ref.py --uniform-values`
learns arrays (`glGetUniformLocation("u_var[1]")` each element) and struct
members (`"u_var.m0"`, `"u_var.m2[1]"`, `"s[1].b[2].b[1]"`), then redraw the
four struct references and the array ones so their FAU words carry values and
the member order is *visible*. Why first: the FAU word of a struct member is not
recoverable from the all-zero draw (the member order Mesa pushes is the block
order of the *used* members, but only a value draw shows which word is which),
and `tests/mali_ref` compares the FAU bytes. Files: `tools/mali/gl_ref.py`,
`tools/mali/test_gl_ref.py`, `tools/mali/testdata/deqp_unif2_{arr_*,fs_struct_uniform,vs_struct_uniform,vs_mediump_struct}.expect.json`
(regenerated by `drm_shim_ref.sh run ... --uniform-values`, note unif2 section
4). Un-ignores nothing. Risk: `refusals_pinned` and every test that loads these
files; the `deqp_uniarray_*` references have the same limit (no array values)
and may be redrawn the same way in this step, or not (their instruction words do
not change).

**Step 2 (D1): the bool word.** Section 5. Files: `src/shaderc/online/mali/fau.rs`
(`FauWord`), `select.rs` (`UboRow`, the push of a bool dword), `src/mali/program.rs`,
`src/mali/frame.rs`. Un-ignores nothing; no reference moves; it must land
before step 6 and with step 3 (the `.not` forms), because they are wrong with
1.

**Step 3 (M1, int and bool arrays, 226 cases).**
- `nir::array_element` (`nir.rs:1103`): add `GL_INT`, `GL_UNSIGNED_INT`,
  `GL_BOOL` (1, 1); `GL_INT_VEC2`/`BOOL_VEC2` (2, 2); `*_VEC3` (4, 3);
  `*_VEC4` (4, 4), the same stride rule as the float vectors (the front end
  pads a vec3 element to 4, `reflect.rs` `type_layout`; Mesa packs 3). Then
  `uniform_of`, `element_load`, `uniform_extent` take them with no other edit.
  Keep `fp16_uniform` (`nir.rs:1148`) and `uniform_idx` (`nir.rs:1197`'s `shape`)
  float-only: an int array is fp32 words, and its run-time index stays
  `UniformIndexed` (no reference).
- `UboLayout::vs` (`select.rs:417`) already lays an array one split row
  (`push_uniform`'s `_` arm): three int words are one row of 3, consecutive, each
  its own load. Check `join_loads`: the three words are *not* vectorised in
  Mesa (the references load each alone: `u5.w1`, `u6.w0`, `u6.w1`), which the
  `split` bit already says.
- `UboLayout::fs16` (`select.rs:579`): the `GL_INT | GL_BOOL` and `*_VEC*` arms
  drop `if u.array_len <= 1`; an array reaches the `is_matrix || (array_len > 1
  && !halves)` branch (`push_uniform`) with `halves = false`.
- the vertex compare chain: bool `x == false` as `LSHIFT_OR.i32 ... .not`,
  `x == true` as the word, ANDed by `LSHIFT_AND.i32`, one `CSEL.u32.eq`: step 6's
  `vs_reduce` generalisation, shared (do step 6 in the same session if the first
  `arr_bool_array` run stops there).
- the mediump int vertex (`arr_int_array`): `UniformPrecision` at `mod.rs:1364`
  and the `LoadUniform` guard `nir.rs:4455` stay **intval step 3**'s.

Word patterns: section 1.2 rows `arr_bool_array`, `arr_int_array`,
`fs_int_uniform`, `fs_bool_uniform`. Tests un-ignored (each needs its
prerequisite): `fs_bool_uniform` and `fs_int_uniform` (fragment: need intval's
fragment compare and fclane's `MUX.v2i16` colour), `arr_bool_array` (vertex:
step 6's chain; fragment: as the other two), `arr_int_array` (vertex: intval
step 3). The first action after the code is to run each pair and read the
refusal that follows: the one this design predicts is `FragColorLane` on the
fragment halves, and none on `arr_bool_array`'s vertex once step 6 is in.
Pinned tests most at risk: `tests/mali_deqp_uniarray.rs` (every `float_const_*`,
`vec{2,3,4}_const_*`, `loop_*_fs`, `dyn_vec*`: they use `array_element` and the
`push_uniform` split rows), `tests/mali_deqp_intval.rs` (`uniapi_*`, scalar bool),
`tests/mali_deqp_unitype.rs` (`vsbvec3`, `fsivec4`, `fsmulti`), the case 108,
122 and 166 references in `src/shaderc/online/mali/tests.rs` (the
`uniidx_*`, `intuni_*`, `unitype_166*` pins), `tests/mali_fau_page.rs`
(`sel_u_fau65`: the push order of many words).

**Step 4 (the host forecast of M1).** After step 3, `deqp-host-measure.sh
--filter uniform`, and a row of the case table of this note for the cases whose
only refusal was `UniformArray`/`FsUniform`. Not a code step; it fixes the
order of 5-8 on measured counts.

**Step 5 (M3, struct uniforms, 191 cases).** The front end already makes a
leaf row per member (names with a `.`; `m2` is one row of 3 with `array_len` 3;
`s[1].b[2].b[1]` is its own row), `mod.rs:1352` exempts a member from
`UniformPrecision`, `fp32` of `mod.rs:1448` keeps a member fp32. What remains:
- member arrays of int, bool, ivec4, bvec4: step 3's `array_element`
  (`name.contains('.')` needs no special case; `UboLayout::push_uniform` already
  treats an fp32 struct-member array as plain words);
- the word order: step 1 shows which of the 20 words is which member; if
  Mesa's push order differs from the declaration order the compiler's
  `UboLayout` rows follow (`fau.rs` `push_ubo` pushes in word order), the
  correction goes in the layout, not in the walk;
- `arr_struct_array`: the int member as a value, `float(s[0].c)`: `S32_TO_F32`
  of the word (`Alu::I2f`?; if intval step 3 does not admit `float(int
  uniform)`, add it in `nir.rs` `LoadUniform`), the nested vec2 member arrays
  (`array_element` GL_FLOAT_VEC2, existing), the f2f16 of fp32 members
  (`vsstruct_partial` pins the rule).
Un-ignores: `fs_struct_uniform` (needs the fragment bool/bvec compare of
intval and the colour), `vs_struct_uniform` (needs intval step 3 for the int
compares; after it, only the member arrays are this step), `arr_struct_array`
(2 cases; vertex only, the FS is `gl_FragColor = v_color`, so this is the first
of the three that can go green). Risk: `tests/mali_deqp_unitype.rs`
(`vsstruct_partial`, `vsstruct_rip`, `vsstruct_add`, `vsapi_struct`),
`tests/mali_deqp_uniarray.rs` (`struct`, `struct_member_vs/fs`).

**Step 6 (M4, bvec / bool compare chain in the vertex walk, 67 cases).**
`nir::vs_reduce` (`nir.rs:3612-3700`): today `Ball` of `Ieq` against one
constant, n = 3 (`vsbvec3`: `inot(ior(ior(y, x), z))`). Add n = 2..4 with a
constant bvec whose lanes mix `false` and `true` (per-lane `x == false` is
`inot(x)`, `x == true` is `x`): the reduction is the AND of the lane terms;
Mesa emits the `inot` first lane as `LSHIFT_OR.i32 r, 0x0, 0x0.b00, uN.not`,
the next `LSHIFT_AND.i32 r, uM, 0x0.b00, r`, the end `CSEL.u32.eq r, r, 0x0,
0x0, 0x3F800000`, with the lanes in the order the reference reads them
(`vs_bool_uniform`: x then y; `vsbvec3`'s y-before-x order is a different
form, an OR, and stays). `emit.rs` needs the `.not` source modifier on
`Alu::Ior`/`Iand` if it has none (`LSHIFT_OR`/`LSHIFT_AND` take it,
`select.rs`). The same scalar chain serves `arr_bool_array`'s vertex
(`[0] == false`, `[1] == true`, `[2] == true`, three compares ANDed). Un-ignores:
`vs_bool_uniform` (its fragment shader is the same compare with a colour select:
fclane), `arr_bool_array` (with step 3). Risk: `vsbvec3`, `uniapi_bool_vs`,
`uniapi_bool_initial_vs`, `tests/mali_vs_arith.rs` (`anyall`, `vsarith_86`).

**Step 7 (M5, two uniform words as one fp16 pair, 7 cases).** `nir_fs::vectorise`
(`nir_fs.rs:8764-8786`): when the two lanes are the low halves of two
*different* words (`ubo.half(d) = (w1, 0, base1)`, `ubo.half(e) = (w2, 0,
base2)`, `w1 != w2`), form `MKVEC.v2i16 rX, uA.wX.h0, uB.wY.h0`. A new
`Leaf::Mk2` (or `Leaf::Uniform` with a second word) in the same place that
`Leaf::Mk` is built (`nir_fs.rs:8750-8757`) and read at the selection (`Leaf`
handling in `select`), then consumed by the `FMA.v2f16` operand. Un-ignores
`fs_other` (its colour lanes are two operations on pairs; the pair of
`FMA.v2f16 r0, u3.w1, r0^.h00` is existing; the colour is the existing `MKVEC
r2, r0^.h0, u4.w1.h0`). Risk: `tests/mali_deqp_mixprec.rs`, `mali_fs_arith.rs`,
`mali_fragment_random.rs` (the pair forms).

**Step 8 (M6, a mat3 array element, 1 case).** `array_element` takes
`GL_FLOAT_MAT2/3/4` elements: (column stride x columns, rows) with the element
at front-end stride `cols * col_stride` (a mat3: 12 dwords, `type_layout`);
`push_uniform` rows one per column (`matrix_cols`) per used element, fp16 halves
for a mediump one (two words a column, the z half with a zero pad half,
`vs_mat_uniform` as the model); `fp16_uniform`'s array guard admits it.
Only element 2 is read in `arr_mat_array`; the layout may hold rows for all five elements (as a float array's today) as long as the push takes only the words the program reads (`fau.rs` `push_ubo`), which the reference shows as six words. Un-ignores `arr_mat_array`; 1 case, do it
last or fold it into step 3 if the per-element `used` rule is needed there
anyway (`arr_struct_array` pushes 12 of 34 words). Risk: the mat tests of
`mali_deqp_unitype.rs` (`vsmat2_*`, `vsmat3_*`, `fsmat*_mediump`).

**Step 9 (M2, check only).** fclane-lower lands the lane; the three shapes
`fs_mat_uniform`, `vs_mediump_array`, `vs_mediump_struct` then need no uniform
code but are not yet known to be byte-equal in their *vertex* program (their
pins today are the fragment `FragColorLane`, so the vertex words were never
compared): `vs_mediump_array`/`vs_mediump_struct`'s `compare_float` /
`compare_vec4` is Mesa's `FMAX.v2f16 .abs` chain with one `FCMP_OR.v2f16.lt.m1`
(`mali-deqp-uniarray.md` section 8 leaves it to `constop`:
`api_float`, `struct`), and the fragment halves read fp16 FAU words of an array
(`vs_mediump_array`: FAU 4-9 as `FADD_IMM.v2f16 r0, u3.w0, #imm`) or fp32 words
narrowed (`vs_mediump_struct`). Compare the vertex and the fragment region of
each with `mali_ref` before attributing a mismatch to the lane. If they differ,
that is arith-lower's (the `abs` / `FMAX` / `FCMP_OR.v2f16` chain), not this
note's.

## 7. Forecast

Upper bounds, to be measured after each step (section header, counts
`314fdc9c`): step 3 up to 226, step 5 up to 191, step 6 up to 67 (the 95 of
`arr_bool_array` are in step 3's number), step 7 7, step 8 1, M2 197 with
fclane-lower, `vs_int_uniform` 117 with intval step 3, 138 (`vs_mat_uniform`)
only at the re-measure. Together 944 of the 9,482 shader refusals of the
2026-11-17 board run, 1,092 with the seven shapes that already have references
(`mali-deqp-unif2.md` section 2), but as a *must-pass* number each shape counts
only when its other refusals are gone too (the `_both` cases, for instance `basic_array_assign_full.basic_array.bool_both`, need
both stages to compile).

dEQP groups unblocked (examples): `dEQP-GLES2.functional.uniform_api.value.*`
(`assigned.by_pointer.get_uniform.basic`, `basic_array_assign_full`,
`array_in_struct`), `shaders.struct.uniform.*`, `shaders.matrix.*` (the mat2/mat3
uniform shapes), `shaders.random.basic_expression.combined.*` (`fs_other`),
`shaders.indexing.matrix_subscript.*_vertex` (`vs_int_uniform`, step 3).

## 8. Open

- The struct member word order and the array element values: step 1.
- Whether intval step 3 covers `float(int uniform)` and the vertex bool compare
  (`vs_reduce`): read `mali-intval-vs16-lowering.md` when it merges and drop
  from steps 5-6 what it takes.
- `vs_struct_uniform`'s FAU constants 31-33 (the constants that are not byte
  immediates) are listed from the program text; their exact words come with the
  redraw of step 1.
- The 25/53/119 cases of M2 and the 117 of `vs_int_uniform` stay in other
  items; this note does not change them.

See also `docs/notes/mali-deqp-unif2.md` (shapes, counts, references),
`mali-deqp-unitype.md`, `mali-deqp-uniarray.md` (the earlier layouts).

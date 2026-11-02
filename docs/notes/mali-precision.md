<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Valhall precision: fp16 vertex uniforms (case 19) and mediump gl_FragCoord (cases 82, 84, 119)

Date 2026-10-24, tip `916266f9`. Plan #153 item `study-precision`, for issue
#1384. It builds on `docs/notes/mali-highp-fragcoord.md` (read it first; it is
not edited here): its 1.1 explains how Mesa decides fp16 or fp32, its 2.1 the
fragment-position lowering, and its open point 6 says "mediump arithmetic on
`gl_FragCoord` is not in any reference". This note closes that point with
references. Citations are Mesa 26.2.1 (`mesa-pan/mesa-26.2.1` in the deps
cache); `B/` is `src/panfrost/compiler/bifrost/`, `G/` is `src/compiler/glsl/`.
The words quoted are **read from the references** `tools/mali/testdata/prec_*.expect.json`
(Mesa 26.2.1's GL driver on drm-shim, GPU id 0xa867), not derived.

## The programs and the references

| Fixture | Case | Program |
|---|---|---|
| `prec_19.{vert,frag}` | 19 `es100` | PowerVR SDK vertex shader: `uniform mediump mat4 transformationMatrix; gl_Position = transformationMatrix * myVertex;` (`myVertex` highp), constant-colour fragment shader (`gl_test_shaders_es100.h:284-296`) |
| `prec_19b.{vert,frag}` | probe | `gl_Position = myVertex * u1 + u4 + uh` with `mediump vec4 u4`, `mediump float u1`, `highp vec4 uh`: the packing of mixed fp16 and fp32 uniforms |
| `prec_82.{vert,frag}` | 82 `fragcentre` | passthrough VS; `precision highp float`; `vec2 p = floor(gl_FragCoord.xy); vec2 f = fract(gl_FragCoord.xy);` then highp arithmetic (`:202-217`) |
| `prec_84.{vert,frag}` | 84 `fragcoord` | passthrough VS; `precision mediump float`; `vec4(gl_FragCoord.x / 64.0, gl_FragCoord.y / 64.0, 0.0, 1.0)` (`:503-511`) |
| `prec_119.{vert,frag}` | 119 `pbufup` | case 82's shader into an EGL pbuffer (`gl_test.c:20213-20224`) |

Recorded with (`$W` a shim install, `MESA_PAN_TREE` not needed):

```
tools/mali/drm_shim_ref.sh run $W tools/mali/testdata/prec_19.vert  tools/mali/testdata/prec_19.frag  tools/mali/testdata/prec_19  --uniform-values
tools/mali/drm_shim_ref.sh run $W tools/mali/testdata/prec_19b.vert tools/mali/testdata/prec_19b.frag tools/mali/testdata/prec_19b --uniform-values
tools/mali/drm_shim_ref.sh run $W tools/mali/testdata/prec_82.vert  tools/mali/testdata/prec_82.frag  tools/mali/testdata/prec_82
tools/mali/drm_shim_ref.sh run $W tools/mali/testdata/prec_84.vert  tools/mali/testdata/prec_84.frag  tools/mali/testdata/prec_84
tools/mali/drm_shim_ref.sh run $W tools/mali/testdata/prec_119.vert tools/mali/testdata/prec_119.frag tools/mali/testdata/prec_119
python3 -I tools/mali/pan_ref_diff.py extract tools/mali/testdata/prec_<n>.trace --draw > tools/mali/testdata/prec_<n>.expect.json
```

Only the `.expect.json` files are committed (the convention of `s4_ffma` and
the others; the `.trace`/`.stdout`/`.stderr` are what the `run` command writes). Two host faults had to be worked around to run the shim here, and
are not in the repository: the install lacked `lib64/panthor_dri.so` (a symlink
to `libgallium-26.2.1.so`; `dri` is a symlink to `.`), and this host's
`stdbuf -o0`, which `drm_shim_ref.sh run` wraps the draw in, replaces
`LD_PRELOAD`, so the shim was unloaded and `eglInitialize` failed with "failed
to create dri2 screen"; a no-op `stdbuf` earlier in `PATH` fixes it.

## Part 1. Case 19: a mediump uniform in the vertex shader

### Refusal

`src/shaderc/online/mali/mod.rs:1063`, in `compile_stage`'s vertex branch:
after `nir::build` and `UboLayout::vs` (`select.rs:268`) every uniform the
program reads whose declared precision is not `Precision::High` returns
`MaliError::UniformPrecision` (text `mod.rs:344-346`). Pinned by
`tests.rs:948` (`uniform mediump vec4 u; ... gl_Position = u;`).

### What Mesa does (`prec_19`, `prec_19b`)

**Why it is fp16.** `lower_variables_visitor::visit(ir_variable)`
(`G/lower_precision.cpp:1023-1060`) retypes a non-UBO float uniform of medium
or low precision to fp16 when the stage has `fp16_const_buffers`
(`:1025-1031`): the trace's NIR says `decl_var uniform ... mediump f16mat4
transformationMatrix`. The multiply has a highp operand (`myVertex`), so it is
not lowered; each fp16 lane is widened where used (`nir_op_f2f32`,
`B/bifrost_compile.c:3072-3075`, `bi_f16_to_f32_to`).

**The FAU layout** (words of the `vs.position` table; `uN.wK` is word `2N+K`):

- `prec_19`, the mat4 is **8 words, two fp16 components a word, column-major,
  low half first**: words 6-13 are `4c404c00 4cc04c80 4d404d00 4dc04d80
  4e404e00 4ec04e80 4f404f00 4fc04f80`, i.e. (16,17) (18,19) (20,21) ... (30,31)
  with `--uniform-values`' 16+c. Column `c` is words `6+2c`, `7+2c`. No padding
  between columns. The words before are the shared constants (viewport clamp
  etc.), after them `c7000000 47000000` and the three streamout offsets
  `4 8 12`, as for any vertex program.
- `prec_19b`, declaration order `u4`, `u1`, `uh`: `u4` words 6-7 (`4c404c00
  4cc04c80`: 16..19), `u1` **one word with the high half zero** (word 8,
  `00005000`: 32.0 in the low half), then the highp `uh` as **four fp32 words**
  (9-12: `42400000 ... 424c0000`, 48..51) starting on a fresh word. So: fp16
  uniforms pack halves within a uniform (a vec4 two words, a float one word,
  its high half zero, a vec3 would be two words with the high half of the
  second zero), each uniform starts a new word, and fp32 uniforms are not
  mixed into those words. This is the form `UboLayout::fs16` / `FauWord::UniformHalves`
  (`fau.rs:36-38`) already emits for a mediump fragment uniform.

**The words** (`prec_19.expect.json`, `vs.position`; the
shape of every lane of the product):

```
F16_TO_F32.wait0 r4, u4.w1.h1        ; one conversion per use: the high half of word 9
FMA.f32 r4, r4^, r1, 0x0.neg         ; first term: x * 0 - (-0) form, i.e. a multiply
F16_TO_F32 r5, u3.w1.h1
FMA.f32 r4, r5^, r0, r4^             ; accumulate
...
```

and for `prec_19b`: `F16_TO_F32 r4, u3.w1.h1; F16_TO_F32.wait0 r5, u4.w0.h0;
FMA.f32 r3, r3^, r5, r4^` (`myVertex.w * u1 + u4.w`). Findings:

1. The conversion is `F16_TO_F32 dst, uN.wK.hH`: a **separate instruction
   per fp16 lane per use** with the half selected on the FAU source (`.h0`
   low, `.h1` high). It is **not fused into the `FMA.f32` source**, and a lane
   used twice is converted once only if its two uses are close (the same
   `F16_TO_F32 r7, u5.w0.h1` register is reused by the next column), otherwise
   again.
2. The arithmetic is the highp vertex stage's `FMA.f32` chain unchanged
   (`FMA ... 0x0.neg` for the first term, then accumulating `FMA`), so the
   rest of the lowering is shared.
3. The order of instructions is Mesa's pressure schedule (conversions
   interleaved with the `FMA`s, `.waitN` flags), which our scheduler
   (`sched.rs::pressure_schedule`) will decide; the byte-equality test will
   show whether it does.
4. The fragment shader of the pair has the `fs.fragment` constant colour
   (`MOV.i32 r0, u2.w0`) with FAU `3c003c00 3c003948`, nothing new.

### Next refusals host compiling shows

Derived from the code path (`cargo` was not runnable on this worker, so
nothing was printed; the first step of the implementation is to print them):

- Lifting only `mod.rs:1063` is **wrong, not refused**: `UboLayout::vs` pushes
  `halves: false` (`select.rs:276-277`), so the mediump matrix would be laid out as
  sixteen fp32 words (the layout of the highp program) while Mesa's is
  eight fp16 words (above), and the draw would write fp32 data for words the
  program reads as halves. The layout and the refusal must change together.
- `nir::build` has only a 32-bit `Kind::Uniform` for the vertex stage; there
  is no half read and no `F16_TO_F32` row: `grep -n F16 bir.rs` is empty, the
  generated ISA has the instruction. That is the first missing piece.
- After that, the vertex shapes in the references pass the existing rules
  (`prec_19b`'s `fma(myVertex, u1, u4) + uh`, two operations).

### Implementation order for case 19

1. `UboLayout::vs`: take `halves` for a mediump uniform (reuse the `fs16`
   push; one word per uniform start, vec2 one word, vec3/vec4 two, a float one
   with a zero high half); `FauWord::UniformHalves` in the vertex FAU
   (`src/mali/program.rs`) and the fp16 data of the vertex push block
   (`frame.rs` `emit_stage_uniforms` already writes the fragment's).
2. A half-read node (`Kind::UniformHalf { dword, hi }`) and the
   `Op::F16ToF32` row (`bir.rs`, `pack.rs`, `emit.rs` `ROWS`); lift `mod.rs:1063`
   for float, vec2-vec4 and mat4 only (keep `UniformType` for the rest).
3. Byte-equal test of `prec_19` and `prec_19b` (`vs.position` words, `fau`,
   `preload`, `work_reg_count`) in `tests.rs`; `tests.rs:948` moves to a type
   still refused (a mediump array). `corpus-verdicts.txt` forward only.
4. Board: case 19 x2, then the sweep x2.

### Lowered (plan #185 item `vsfp16-19`, 2026-11-02)

Steps 1-3 are done; the board (step 4) is not. Byte-equal, both stages and
every fact, to `tools/mali/testdata/vsfp16_19_{hello,mixed,tex,vec4}.expect.json`
(drm-shim, `--uniform-values`): `tests.rs`
`the_fp16_vertex_uniforms_are_byte_equal_to_mesa_26_2_1`, `VSFP16_POPULATION`.
`hello` is the corpus's `es100/hello.vert` (prec_19), `tex` its
`textured.vert`, `mixed` a mediump vec4 before a highp vec4, and `vec4`
a mediump vec4 alone.

- **Which uniforms** (`nir.rs` `fp16_uniform`): a float, vec2, vec3, vec4
  or mat4, not an array, declared mediump or lowp. That is
  `lower_variables_visitor::visit` (`G/lower_precision.cpp:1023-1060`): a
  uniform outside a block whose base type is a float, when the stage has
  `fp16_const_buffers` (panfrost from v6, `pan_screen.c:588`). A struct
  member stays fp32 (its base type is a struct), so `mod.rs` gives the
  walk the plain uniforms only. A mediump array (which Mesa does lower)
  and a mediump struct member have no reference and stay
  `MaliError::UniformPrecision`, and so does any other type.
- **The node.** The walk's `LoadUniform` makes the same `Kind::Uniform`
  load (`Nir::load16`, `bits: 16`) and widens each lane it reads with its
  own `Alu::F2f32` (CSE'd like any ALU node). Everything after is the fp32
  vertex walk, unchanged.
- **The layout** (`select.rs` `UboLayout::vs`): a uniform read through a
  16-bit node is a `halves` row (`FauWord::UniformHalves`, the fragment
  stage's form). The frame already uploads such words for either stage
  (`src/mali/frame.rs` `PushWord::UniformHalves`, `f32_to_f16`), so
  `src/mali` did not change.
- **The selection** (`select.rs` `vs_variant`): lane `c` of a 16-bit
  uniform node is `Idx::Ubo { base, comp }` of its word, swizzled `H00` or
  `H11` for its half. `base` is the load's first word, so a mat4 column is
  its own base (the push keeps Mesa's column order). The existing
  `F16_TO_F32` row (`emit.rs`, case 121's) reads it as `uK.wJ.hH`.
- The scheduler and register allocator put the conversions where Mesa
  does (each before its `FMA`, `r7` reused), with no change to either.

The `tex` pair declares `TexCoord` and `Tint` in the opposite order in each
stage. Our varying locations follow each stage's declaration order, so the
test gives `compile_vs_for_reads` the fragment shader's reads at the vertex
shader's locations (Mesa's link matches them by name).

`es100/texfill.vert` (`uniform Fill u_fill`, a struct with default highp
members) was refused with the same text only because the precision lookup
found no row for `u_fill.matrix`. `mod.rs` `uniform_precision` now reads a
member's precision from the struct's spelling (`reflect.rs`
`UniformDecl::ty`), and the shader is `ok` on the existing fp32 path. No
reference draws a struct uniform's block layout yet.

## Part 2. Cases 82, 84, 119: mediump arithmetic on gl_FragCoord

### Refusal sites

`MaliError::FragCoordMediump` (`mod.rs:353-355`) is raised in `nir_fs.rs`:

| Site | Condition | Cases |
|---|---|---|
| `nir_fs.rs:1484` | any `LoadInput { slot: FragCoord }` in a **mediump** shader (`prec.highp` false) | 84 |
| `nir_fs.rs:1536` | an `Alu` in a **highp** shader with a source that is directly the `FragCoord` load (`fc`, `:1531`) | 82, 119 |

A highp shader that first copies `gl_FragCoord` to a highp variable is not
refused: `f.frag_coord(at)` (`:677-700`) builds the fp32 graph (`PixelCoord`,
`U2f32`, `Fadd 0.5`, `Ffma` by `FB_WPOS_Y_TRANSFORM`). Pinned by
`tests.rs:1014-1015`.

### What Mesa does

`gl_FragCoord` is mediump in ES 1.00 whatever `precision` says
(`G/builtin_variables.cpp:1333-1335`) and an input is not retyped, so an
operation applied to the builtin itself is lowered (`handle_precision`,
`G/lower_precision.cpp:295-312`): the position is computed in **fp32** exactly
as `mali-highp-fragcoord.md` 2.1, **narrowed once to an fp16 pair**, and the
operation runs on the halves; a highp consumer widens a lane back
(`F16_TO_F32`).

**The fp32 block is the same as the highp case, in a different order** (the
scheduler's), `prec_84`: `U16_TO_F32.discard r0, r59.h0` (x), `U16_TO_F32 r1,
r59^.h1` (y), `FADD_IMM.f32` 0.5 each, `FMA.f32 r1, r1^, u2.w0, u2.w1` (y
transform; FAU words 4-5 `3f800000 00000000` = (1, 0), the unflipped
framebuffer), preload `0x3800000000000000`, `work_reg_count` 32.

**The narrowing is the two-different-defs form** of `mali-highp-fragcoord.md`
open point 4 (x and y are an `FADD` and an `FMA` result, never one def):

```
FADD.f32 r0.h0, r0^, 0x0.neg       ; f2f16 of x into the low half
FADD.f32 r1.h0, r1^, 0x0.neg       ; f2f16 of y
MKVEC.v2i16 r0, r0^.h0, r1^.h0     ; the pair (x, y)
```

### Case 84 (`prec_84.expect.json`, mediump shader)

After the narrowing, the whole division is **one instruction**:

```
FMA.v2f16.wait0126 r0, r0^, u3.w0, 0x0.neg      ; (x, y) * (1/64, 1/64)
ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
MOV.i32.wait r1, 0x3C000000                      ; (0.0, 1.0) as halves
BLEND.slot0.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
```

`u3.w0` is FAU word 6 `24002400`: **`x / 64.0` is `x * 0.015625`** as a
halves pair, the multiply being `FMA.v2f16 ..., -0.0` (the same form
`select.rs` uses for the mediump `Fmul`). 14 instructions in all. No widening:
the colour is fp16.

### Cases 82 and 119 (`prec_82`, `prec_119.expect.json`, highp shader)

Only the two builtin calls are fp16; everything after them is the highp walk.
After the fp32 block (y first here: `U16_TO_F32.discard r0, r59.h1` is y, `r1`
is x) and the narrowing, where the pair's **low half is x**
(`MKVEC.v2i16 r0, r1^.h0, r0^.h0`):

```
FROUND.v2f16.rtn r1, r0            ; floor(xy) on both halves: p
FADD.v2f16 r0, r0^, r1.neg         ; fract(xy) = xy - floor(xy): f
F16_TO_F32 r2, r0.h1               ; widen f.y   (r0 = f)
FMA.f32 r2, r2^, u3.w1, 0x0.neg    ; f.y * 64
F16_TO_F32 r0, r0^.h0              ; widen f.x
FMA.f32 r0, r0^, u4.w0, r2^        ; f.x * 128 + f.y*64
F16_TO_F32 r2, r1.h1               ; widen p.y
F16_TO_F32 r1, r1^.h0              ; widen p.x
FMA.f32 r2, u3.w0, r2^, 0x0.neg    ; p.y * 4/255
FMA.f32 r1, u3.w0, r1^, 0x0.neg
FADD.f32 r2.h0 ... ; MKVEC.v2i16   ; colour narrowing (the highp colour store)
FMA.f32 r0, r0^, u4.w1, 0x0.neg    ; (.. ) / 255
...
```

(The full 30-instruction program is `prec_82.expect.json`; the FAU is words
6-11 `3c808081 42800000 43000000 3b808081 3c003c00`: 4/255, 64, 128, 1/255, the
halves pair.) Findings:

1. `floor` is **`FROUND.v2f16.rtn`** and `fract` is **`FADD.v2f16 xy, floor.neg`**,
   both on the one narrowed pair: a single narrowing is shared by the two calls
   (CSE before vectorising), not one per call.
2. Each half of `p` and `f` is widened separately, `F16_TO_F32 dst, r.hH`, at
   its use (the same instruction as case 19, the source a register half
   instead of a FAU half).
3. Everything after is the existing fp32 walk (`alu32`), and the colour
   narrowing at the store is the existing `FADD.f32 .h0` + `MKVEC.v2i16`.
4. **`prec_119` is word-for-word equal to `prec_82`**, `text` and `fau`
   (checked): the shim draws a framebuffer object, stored y-up like a
   pbuffer, so `T = (1, 0)` in both. 119 differs from 82 only on the board: the
   pbuffer is unflipped, the window surface flipped (`(-1, height)`), and the
   draw must write the transform word per target (`frame.rs`, `flip`;
   `mali-highp-fragcoord.md` open point 3). Back-face culling is draw state.

### Next refusals host compiling shows

Derived from the code path (see Part 1: `cargo` was not runnable here):

- **84, lifting `nir_fs.rs:1484`.** `frag_coord(at)` returns fp32 `Val::Lane`s,
  but a mediump shader's arithmetic is `Pairs` of fp16 (`Row { bits: 16,
  width: 2 }`, `select`/`vectorise`, `nir_fs.rs:636-`), whose leaves are
  `Leaf::Half`/`Leaf::Mk`. The fp32 lanes have to become a pair first: `Leaf::Mk`
  of two `F2f16 16 1` lanes (rows `emit.rs` already has) is exactly the
  `FADD.f32 .h0` + `MKVEC.v2i16` above. Without it the next verdict is
  `MaliError::Arithmetic` or a `Defect` at the pair construction. The multiply
  by the exact constant pair `24002400` is then the existing mediump rule
  (`F16X2_*`, `nir_fs.rs:42`), `FMA.v2f16 ..., -0.0`.
- **82/119, lifting `nir_fs.rs:1536`.** `alu32` would accept `floor`/`fract` of
  the fp32 position and emit `FROUND.f32.rtn`: **no refusal, wrong words** (and
  a different value above 2048). There is no 16-bit `FROUND` or
  `FADD.v2f16 .neg` row on this path for the builtin. So the guard must be
  replaced by the half path, not removed.

### Implementation order for 82/84/119

1. The narrowing node: a pair of the two fp32 frag-coord lanes
   (`Alu::F2f16` lane by lane, `Alu::Vec2`), emitted as above, shared by every
   use of the builtin (one `MKVEC` per shader).
2. Case 84: lift `:1484` with the narrowed pair feeding the existing 16-bit
   multiply; byte-equal to `prec_84` (14 instructions, FAU
   `... 3f800000 00000000 24002400`).
3. Cases 82/119: a half `floor` (`FROUND.v2f16.rtn`) and `fract` (`FADD.v2f16`
   with the floor negated) over the shared pair, and the `F16_TO_F32` row of a
   register half (shared with case 19); lift `:1536` only for `floor`/`fract`
   applied directly to the builtin, keep it for any other operation.
4. Tests: byte-equal `prec_82` (and `prec_119` as the same words),
   `prec_84`; `tests.rs:1014-1015` move to a still-refused shape (mediump
   `sqrt(gl_FragCoord.x)`). `corpus-verdicts.txt` forward only.
5. Board: cases 82, 84, 119 x2, then the sweep x2; 119 additionally proves the
   unflipped transform word on a pbuffer.

## Order across the four cases

`F16_TO_F32` is shared (case 19 from the FAU, cases 82/119 from a register):
do case 19 first, then the narrowing with case 84, then 82/119. Nothing in
`src/` changed in this item.

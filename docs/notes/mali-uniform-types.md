<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): uniforms other than float/vec/mat4, and uniforms past the FAU

Issue #1384 / #1385, plan #153 item `study-unitype`. gl_test cases 100
(`idxtex`) and 116 (`uniidxe`) are refused with `Valhall: a uniform other than
a float, vec2, vec3, vec4 or mat4 is not lowered yet`; case 166 (`fshprobe`)
with `Valhall: a uniform past the FAU's 128 words (Mesa loads it from the UBO)
is not lowered yet` (`0dce654a8:tools/mali/board-ref/2026-10-23-s6-discard/ours/03-panthor-gltest--sweep-1.log`,
cases 100, 116, 166). This note says which types the three programs use, what
Mesa 26.2.1 does with each (recorded, section 6), where our refusals are, and in
what order to lower them.

**Where the words come from.** Mesa 26.2.1's gallium panfrost on the drm-shim,
built with `tools/mali/drm_shim_ref.sh build` (the tarball at
`$CONNECTED_DEPS_DIR/scratch/tfdesign/mesa-26.2.1.tar.xz`) and run with
`tools/mali/drm_shim_ref.sh run`, one draw per pair, the trace reduced by
`tools/mali/pan_ref_diff.py extract <trace> --draw`. The results are
`tools/mali/testdata/unitype_<case>.expect.json` (words, disassembly, FAU,
register and preload facts) next to their sources (section 6 lists the exact
commands). `docs/notes/mali-uniform-index.md` (the run-time-indexed read) and
`docs/notes/mali-uniform-arrays.md` (case 108, a float array) are read first and
not repeated.

## Citations

- Mesa: `M:<path>:<line>`, `<path>` under `src/` of the Mesa 26.2.1 tree
  (`MESA_PAN_TREE`, `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`).
- Ours: `<path>:<line>` from the repository root at `916266f9`.

## The answers

(Since plan #154 item intbool, `nir::uniform_of` takes `int`, `bool` and
`ivec2/3/4` as a word a component, step 1 of section 5, and a highp
fragment shader reads an int uniform as a value; the vertex stage still
refuses one read other than as an index. `docs/notes/mali-int-uniforms.md`.)

1. **Cases 100 and 116 fail on `int` and nothing else.** Their uniforms are
   `vec4 xcols[4]` (an array: refused first, as `UniformArray`, in the vertex
   stage), `uniform int xidx` (the type that raises `UniformType`) and, in 100
   only, a `sampler2D` per stage (no block word, `SHC_UNIFORM_NO_BLOCK`, so not
   a type refusal). No `bool`, `mat2`, `mat3`, `ivec` or sampler array is in
   either program; the corpus holds none either (section 1).
2. **Mesa packs an `int` (and a `bool`) as one 32-bit word** in the uniform
   block, like a float; only the read differs: raw bits, not a float. Recorded:
   the `int` is pushed to the FAU (a constant-offset read at the word after the
   array) and used raw as `LSHIFT_OR` input to build the `LD_PKA` byte address,
   in every stage (`unitype_100`, `unitype_116v`, `uniidx_d`). The mediump
   fragment back end already takes it (`GL_INT`, `src/shaderc/online/mali/select.rs:332`;
   `uniidx_d`, `uniidx_i`). The vertex stage (`nir::uniform_of`) and the highp
   fragment stage (`uniform_extent`, which ends in `uniform_of`) do not.
3. **Case 166 does not need Mesa's UBO fallback in arms A and B.** Recorded:
   `unitype_166a` pushes 16 FAU words and `unitype_166b` 28, with no `LD_PKA`
   and no `LD_BUFFER`/`LOAD`: every read is a constant-offset FAU read of a
   mediump (fp16 pair) array. Our refusal is `MAX_UBO_ROWS = 32`
   (`select.rs:236`, raised at `:258`): `fs16` makes one row per *element* of
   `u_p[64]`/`u_p[70]`, and the message says "past the FAU" for a table-size
   limit of ours (section 3). The real FAU overflow (a constant-offset read
   past word 128) is `sel_u_fau130`'s `LD_PKA` and is a separate row.
4. **Arm C of 166 is a different refusal behind it**: 16 samplers and 16
   look-ups; the fragment back end takes one look-up (`MaliError::TexCount`,
   `src/shaderc/online/mali/nir_fs.rs:1668`). Recorded in `unitype_166c`
   (116 instructions, 30 FAU words). That is the texture study's, not this
   one's; 166 stays refused after arms A and B pass.
5. The order is section 5.

## 1. Which shaders use which uniform types

| case | stage | file | uniforms | refusal today |
|---|---|---|---|---|
| 100 | vertex | `tools/shaders/es300/idxtex.vert` | `highp sampler2D vtex`, `vec4 vcols[4]`, `int vidx` | `UniformArray` (the array, first), then `UniformType` (the `int`) |
| 100 | fragment | `tools/shaders/es300/idxtex.frag` | `highp sampler2D ftex`, `vec4 fcols[4]`, `int fidx` | `UniformType` |
| 116 | vertex | `tools/shaders/es100/uniidx_e.vert` | `vec4 vcols[4]`, `int vidx` | `UniformArray`, then `UniformType` |
| 116 | fragment | `tools/shaders/es100/uniidx_d.frag` (mediump) | `vec4 fcols[4]`, `int fidx` | none: `ok` in `tools/mali/corpus-verdicts.txt:105` |
| 166 | vertex | `tools/shaders/gltest/fshp_vs.vert` | none | none (`ok`, `corpus-verdicts.txt:233`) |
| 166 | fragment A, B | built by `shp_src` (`driver/tools/pvrlab/gl_test.c:9540-9566`) | mediump `vec4 u_p[64]` / `u_p[70]` | `UboFallback` (row table) |
| 166 | fragment C | built by `fshp_units_src` (`gl_test.c:9680-9700`) | mediump `vec4 u_w[4]`, 16 `sampler2D` | `TexCount` after A, B |

The case 100 fragment program is `idxtex.frag` (ES 3.00, `precision highp`),
116's is `uniidx_d.frag` (ES 1.00, mediump). Only the highp fragment stage and
both vertex stages raise `UniformType`: the corpus lines are
`corpus-verdicts.txt:106` (`uniidx_e.vert`), `:137` and `:138`
(`idxtex.frag`, `idxtex.vert`). The texture look-up of `idxtex` is a constant
coordinate (`vec2(0.25)`, `vec2(0.75)`) and `textureLod(..., 0.0)` in the
vertex stage, the vertex-stage look-up being a texture item of its own
(`docs/notes/mali-texture-refusals.md`).

## 2. Mesa's layout, type by type (FAU and UBO)

The block is Mesa's *packed* uniform storage: `nir_lower_io` with
`glsl_count_dword_slots` (`M:compiler/glsl_types.c:2961-3000`), then
`nir_lower_uniforms_to_ubo` makes it UBO 0 (`mali-uniform-index.md`, section 1).
Nothing is aligned, nothing is padded between uniforms; the stage's *used*
uniforms are laid out in declaration order (unused ones get no word:
`sel_u_unused`).

| GLSL type | dwords in the block | read as | FAU after push | status here |
|---|---|---|---|---|
| `float`, `int`, `bool`, `uint` | 1 | one word; `int`/`bool` are raw bits, no conversion | one FAU word (`u<n>.w<h>`) | float: yes; int: fs16 only, recorded in all stages; bool: no reference |
| `vec2/3/4`, `ivec2/3/4` | 2 / 3 / 4 | a word per component | consecutive words (`sel_u_float_vec4`: vec4 at word 1) | float: yes |
| `mat4` | 16 | column by column, 4 words each | the words read, in ascending order | yes (`sel_u_float_mat4`) |
| `mat2`, `mat3` | 4 / 9 (`glsl_get_components` = columns x rows; **no** column padding in the packed layout) | by column | the words read | refused (`GL_FLOAT_MAT2/3` fall to `_ =>`, `nir.rs:568`); no reference (by the source alone) |
| mediump float scalar/vector/matrix, fragment stage | `DIV_ROUND_UP(rows, 2)` per column: a mediump `vec4` is 2 dwords | two fp16 lanes a word, read with `.h0/.h1` or as `v2f16` | the word pair | `fs16`; vertex stage: `UniformPrecision` (case 19, another study) |
| `sampler2D` etc. | **0** (`is_bindless` false) | a descriptor in the resource table, not a block word | none (the texture handle `0x2000180N` is a FAU constant, `unitype_166c`) | not a block uniform |
| array of T | `count(T) * length`, element stride `count(T)` dwords | element offset constant: pushable; run-time: `LD_PKA` | the words read | case 108 (`mali-uniform-arrays.md`) |

### The recorded words for `int`

**Fragment, mediump** (`uniidx_d.expect.json`, and `unitype_116.expect.json`,
the same program drawn with 116's vertex shader):

```
MOV.i32.discard r0, 0x3020100
LSHIFT_OR.i32 r0, u2.w0, r0^.b33, 0x0           ; fidx << 3 (fp16 vec4: 8 bytes)
LD_PKA.i64.unsigned.slot0.wait0126 @r0:r1, r0^, 0x0     ; 406a80f42800c040
```

The FAU is six zero words (4 blend constants, then `fidx` at word 4 =
`u2.w0`, then a zero pad): `fidx` is *pushed*, a constant-offset read at word 8
of the block, and read raw.

**Fragment, highp** (`unitype_100.expect.json`, `fs.fragment`; `idxtex.frag`
as ES 3.00): the vec4 array is highp, so the stride is 16 bytes and only `.ba`
is used:

```
MOV.i32 r0, 0x7060504
LSHIFT_OR.i32 r0, u2.w0, r0^.b00, 0x0           ; fidx << 4
...                                              ; the TEX_SINGLE (coordinate 0.75 = u5.w0)
IADD_IMM.i32 r0, r0^, #0x8                       ; + the 8 bytes of .rg
LD_PKA.i64.unsigned.slot1.wait01 @r0:r1, r0^, 0x0
```

The FAU is `[0 0 0 0 | fidx | 0x3f400000]`: the four blend constants, the raw
`int` (word 4), the texture coordinate `0.75` (word 5); the texture handle is
an immediate (`IADD_IMM.i32 r1, 0x0, #0x20001800`, the texture/sampler
handle as in `mali-texture-refusals.md`). The shift is a `bi_lshift_or` whose
shift amount is a byte lane of a constant: `0x3020100`'s `.b33` (3) for the
8-byte mediump element, `0x7060504`'s `.b00` (4) for the 16-byte highp one.

**Vertex, highp** (`unitype_116v.expect.json` `vs.varying` and
`unitype_100.expect.json` `vs.varying`, case 116's `uniidx_e.vert` with a
fragment shader that reads its varying, and 100's `idxtex.vert`). The *position*
program does not read the index (the index only feeds a varying), so the
read is in the `varying` program:

```
MOV.i32 r0, 0x7060504
LSHIFT_OR.i32 r0, u5.w1, r0^.b00, 0x0           ; vidx << 4
LD_PKA.i128.unsigned.slot0.wait0 @r0:r1:r2:r3, r0^, 0x0     ; 086a80f83800c040
FMA.f32 r0, r0^, 0x0, r0^  ...                  ; the varying passed on (116)
```

`LD_PKA.i128` is the highp `vec4` as one 16-byte load. In case 100's vertex
stage the varying carries `vcols[vidx]` only through `v_col.rg`, so Mesa loads
`LD_PKA.i64.unsigned.slot1.wait01 @r0:r1, r0^, 0x0` (two components) and adds
the `TEX_SINGLE.slot0.reserved.32.rg.2d.zero` result (a look-up at lod 0, the
coordinate `0.25` and the handle `0x20001800` as FAU words). The vertex FAU
(`fau` in the extract; 12 words in 116v, 14 in 100) ends in:

| words | content |
|---|---|
| 8-10 | `4`, `8`, `12`: the varying buffer's output offsets (`u4.w0`, `u4.w1`, `u5.w0`) |
| 11 | `vidx`, raw (`u5.w1`) |
| 12-13 (100 only) | `0x3e800000` (the coordinate `0.25`), `0x20001800` (the texture handle) |

and begins with the position program's eight constants (`0x42000000` x4,
`0x3f000000` x2, `0xc7000000`, `0x47000000`), the same in both programs.

Two facts that matter for the lowering:

- **The unused read disappears.** In case 116 itself (`uniidx_e.vert` with
  `uniidx_d.frag`) the fragment shader does not read `v_color`, so Mesa deletes
  the vertex stage's `vcols[vidx]` read: `unitype_116.expect.json` has no
  varying program (only `vs.position` and `fs.fragment`). `unitype_116v` is the same vertex shader with a fragment shader
  that reads the varying, to record the read that is otherwise dead.
- **The index is not clamped.** `vidx << 4` goes straight to the load; there is
  no `IMIN`/`CLAMP` (the array is declared, an out-of-range index is undefined
  in GLSL ES).

### Not recorded

- A `bool` uniform, `mat2`/`mat3`, an `ivec` (no program of the corpus declares
  one; `src/shaderc/online/mali/tests.rs:946` asserts the `mat2` refusal as the
  pinned gap). Mesa's layout for them follows the table above by the source
  alone; record the reference when a corpus program needs one.
- `gl_ref.py --uniform-values` sets float uniforms only, so an `int` is 0 in
  every recorded FAU (the value does not change the instructions).

## 3. Case 166: what Mesa pushes and what it loads

### How Mesa decides FAU or UBO

`bi_opt_push_ubo` (`M:panfrost/compiler/bifrost/bi_opt_push_ubo.c:149-210`):

1. A fragment shader first pushes the four blend constants (`:94-114`), the
   first four FAU words.
2. `bi_analyze_ranges` (`:50-89`) collects, for every *pushable* load (both the
   address and the buffer constants, buffer in `pushable_ubos`, offset word
   aligned: `:21-33`), the range `[word, word + channels)`.
3. `bi_pick_ubo` (`:120-147`) walks UBOs **last first** (the sysval UBO, then
   UBO 0) and each UBO's ranges by ascending word. It emits a range whole;
   **at the first range that does not fit the remaining FAU
   (`pan_fau_available(fau) < range`) it returns, so every later range of every
   UBO is left unpushed** (not just that one).
4. A load whose word was not picked keeps its `LD_PKA` with the constant byte
   address (`:171-187`), and sets bit 0 of `ubo_mask`: the block is then
   uploaded and has a UBO descriptor (`mali-uniform-index.md`, section 2).
5. The FAU holds 128 32-bit words, two a slot (`PAN_MAX_PUSH`,
   `M:panfrost/compiler/pan_compiler.h:137`); the constants the later
   passes promote (`0x4700...`, `0x3f800000`) are appended **after** the pushed
   words from the same 128 (`sel_u_fau65`: the 65th word lands in page 1).

### The pinned reference of the overflow

`tools/mali/testdata/sel_u_fau130.vert`: 31 `vec4` read in `.w` (124 words)
after the 6 sysval words: the last `vec4` (`u30`, word 120, byte 480) does not
fit and is loaded whole:

```
LD_PKA.i128.unsigned.slot0 @r0:r1:r2:r3, u63.w0, 0x0   ; 026a80f83800c0be
```

`u63.w0` is the promoted constant `0x1e0` (byte 480), the mode descriptor `0x0`
is UBO 0 of the resource table (`pan_nir_res_handle(PAN_TABLE_UBO, 0, 0)`,
`M:panfrost/compiler/pan_nir.h:51-61`). The program's `.w` is then read from
the loaded register (`r3`). This is the form of a constant-offset read from the
UBO: the same `0x6A` opcode (`LD_PKA`) as the run-time read, not `LD_BUFFER`
(Valhall's `LD_BUFFER` is the *buffer-texture* load, not this one) and not
`LOAD.i32`. The widths follow the load: `i128` for a four-word vec4, `i64`,
`i32` otherwise, by the same alignment splitting as `mali-uniform-index.md`
section 1.

### What 166's programs make Mesa do (recorded)

`unitype_166a.frag`/`166b.frag` read, with constant indices only,
`u_p[56..58]` (the twelve selector floats) and `u_p[0,32,60,63]` (arm A) or
additionally `u_p[64..67, 69]` (arm B). The array is mediump, so each `vec4` is
two dwords of fp16 and a whole-vec4 read is a two-word range. Recorded
(`unitype_166a/166b.expect.json`):

| arm | `fs.fragment` words | FAU words | `LD_PKA` | `ubo_mask` bit 0 |
|---|---|---|---|---|
| A (`u_p[64]`) | 22 | 16 (the 4 blend constants first) | none | clear |
| B (`u_p[70]`) | 40 | 28 (the 4 blend constants first) | none | clear |
| C (`u_w[4]` + 16 samplers) | 116 | 30 (4 blend + 8 + `0.5` + 16 texture handles + pad) | none | clear |

Every uniform is a `MOV.i32 rN, uK.wH` or an `FMA.v2f16` source straight from
the FAU (`FMA.v2f16 r0, u2.w1.h00, u2.w0, r0^.neg`, `MOV.i32 r1, u7.w1`): with
the 128 words the FAU never fills, so **no uniform of 166 is loaded from the
UBO and no UBO descriptor is needed**. The cell's colour sums are FMA chains of
FAU words; the first non-fitting range of `bi_pick_ubo` only matters at word 128.

The generic fallback, should a program ever read more than the FAU holds (the
vertex stage of the same probe, case 150, has six sysval words before its
uniforms and a highp fp32 array), is exactly `sel_u_fau130`'s: the first
non-fitting range and all after it become `LD_PKA` with a constant offset, and
the frame supplies the UBO of `mali-uniform-index.md`, section 3.

### The UBO descriptor the frame must supply

Entry 0 of the UBO table is a `Buffer` descriptor: `Size` = the used block's
exact bytes (`UboLayout::bytes`, `select.rs:252`; `frame.rs::ubo_block_bytes`
for the frame), `Address` = the uploaded packed block. The frame already builds
it for the run-time-indexed read: `ShaderMeta::ubo` (`src/mali/frame.rs:581-584`,
rows in Mesa's packed layout, empty when every read is pushed, so `ubo_mask`
bit 0 is clear), the block's `Size` check at `frame.rs:2018-2019`
(`NOT_BUILT_UBO_BLOCK`) and the UBO table whose last entry is the sysval block
(`frame.rs:576-577`, `:2357-2384`). Nothing of the descriptor is new for
a constant-offset read past the FAU: it is the same block with more rows.

## 4. The refusal sites

| refusal | site | what it checks |
|---|---|---|
| `UniformType` raised | `src/shaderc/online/mali/nir.rs:568` (`uniform_of`, the `_ =>` arm of the `gl_type` match at `:562-568`) | float, vec2, vec3, vec4, mat4 only: `GL_INT`, `GL_BOOL`, `GL_FLOAT_MAT2/3`, `ivec`, `bvec` fall through |
| | called for the vertex stage at `nir.rs:1376` (`Uniform` node), `:1912` and `:1256`, the layout at `select.rs:278` (`UboLayout::vs`) | |
| | called for the highp fragment stage by `uniform_extent` (`nir_fs.rs:1771-1787`, ends in `uniform_of` at `:1787`), from `select.rs:297` (`fs32`) | |
| `UniformType` message | `src/shaderc/online/mali/mod.rs:126` (variant), `:337-339` (text) | |
| `UniformArray` (hit first on arrays) | `nir.rs:559-561`, `nir_fs.rs:1780-1783` | any array in the vertex stage, any but a vec4 array in a fragment |
| `FsUniform` (mediump fragment) | `select.rs:337` (`_ =>` of the type match in `fs16`, `:331-337`), `:339-341` (a highp-declared uniform) | float, vec2/3, vec4, vec4 array, int |
| `UboFallback`, rows | `select.rs:258` (`UboLayout::push`: more than `MAX_UBO_ROWS = 32` rows) | 166 A and B: 64 and 70 element rows |
| `UboFallback`, word table | `src/shaderc/online/mali/fau.rs:159` (`ubo_range` has `BASES = 512` entries) | an index past 512 words |
| `UboFallback`, past the FAU | `fau.rs:188` (a pushed read whose range did not fit: `ubo_first[base] == u16::MAX`), after the loop `:166-176` that stops at the first non-fitting range | the shape of `sel_u_fau130`; the message at `mod.rs:356-358` |
| `TexCount` | `nir_fs.rs:1667-1668` | 166 arm C: the second look-up |

Two things about the sites: the `UboFallback` message is the same for the real
overflow (`fau.rs:188`) and for our own table limits (`select.rs:258`,
`fau.rs:159`); lifting the row limit changes what 166 reports, not the
truth of "past the FAU". And `fau.rs:166-176` `break`s at the first
non-fitting range exactly as Mesa's `return` does (`bi_pick_ubo:134-135`), so
the push order is already Mesa's; only the `LD_PKA` the leftover reads need is
missing.

## 5. The words to match, and an implementation order

The words to match are all in `tools/mali/testdata`:

| program | file | region(s) |
|---|---|---|
| 116 vertex (live read), `int` + vec4[4] | `unitype_116v.expect.json` | `vs.varying`, `vs.position`, `fs.fragment` |
| 116 as the corpus has it | `unitype_116.expect.json` | `vs.position` (no read), `fs.fragment` = `uniidx_d` |
| 100 (both stages, highp, with a texture) | `unitype_100.expect.json` | `vs.position`, `vs.varying`, `fs.fragment` |
| 166 A, B (pushed arrays) | `unitype_166a.expect.json`, `unitype_166b.expect.json` | `fs.fragment` |
| 166 C (samplers) | `unitype_166c.expect.json` | `fs.fragment` (texture study) |
| a read past the FAU | `sel_u_fau130.expect.json` | `LD_PKA.i128 ... u63.w0` |

Order (each step keeps `tools/mali/check.sh` and `corpus-verdicts.txt` green,
moving forward only):

1. **Raw-bit uniform in the vertex and highp fragment stages**: accept
   `GL_INT` (and `GL_BOOL`, one word, `0`/`1`) in `uniform_of`
   (`nir.rs:562-568`) as a one-word raw uniform, as `fs16` already does
   (`select.rs:332`). Declared by itself it unlocks nothing; the corpus rows
   that read an `int` only to use it move, `uniidx_e.vert` still waits for
   step 2.
2. **The run-time-indexed `LD_PKA` in the vertex and highp fragment stages**
   (`mali-uniform-index.md`, section 5, `uniform-index-122`, and
   `mali-uniform-arrays.md`): the `vcols[vidx]` read of 100 and 116 in the shape
   recorded in section 2 (`LSHIFT_OR` of the raw index with the byte-lane
   constant, `LD_PKA.i128`/`.i64`, an `IADD_IMM #8` for a `.ba`-only read),
   with the frame's table entry 0 (section 3). Case 116 passes here; case 100
   then waits for its vertex-stage `textureLod` (`mali-texture-refusals.md`).
3. **`MAX_UBO_ROWS` for arrays**: make an array one row of `length * comps`
   words, not a row per element (`select.rs:286-297`, `fs16` loop at
   `:343`), or raise the limit, so 166 A and B reach the push. A, B then need
   only constant-offset reads and the existing push; **no new instruction**
   (section 3, recorded).
4. **The constant-offset `LD_PKA`** (`sel_u_fau130`) for a read past the FAU:
   a `Ld Ubo` BIR op whose offset is a promoted constant, with `ubo_mask` bit 0.
   This is `fau.rs:188`'s `UboFallback`, and it is the same op as step 2's, so
   do it with step 2 if the item is one.
5. `mat2`, `mat3`, `bool`/`ivec` and the other types: by the table in
   section 2, only when a program of the corpus needs them (none does today);
   record the references then.
6. Case 166 arm C is the texture study's (`TexCount`, 16 samplers).

## 6. The references and how they were made

From the repository root; `<work>` is the `drm_shim_ref.sh build` directory
(`$CONNECTED_DEPS_DIR/mesa-shim`), `D=tools/mali/testdata`:

```
tools/mali/drm_shim_ref.sh run <work> $D/unitype_116.vert $D/unitype_116.frag  $D/unitype_116
tools/mali/drm_shim_ref.sh run <work> $D/unitype_116.vert $D/unitype_116v.frag $D/unitype_116v
tools/mali/drm_shim_ref.sh run <work> $D/unitype_100.vert $D/unitype_100.frag  $D/unitype_100
tools/mali/drm_shim_ref.sh run <work> $D/unitype_166.vert $D/unitype_166a.frag $D/unitype_166a --uniform-values
tools/mali/drm_shim_ref.sh run <work> $D/unitype_166.vert $D/unitype_166b.frag $D/unitype_166b --uniform-values
tools/mali/drm_shim_ref.sh run <work> $D/unitype_166.vert $D/unitype_166c.frag $D/unitype_166c --uniform-values
python3 -I tools/mali/pan_ref_diff.py extract $D/unitype_<x>.trace --draw > $D/unitype_<x>.expect.json
```

The `.trace` files are not kept (as for the other `testdata` references); the
`expect.json` holds the words, text, FAU, `work_reg_count` and preload of each
region. `--uniform-values` is omitted where an `int` is declared (it sets float
uniforms only: `tools/mali/gl_ref.py`, the `UNIFORM_COMPONENTS` table), and it
reports `u_p` and `u_w` "at -1" (an array name that `glGetUniformLocation`
does not resolve without `[0]`), so the FAU of 166 holds zeros where the
values would be.

Case 100 is ES 3.00. `gl_ref.py` opens an ES 3.0 context when a shader starts
with `#version 300 es` (`is_es3`, texmore2) and draws `idxtex.vert` and
`idxtex.frag` as they are, so `unitype_100.vert`/`.frag` are those two files
and the reference is the real ES 3.00 compile. (An ES 1.00 twin recorded first
had the same vertex stage; its fragment epilogue differed, `gl_FragColor`
being mediump where `fragColor` is highp, so it was not kept.)

`drm_shim_ref.sh run` preloads only the shim: its former `stdbuf -o0`
replaced the shim's `LD_PRELOAD`, so `eglInitialize` ran against the host's own
render node and failed with "failed to create dri2 screen" (shimfix removed
it). The shim build's Gallium driver is `libgallium-26.2.1.so` in
`mesa-shim/inst/lib64`; there is no separate `panthor_dri.so`.

Sources in `tools/mali/testdata`:

| file | what |
|---|---|
| `unitype_116.vert`, `.frag` | case 116's pair, `uniidx_e.vert` and `uniidx_d.frag` verbatim |
| `unitype_116v.frag` | a fragment shader that reads 116's varying (the read is otherwise dead) |
| `unitype_100.vert`, `.frag` | case 100's pair, `idxtex.vert` and `idxtex.frag` verbatim (ES 3.00) |
| `unitype_166.vert` | `gltest/fshp_vs.vert` |
| `unitype_166a.frag`, `unitype_166b.frag` | `shp_src` regenerated for `n_vec4` 64 and 70 |
| `unitype_166c.frag` | `fshp_units_src` regenerated |

## 7. Case 80 (`matdyn`): an `int` uniform that indexes

Plan #153 item `study-matdyn-ifbranch`, issue #1386. The newest sweep
(`0dce654a8:tools/mali/board-ref/2026-10-24-refusals/ours/15-panthor-gltest--sweep-1.log`,
`verdict case 80`) refuses it with the same `UniformType` message as 100 and
116, and `corpus-verdicts.txt` lists `tools/shaders/es100/matdyn.frag` the same
way.

**The uniform types.** The fragment shader declares `uniform vec4 u_k;
uniform float u_c; uniform int u_i;` (the vertex stage, `gl_test_es100/passthrough.vert`,
declares none). There is no mat2/mat3 or matrix-array uniform: the
"non-symmetric mat2" is a local built from `u_k`. The only type outside the
lowered set is therefore the **`int` `u_i`**, which is used as a run-time index
into a local `float[3]`, a mat2 column (`p[i]`), and a vector lane (`col[i - 1]`,
`v[j]`, `j = i + 1`). This is 100's and 116's category (`int` uniform, section 1)
and not 166's.

**Mesa's layout** (`tools/mali/testdata/unitype_80.expect.json`, Mesa 26.2.1):
the three uniforms are one UBO block, pushed to the FAU in declaration order,
word by word, as in section 2: NIR has `load_ubo(0, 0)` as a vec4 (range 16,
`u_k`) and `load_ubo(0, 16)` as a vec2 (range 8: `u_c`, `u_i`). In the FAU
(`fs.fragment.fau`, 16 words, the first 10 zero because the shim sets no values):

| FAU slot | word | content |
|---|---|---|
| `u2.w0`, `u2.w1`, `u3.w0`, `u3.w1` | 4-7 | `u_k.xyzw` |
| `u4.w0` | 8 | `u_c` (float) |
| `u4.w1` | 9 | `u_i` (a 32-bit int word, read raw) |
| `u5.w0`..`u7.w1` | 10-15 | constants 0.0625, 0.5, 0.25, 0.125, 0.375, `0x3c003c00` |

`u0`/`u1` are zero in the fragment stage, like the other recorded fragments.
So the `int` costs no layout: it is a plain 32-bit word next to the floats; what
is new is how it is consumed.

**The instruction words.** The fragment shader is 61 instructions (shaderdb;
60 listed in the JSON plus the epilogue), 12 registers, 0 spills, 8 uniforms,
512 bytes. Mesa does not use scratch or a dynamic register index: the
`arr[i]`, `p[i]`, `col[i - 1]`, `v[j]` reads are expanded by NIR into
`ilt_pan u_i, const` compares and `bcsel_pan` chains (`ilt_pan %22.y, 2`, `1`, `3`
and so on), which become
`ICMP_OR.s32.lt.m1 rN, u4.w1, 0x3020100.b2, r0` (the constant is byte 2 of
`0x03020100`, i.e. 2; `0x1000000.b3` is 1) followed by
`CSEL.u32.eq rD, rN, 0x0, a, b`; `i + 1` is `IADD_IMM.i32 r4, u4.w1, #0x1`.
`m++`, `m *= m`, `4.0 - m`, `2.0 / m` fold to `FADD_IMM`, `FMA` and one `FRCP.f32
r4, u3.w0`. `u_c > 0.5` is `FCMP_OR.f32.lt.m1 ..., u4.w0` and the `?:` a `CSEL`.
The vertex stage is the same position shader as the other `a_pos` references
(24 instructions, 11 registers).

**Relation to 100 and 116.** All three read an `int` uniform; 100/116 use it as a
uniform-array / sampler index, 80 as a local-array and lane index, where Mesa
folds the index into compares on the FAU word. The lowering needed is the
`ICMP`/`CSEL`/`IADD_IMM` set of `docs/notes/mali-int-and-size.md` ("Integers")
plus accepting `GL_INT` in `uniform_of` / `uniform_extent` (section 4) as one
32-bit word, so 80 depends on both. Dynamic indexing of locals (`MAX_LOCALS`,
the `Ilt` + select chain) is the part with no existing row; it should follow
section 5's int-uniform step, not precede it.

**References.**

```
tools/mali/drm_shim_ref.sh run <work> $D/unitype_80.vert $D/unitype_80.frag $D/unitype_80
python3 -I tools/mali/pan_ref_diff.py extract $D/unitype_80.trace --draw > $D/unitype_80.expect.json
```

(`<work>`, `$D` as in section 6; no `--uniform-values`, `u_i` being an int; the
`.trace` is not kept.) `unitype_80.frag` is `tools/shaders/es100/matdyn.frag`
and `unitype_80.vert` is `tools/shaders/gl_test_es100/passthrough.vert`, both
verbatim.

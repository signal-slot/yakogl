<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the eight `deqp_vsuniarr2_*` references (vertex-shader uniform arrays, the `random` shapes)

**Status: study** (plan #402, item `note-vsuniarr2`; issue #20). HOST-ONLY: no
compiler change, nothing ran on the NanoPC-T6. The references are
`tools/mali/testdata/deqp_vsuniarr2_*` (plan item `ref-vsuniarr2b`, commit
`2bd8e747`), pinned by `tests/mali_deqp_vsuniarr2.rs`. They are the
`dEQP-GLES2.functional.uniform_api.random.*` programs that
`docs/notes/mali-deqp-vsuniarr.md` (the eight `deqp_vsuniarr_*`) left
undrawn (its section 2, row `random`). Every verdict below is the host
compile at the tip `22c5fc5f`, read from the test's pins; none is a render
verdict.

The refusal is the one of the first note: `MaliError::UniformArray`
(`src/shaderc/online/mali/mod.rs:430`, "a uniform array is not lowered
yet"), raised at `nir.rs:1103` in `uniform_of` for an array that
`array_element` (`nir.rs:1141`) does not take (it takes float, vec2, vec3
and vec4 elements only). So every int, bool and matrix array refuses, plain
or as a struct member.

## 1. The shape table (HOST-ONLY, from the test header)

19 `uniform_api.random.*` cases of the slices
`tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055` refuse with this text in
the vertex shader on the host, in five shapes of the measure (bool_array 8,
mat_array 5, int_array 3, array_in_struct 2, struct_array 1). Each case is one
program with a random mix of uniforms, compared to constants in the vertex
shader (and in four of the eight also in the fragment shader). Cases that
declare a sampler have no reference (`gl_ref.py --uniform-values` sets none).

| Reference | dEQP case (`dEQP-GLES2.functional.uniform_api.`) | Arrays and members read by the vertex shader | Read | Verdict at the tip |
|---|---|---|---|---|
| `mat3_part` | `random.10` | `mat3 u_var0[5]`, `ivec4 u_var1` (unused) | `u_var0[2]` alone | refused `UniformArray`, `#[ignore]`d |
| `struct_mat4_arr` | `random.15` | struct `{float m0; mat4 m1[3]}`, `float`, `ivec3` (unused), `float` | `m0`, `m1[1]` | refused `UniformArray`, `#[ignore]`d |
| `mat2_struct` | `random.17` | `mat2 u_var0[4]`, `mat3`, struct `{struct {ivec2 m0; bvec2 m1} m0}` | `u_var0[0]`, `[2]`, the `mat3`, `m0.m1` | refused `UniformArray`, `#[ignore]`d |
| `mat4_mixed` | `random.53` | `mat4 u_var0[3]`, `vec3`, two `int`, `bool` | every element and scalar | refused `UniformArray`, `#[ignore]`d |
| `struct_vec3_mat2` | `random.57` | `mat3`, struct `{.. vec3 m0[1] ..; mat2 m3[3]}` | the `mat3`, `m1.m0[0]`, `m3[0]`, `m3[2]` | refused `UniformArray`, `#[ignore]`d |
| `ivec3_part` | `random.66` | `ivec3 u_var0[2]` | `u_var0[1]` alone | refused `UniformArray`, `#[ignore]`d |
| `bvec3_part` | `random.67` | `vec3`, `bvec3 u_var1[5]` | the `vec3`, `u_var1[1]`, `[3]` | refused `UniformArray`, `#[ignore]`d |
| `struct_nested` | `random.74` | `float`, `bool[4]`, `float[4]`, struct array `{bool, vec3, bvec4[2], mat3[4], ivec3}` | `u_var0`, `m0[1].m0`, `m0[1].m3[2]`, `m0[1].m4` | refused `UniformArray`, `#[ignore]`d |

Matched: none. Differs: none. The eight byte-compares are `#[ignore]`d with
the refusal quoted; `refusals_pinned` passes and holds the text.
Not drawn: random.6, 24, 37, 61 (sampler) and random.71, 75, 76, 77, 83, 94,
99 (the same kinds of arrays and members, other sizes).

Mesa 26.2.1 (G610 drm-shim, `deqp_ref.sh --instrument shim`) makes all eight
with `vs.position` 24 words. `vs.varying` and `fs.fragment` word counts, and
the FAU words of `vs.varying` (`fau` in each `.expect.json`):

| Reference | `vs.varying` words | FAU words | `fs.fragment` words |
|---|--:|--:|--:|
| `mat3_part` | 23 | 22 | 10 |
| `struct_mat4_arr` | 50 | 32 | 10 |
| `mat2_struct` | 28 | 24 | 34 |
| `mat4_mixed` | 66 | 42 | 10 |
| `struct_vec3_mat2` | 56 | 34 | 10 |
| `ivec3_part` | 8 | 14 | 14 |
| `bvec3_part` | 19 | 22 | 24 |
| `struct_nested` | 35 | 26 | 36 |

## 2. How to read the FAU of these references

FAU word *n* is `u(n/2).w(n%2)` in the disassembly. Words 0 to 10 are the
viewport transform and the three varying offsets (`00000004 00000008
0000000c`); the uniforms start at word 11 (`u5.w1`). After the pushed
uniform words come the constants the compare folds in (a negated fp16 pair
such as `c60dc60d`) and the threshold `2a662a66` (fp16 0.05), then a zero.

`frame_uniarray_tests.rs` (`src/mali/frame_uniarray_tests.rs`, its header
and the "VS uniform arrays" section) gives the GL-to-word mapping the lowering
has to feed: a vertex shader's block is fp32 only (`halves` false), one row
a uniform (or a struct leaf) in declaration order, a bool leaf's nonzero
words become `0xffffffff`; element *e* of a `bvec4[N]` is at word 4*e*, of a
`mat4[N]` column *c* of matrix *k* at 16*k* + 4*c*. Two things in these eight
differ from the block order and the lowering must meet them:

1. **Mesa pushes only the words the shader reads.** Unread elements and
   unused uniforms (`// UNUSED`) take no FAU word, so a read element's
   first word is not at its block offset. `lower-vsuniarr` needs a push
   range per read element, as `deqp_vsuniarr_unused_*` already asked.
2. **The push order is not the declaration order** in four of the eight
   (details per reference). `gl_ref.py --uniform-values` sets no element of a
   plain array (it stays 0 in the dump), so a word of an array cannot be
   identified by its value; for the struct references it can (the value of
   leaf *k*, component *c* is 16(*k*+1)+*c*, a bool is `ffffffff` where *c* is
   even). Below, "by the dump" means such a value, "by the code" means the
   constant the compare expects.

**fp16 pairing of matrices.** A plain `mediump` float or matrix is fp16 in the
block (`frame_uniarray_tests.rs`: each element rounded up to whole dwords:
`vec3` 2 dwords, `mat2` 2, `mat3` 6, `mat4` 8). In the code: a mat2 or
mat4 column of 2 or 4 halves is a word pair (`.h00`/`.h11`) or two words; a
mat3 column is one full word plus a word whose `.h00` is the third value
(`50205000`, `00005040`: 32, 33 and 34 then pad). A matrix that is a **struct
member** stays fp32 (one word an element) and is narrowed in the shader by
`V2F32_TO_V2F16` of two words, with an unpaired word read through
`FADD.f32 r.h0, uN.wK, 0x0.neg` and joined by `MKVEC.v2i16`. A bool and an int
are one 32-bit word.

## 3. The eight references

### `mat3_part` (random.10)

Mesa pushes 6 words, 11 to 16 (`u5.w1` to `u8.w0`), for `u_var0[2]` alone: the
fp16 mat3 of 3 columns x (1 pair word + 1 word read as `.h00`). `u_var0[0]`,
`[1]`, `[3]`, `[4]` and `u_var1` have no word. Constants at 17 to 19, three
pairs `c60dc60d c47bc47b c853c853` (the negated 4.47... of the compare, folded
with `FADD_IMM.v2f16` / `FADD.v2f16`), threshold at 20.
Relation: `deqp_unif2_arr_mat_array` (the first note's `random.10` link) is
this shape's whole-array form; here only element 2 is read, the
`unused_mat2` form with an fp16 plain array. Lowering: row of 6 words per
mat3 element at the storage dword of element 2 (12 dwords in), a push range
of only that element, the existing `compare_mat3` fp16 tree (`FMAX.v2f16`
of the `abs` of each `FADD` result, one `FCMP_OR.v2f16.lt` with
`2a662a66`); the constants are the negated expected values rather than a
`compare` of a loaded value, so `lower-vsuniarr` needs the `FADD_IMM`
pre-subtract Mesa chooses for a constant matrix (as `deqp_unitype_*` mat
references show). Verdict: refused, pinned.

### `struct_mat4_arr` (random.15)

All members are struct or scalar leaves; every word is fp32 except the two
plain `mediump float` (`u_var1`, `u_var3`), which are fp16 words 25
(`00005500`) and 26 (`00005700`). The struct: `m0` (16.0) is the lone word 27
(`41800000`); `m1[1]` is 16 words, by the dump 48 to 63: elements 1 to 14 at
words 11 to 24 (49.0 to 62.0, pairs through `V2F32_TO_V2F16`), element 0 at
28 (`42400000`) and element 15 at 29 (`427c0000`), each of the two read alone
through `FADD.f32 .h0 ..neg`. `m1[0]`, `m1[2]` and `u_var2` have no word.
Relation: the `struct_mat` shape of `deqp_vsuniarr` (80 words, fp32
members in order); here the mat4 element is split by use (paired words low,
unpaired high). Lowering: the pair/single split follows the use order of the
comparison, so the lowering's allocation of FAU slots must follow Mesa's
(pair words first, singles after the fp16 and bool words); this is the
first reference where a leaf's words are not contiguous. Verdict: refused,
pinned.

### `mat2_struct` (random.17)

Pushed in this order: words 11 and 12, the bools of `u_var2.m0.m1` (by the
dump `ffffffff`, `00000000`; declared last, pushed first); 13 and 14, then 15
and 16: `u_var0[0]` and `u_var0[2]` as fp16, two words each (one per column,
`.h00`/`.h11`); 17 to 22, `u_var1` mat3 fp16 as 3 x (pair, `.h00`) (by the
dump `50205000 00005040 ...`, 32 to 40 with pads). Threshold at 23.
Relation: the fp16 mat2 of `deqp_unitype_vsapi_mat2` repeated per element,
with `deqp_vsuniarr_unused_bvec4`'s order surprise (the struct's bools ahead
of the arrays). Lowering: per read element a 2-word fp16 row; the `bvec2`
compare is `LSHIFT_OR ... .not` / `LSHIFT_AND` over its two words; the push
order must put the struct's leaf first. Both mat2 compares share the one
threshold. Verdict: refused, pinned. (The fragment shader reads
the same `mat2`/`bvec2` values; its 34 words are its own refusal,
`fsint`-family, once the vertex shader compiles.)

### `mat4_mixed` (random.53)

Declaration order here. Words 11 to 34: `u_var0[0..2]` as fp16 mat4, 8 words
each (4 columns x pair), `abs.h00`/`abs.h11` alternating in the code (so the
word pairs are `h00`,`h11` then `h11`,`h00` per column pair, which is what
Mesa's folding of the two halves produces). 35 and 36: `u_var1` vec3 fp16 (by
the dump `50205000 00005040`). 37 and 38: the two ints (48, 64). 39: the bool
(`ffffffff`). 40: threshold. FAU 42 words.
Relation: the `mat4` shape of `deqp_vsuniarr` with all three elements read,
plus the scalars of `deqp_unitype_*`. Lowering: the plainest of the eight,
and the first to try: a mat4 array of three elements as one 24-word fp16 row,
the 48-`abs` `FMAX.v2f16` tree grouped per column pair, `ICMP_OR.u32.eq.m1`
for the ints, `LSHIFT_OR ... .not` for the bool (as `deqp_vsuniarr_bvec4`).
Verdict: refused, pinned.

### `struct_vec3_mat2` (random.57)

All struct leaves are fp32, the plain `mat3` is fp16. Words: 11 and 12
(49.0, 50.0) and 23 (48.0) are `m1.m0[0]` (vec3: two words as a pair, the
third alone); 13 and 14 with 24 and 25 are `m3[0]` (by the dump 81, 82 at 13,
14; 80 at 24; 83 at 25); 15 and 16 with 26 and 27 are `m3[2]` (113, 114 at
15, 16; 112 at 26; 115 at 27). 17 to 22: the plain `mat3` fp16 (the value
pattern `4c404c00 00004c80 ...`). 28 to 31: four fp16 constant pairs
(`c849c849 48fa48fa 45f145f1 40ec40ec`); 32 threshold. The rule is the same
as in `struct_mat4_arr`: an fp32 word that `V2F32_TO_V2F16` takes as one of a
pair is low (11 to 16), one read alone (`FADD.f32 .h0 ..neg`) is high (23 to
27). Relation: `deqp_vsuniarr_struct_mat` (struct mat members, fp32). The
`FADD_IMM.v2f16` of a pair against its negated constant is the compare of a
`vec3` or `mat2` column with a literal. Lowering: as `struct_mat4_arr`,
plus the two-dimensional narrowing (`MKVEC.v2i16` of an unpaired `.h0`).
Verdict: refused, pinned.

### `ivec3_part` (random.66)

Smallest. `u_var0[1]` is three int words, 11 to 13; FAU 14 words and no
constant (the expected value is 0). The compare is a max: `CSEL.u32.gt` twice
(the unsigned max of the three words, via `MOV r0,u5.w1` and `u6.w0`,
`u6.w1`), `ICMP_OR.u32.eq.m1` against 0, then `CSEL.u32.eq` to 1.0. The
fragment shader holds the same array and reads it likewise (14 words; its
FAU holds `3c003c00` at word 7, fp16 1.0, and three words 0).
Relation: `ivec4` of `deqp_vsuniarr` with one lane fewer and the
all-zero special case that Mesa folds to a max (the `ICMP_OR`/`ICMP_AND`
per-lane form of the `ivec4` reference does not occur). Lowering: ivec3 is
3 words an element (GL storage 3 dwords each, so element 1 at dword 3); a
push range of 3 words, and the compare-to-zero folding to a `CSEL.u32.gt`
max chain; `deqp_fsint_*` already holds the fragment-side form. Verdict:
refused, pinned.

### `bvec3_part` (random.67)

Words 11 to 20. 13 and 14 are the plain `vec3 u_var0` (fp16, by the dump
`4c404c00 00004c80`: 16 to 18); 19 the negated constants pair `c8e5c8e5`; 20
threshold. The other six, 11, 12 and 15 to 18, are the bool words of
`u_var1[1]` (expected true, true, true) and `u_var1[3]` (true, false, true):
three each, `LSHIFT_AND.i32` over the true lanes and `.not` of the false
lane (word 17 by the code), exactly the folding of `deqp_vsuniarr_bvec4`.
Which of 11, 12, 15 to 18 holds which lane the dump cannot say (array
words are 0); by the code the bools of `[1]` are 11, 12, 15 and those of
`[3]` 16, 17, 18, so the vec3 (declared first) sits in the middle of them.
Relation: `bvec4` of `deqp_vsuniarr`, one lane fewer and in two elements.
Lowering: two read elements of a bvec3 array = two separate 3-word push
ranges; the order is Mesa's (the vec3 inside), to be matched by trying the
allocation rule first (a fp16 vec3 is split by the compare's use of
`FADD_IMM` and gets words 13 and 14), and re-deriving it from the
disassembly if byte-equality fails. Verdict: refused, pinned.

### `struct_nested` (random.74)

Only four leaves are read; all else is `// UNUSED`. Words: 11 to 18 (304.0 to
311.0 by the dump) and 23 (312.0) are `m0[1].m3[2]`, a mat3 of nine fp32
words (eight paired, one single); 19 and 20 (`00000151`, `00000152`) and 24
(`00000150`) are `m0[1].m4`, an ivec3 (lane 0 alone, at 24); 21 (`00004c00`,
fp16 16.0) is the plain float `u_var0`; 22 (`ffffffff`) the bool
`m0[1].m0`; 25 threshold. Relation: `nested_mat` of `deqp_vsuniarr` (a
struct array in a struct, fp32 depth first). The read of `m0[1]` of a
struct array whose element 0 is unused puts the first word of the row at
the second struct's offset: the block row offset of `u_var3.m0[1].m3[2]`
is the end of `m0[0]` plus the leaves before it in element 1.
Lowering: the struct array's element stride (the sum of its leaves in
words, with each leaf's own alignment) and the fp32-narrowing machinery of
`struct_mat4_arr`; the `ivec3` compare is the `CSEL.u32.gt` max chain of
`ivec3_part` with a non-zero expected word for lane 0. The fragment shader
(36 words) reads four more bools and ints. Verdict: refused, pinned.

## 4. How the eight relate to `deqp_vsuniarr` and what `lower-vsuniarr` does

- **Covered by the first note's design** (`mali-deqp-vsuniarr.md` section 5,
  `array_element` extended past float to int, bool and matrix elements):
  `mat4_mixed` (the plain mat4 array), `mat3_part`, `mat2_struct`'s plain
  arrays, `ivec3_part`, `bvec3_part`'s array.
- **New relative to it:**
  1. A matrix array whose element is **not a power-of-two stride**: a mat3
     element is 6 fp16 dwords, a vec3 2, an ivec3 3 or a bvec3 3 words.
     `uniform_of`'s `array_len > 1` path must store the per-element dword
     count from the type, not 4 or 16.
  2. **Reads of single elements** (`mat3_part`, `ivec3_part`,
     `mat2_struct`, `bvec3_part`, `struct_mat4_arr`, `struct_vec3_mat2`,
     `struct_nested`): the push range is per element read, not the whole
     array. This is what `unused_bvec4` and `unused_mat2` of the first
     note already showed; here it also holds for plain arrays.
  3. **The push order**, which is not the declaration order in
     `mat2_struct` (struct bools first), `bvec3_part` (a vec3 in between),
     `struct_mat4_arr`, `struct_vec3_mat2` and `struct_nested` (fp32 words
     taken as a pair low, unpaired high). The `lower-vsuniarr` item must
     either reproduce Mesa's slot order or take the four as
     "differs, pinned"; the first is the only way to be byte-equal.
  4. **Struct members that are fp32 arrays of matrices or vectors**
     (`struct_mat4_arr` `m1[1]`, `struct_vec3_mat2` `m3[]`): the narrowing
     (`V2F32_TO_V2F16`, `MKVEC.v2i16`, `FADD.f32 .h0 ..neg`) and the fp32
     row, not the fp16 row of a plain array.
- **After the vertex shader compiles**, four of the eight (`ivec3_part`,
  `bvec3_part`, `mat2_struct`, `struct_nested`) stop at the fragment shader's
  own refusal (its int/bool or `unitype` family), as the half `_both` of the
  first note; the vertex-shader byte-compare is separate (the test compares
  both shaders through `mali_ref::assert_matches`, so those four need
  `lower-fsint` as well).
- **Order of work:** `mat4_mixed` (declaration order, no struct) first;
  `mat3_part`, `ivec3_part` next (one element, no struct); `bvec3_part`,
  `mat2_struct` (push order); the three struct references last
  (`struct_mat4_arr`, `struct_vec3_mat2`, `struct_nested`).

## 5. The test

`cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsuniarr2`:
one test runs (`refusals_pinned`), eight are `#[ignore]`d with the refusal.
A lowering that takes a reference removes its `#[ignore]` and its `REFUSED`
pin and adds the name to `MATCHED`.

## 6. Caveats

- The word-to-lane assignment of `bvec3_part` and the per-element offsets of
  the plain arrays are read from the code, not the dump: an array element is
  0 in the FAU dump. A lowering that ends byte-different on the push order
  should first check these.
- `refusals_pinned` was not re-run for this note; the verdicts are the
  pins in `tests/mali_deqp_vsuniarr2.rs` at the tip.

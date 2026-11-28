<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P dEQP-GLES3 `vertex_arrays`: the 41 PowerVR fails (cause VA)

This note gives the cause of the 41 `Fail` lines under
`dEQP-GLES3.functional.vertex_arrays.*` in `docs/es3/am62p-gles3-notpass.tsv`.
`tools/cts/am62p-notpass-causes.py` files them under rule `VA`, status `open`.
This is cause analysis only: no code changes. Everything measured here is
HOST-ONLY. No board was used.

**Result.** Both groups are GL-core defects, and both were already fixed by
the GL-core commit `6c9606f6` ("gl: convert packed and half-float buffer
arrays, integer w fill for narrow ivec4 inputs", 2026-10-14). The PowerVR
vertex fetch is not at fault. The 41 lines are stale. They come from the
`ad2b5ed` build (2026-10-13/14): the TSV header says rows 22-45 are `build
ad2b5ed`, and `vertex_arrays` is row 45 of `tools/cts/am62p-gles3-tree.txt`.
That build predates the fix. `6c9606f6` is not an object in this repository:
it reached main inside the 0.1.0 squash `381b0f09` (2026-10-27). Its hash,
its date and the `ad2b5ed` line numbers below come from
`docs/notes/es3-draw-random.md`, which replayed the same check on `ad2b5ed`.
That note studied the same two classes in `draw.random` (its classes B and
C). At the integration tip `ad2b6cc9`, the GL core gives the PowerVR-shaped
back end the right rows for all 44 dEQP shapes of these groups (section 3).
**No code fix is left.** The only open step is a board rerun of row 45 at a
build that has the fix.

dEQP source: VK-GL-CTS `93bca01` (the revision `tools/cts/README.md` and
RESULTS.md use), `modules/gles3/functional/es3fVertexArrayTest.cpp` (the
case lists) and `modules/glshared/glsVertexArrayTests.cpp` (the buffer
sizes, shaders and draw). Lines drift, so each place is also named by its
function.

## 1. The groups

`vertex_arrays` has 1,005 mustpass cases (`single_attribute` 802,
`multiple_attributes` 203). Only `single_attribute.{first, offset, strides,
output_types}` fail.

| Group | Type | Comps -> input | Storage | Stride / offset | Fail / mustpass | Message |
|---|---|---|---|---|---:|---|
| A1 `first.int2_10_10_10.first{6,24}_offset16_stride{8,32}_quads{5,256}` | `GL_INT_2_10_10_10_REV`, not normalized | 4 -> `vec2` | buffer | stride 8, 32; offset 16; first 6, 24 | 8 / 8 | `glDrawArrays(): glGetError() returned GL_INVALID_OPERATION at glsVertexArrayTests.cpp:1148` |
| A2 `offset.int2_10_10_10.offset{4,32}_stride{4,32}_quads{1,256}` | same | 4 -> `vec2` | buffer | stride 4, 32; offset 4, 32 | 8 / 8 | same |
| A3 `strides.int2_10_10_10.buffer_stride{16,32}_components4_quads{1,256}` | same | 4 -> `vec4` | buffer | stride 16, 32; offset 0 | 4 / 4 | same |
| (A, passing) `strides.int2_10_10_10.user_ptr_stride{16,17,32}_*`, `normalize.{int,usigned_int}2_10_10_10.*`, `output_types.{int,usigned_int}2_10_10_10.*` | packed, signed and unsigned | 4 -> `vec4` and narrower | client | any | 0 / 14 | Pass |
| B1 `output_types.{byte,short,int}.components{2,3}_ivec4_quads{1,256}` | `BYTE`, `SHORT`, `INT`, `glVertexAttribIPointer` | 2, 3 -> `ivec4` | client | 0 (tight) | 12 / 12 | `Image comparison failed.` |
| B2 `output_types.unsigned_{byte,short,int}.components{2,3}_uvec4_quads{1,256}` | `UNSIGNED_BYTE`, `UNSIGNED_SHORT`, `UNSIGNED_INT`, `glVertexAttribIPointer` | 2, 3 -> `uvec4` | client | 0 (tight) | 9 / 12 | `Image comparison failed.` |
| **total** | | | | | **41** (A 20, B 21) | |

What the table shows:

- **Group A is every packed case in a buffer, and only those.** All 20 fail.
  The 14 packed cases with a client array pass, whether signed or unsigned,
  normalized or not, at any stride (17 included). The unsigned type never
  appears among the failures only because dEQP's `first`, `offset` and
  `strides` lists use `INPUTTYPE_INT_2_10_10_10` alone
  (`SingleVertexArray{First,Offset}Tests::init`,
  `SingleVertexArrayStrideTests::init`). There is no sign-dependent defect.
  The failure is a GL error raised before anything is drawn, so neither
  normalization nor the decoded value is involved.
- **Group B is every integer `output_types` case whose array is narrower
  than its 4-component integer input.** The missing `w` matters. The
  `components{2,3}_ivec{2,3}` and `components4_*vec4` cases pass, and so do
  all float, half and fixed cases into `vec4`. The 3 that passed are
  `unsigned_{byte,short,int}.components2_uvec4_quads1`. Their picture was
  not decoded (the wrong `w` probably moves that one-quad picture to where
  the reference has no coverage either). That does not change the cause:
  the same `w` is wrong in them.
- Group A has no half-float, unaligned or normalized subgroup. dEQP drops
  the unaligned buffer strides and offsets (`if (aligned)`), and the
  `normalize` group uses client arrays.

## 2. The cause

### Group A: the in-place range check sized a packed element as floats

ES 3.0 2.8: an element of `GL_[UNSIGNED_]INT_2_10_10_10_REV` is one 32-bit
word, and its size must be 4. dEQP makes the buffer exactly as long as the
draw reads (`MultiVertexArrayTest::iterate`):
`bufferSize = offset + stride * (quads * 6 - 1) + componentCount *
inputTypeSize`. Here `inputTypeSize(INT_2_10_10_10)` is `sizeof(uint32_t) / 4`
= 1, so the last element is 4 bytes and ends exactly at the end of the
buffer.

At `ad2b5ed`, `map_attrib_format` mapped the packed types to
`GLES_ATTRIBF_FLOAT32`. The in-place filter in `build_common` was only
`attrib_format_is_native(src_fmt) && comps >= sa.components`, so a packed
buffer array took the in-place branch. That branch's range check sized one
element as the input's components times `attrib_type_size`. That is 8 bytes
for the `vec2` input of A1 and A2, and 16 bytes for the `vec4` input of A3,
instead of 4. The last vertex then ran 4 or 12 bytes past the buffer, and
the branch returned `GL_INVALID_OPERATION` ("the array runs past the VBO";
`ad2b5ed:src/gl/draw.rs:3566`, as `es3-draw-random.md` class C showed with
a marker). That is the error dEQP reports at `glsVertexArrayTests.cpp:1148`,
the `GLU_EXPECT_NO_ERROR` after `drawArrays` in `ContextArrayPack::render`.
A client array never reached that check: `convert_attrib` has always sized a
packed element as one word (`attrib_elem_bytes`), which is why every
client-array packed case passed. Had the check passed, the GPU would have
read packed words as float32 (`es3-draw-random.md` class A).

### Group B: the missing `w` of an integer array was the float 1.0

ES 3.0 2.8: an array with fewer components than the shader input reads the
missing ones as (0, 0, 0, 1), and for an integer input that 1 is the integer
1. dEQP's vertex shader for an `ivec4` / `uvec4` input is
`coord = vec2(a_0.xy); coord += vec2(a_0.zw);`
(`ContextShaderProgram::genVertexSource` in `glsVertexArrayTests.cpp`, the
`OUTPUTTYPE_IVEC4` / `UVEC4` case). These arrays are client arrays
(`STORAGE_USER`), so every one went through `convert_attrib` /
`attrib_row`. At `ad2b5ed`, `attrib_row` filled `w` with
`1.0f32.to_bits()` = `0x3f800000` = 1,065,353,216 for every array. The
shader added that to `coord.y`, which put the quad far outside the viewport,
while the reference renderer (`sglr` with `w` = 1) draws it on screen.

### The PowerVR vertex fetch is not involved

- `yakogl_backend_pvr.native_attrib_formats` is 0 (`src/pvr/backend.rs`,
  the `YakoglBackend` static, now line 11503): the back end claims no format
  other than float32, so the GL core converts every other array.
- The draw path refuses any attribute whose `format` is not 0
  (`src/pvr/backend.rs` line 9252, "only float32 vec1..4 attributes are
  supported").
- `add_stream` (line 11576) turns each attribute into a PDS vertex DMA of
  `components * 4` bytes at `stride`, with no format conversion. The vertex
  shader receives the dwords exactly as the GL core staged them, so the
  integer input sees the staged integer bits. Mesa's PowerVR driver does its
  format conversion in the vertex shader (`vs.attrib_formats`), which YakoGL
  does not use. This is why the GL core must hand over finished rows.

So both defects were in the GL front end's array staging, and both were
fixed there.

## 3. Host evidence at the tip (`ad2b6cc9`, HOST-ONLY)

Feature set `--no-default-features --features
host-test,gpu-powervr,shaderc-online`.

- `tests/pvr_vertex_attrib_formats.rs`: 6 / 6 pass. Among them,
  `powervr_table_has_no_native_attrib_formats` asserts that the real PowerVR
  table claims no native format, so the converted path is the one the board
  takes. `packed_types_convert_to_float` and
  `integer_entry_point_keeps_integers` also pass.
- `tests/es3_vertex_arrays.rs` (the fix's own tests): 10 / 10 pass. Among
  them are `packed_array_in_a_buffer_is_converted_from_first`,
  `..._honours_the_offset`, `..._steps_by_the_stride`,
  `packed_array_filling_the_buffer_is_not_refused`,
  `narrow_integer_arrays_fill_w_with_integer_one` and
  `narrow_signed_integer_array_sign_extends`.
- A throwaway scratch test (not committed) replayed the exact dEQP shape of
  each case. It was built on the stand-in back end of
  `tests/es3_vertex_arrays.rs`, which uses the PowerVR table's
  `native_attrib_formats` 0 and records the rows the GL core hands the back
  end.
  - Group A, all 20 names: a `GL_INT_2_10_10_10_REV` buffer of exactly
    dEQP's `bufferSize`, with the name's first, offset and stride and with
    `quads * 6` vertices, drawn by `glDrawArrays(first, quads * 6 - first)`
    into a `vec2` or `vec4` input. Every draw gives `GL_NO_ERROR`. Each is
    converted, not fetched in place: float32 rows of 2 or 4 components,
    every vertex equal to the ES 3.0 2.1.6.1 decode of its word. For
    example, `offset4_stride4_quads256` gives 1,536 vertices and 3,072
    dwords, and `buffer_stride32_components4_quads256` gives 6,144 dwords.
  - Group B, all 24 shapes (the 21 fails and the 3 passes): a client array
    of `quads * 6` vertices of 2 or 3 components, `glVertexAttribIPointer`
    with stride 0, into `ivec4` / `uvec4`. The data is sign-mixed for the
    signed types. Every draw gives `GL_NO_ERROR`, rows of 4 dwords, the
    components as integers (sign-extended), z 0 and **w the integer 1**
    (`0x00000001`, not `0x3f800000`).
  - Both scratch tests passed (2 / 2).

  The failing case at `ad2b5ed` follows from the arithmetic in section 2:
  the old element size of 8 or 16 bytes against a 4-byte last element. This
  commit cannot rerun that code, because `ad2b5ed` is not in the squashed
  history. `es3-draw-random.md` section 3 did rerun it, for case 118, which
  has the same check and the same shape.
- Second GPU: the Pi 4 (V3D 4.2) whole tree has no `vertex_arrays` line in
  `docs/es3/pi4-gles3-fails.tsv`. Its images `VA1` and `VA2` ran at
  `b1b5b845` (2026-11), a build that has the fix. The V3D also claims only
  float32, so it takes the same GL-core staging.

Host dEQP (`tools/cts/host-deqp-run.sh`) was not run. Its counting list
(`docs/es3/host-gles3-notpass.tsv`, `26d1c4a2`) already has no
`vertex_arrays` line on `GPU=powervr`, so group A raises no GL error on the
host. Group B is a picture verdict, which the host does not give.

## 4. Code locations (tip `ad2b6cc9`)

| What | Where |
|---|---|
| A packed or half-float array, or `GL_INT` / `GL_UNSIGNED_INT` read through `glVertexAttribPointer`, is never fetched in place | `src/gl/draw.rs` `attrib_in_place_layout` (line 1193), used by the in-place filter in `build_common` (line 4770) |
| One packed element is 4 bytes | `src/gl/draw.rs` `attrib_elem_bytes` (line 1206); the converted branch's range check (line 4848-4851) uses it |
| The in-place range check that raised group A's error (now reached only by the layouts the fetch reads) | `src/gl/draw.rs` `build_common`, the `array_end(elem)` match (lines 4790-4805) |
| The integer 1 as `w` | `src/gl/draw.rs` `attrib_row` (line 1227, the `c == 3` arm) |
| Conversion to staged rows | `src/gl/draw.rs` `convert_attrib` (line 1270) |
| PowerVR: no native formats; float32-only refusal; PDS DMA streams | `src/pvr/backend.rs` lines 11503, 9252, `add_stream` 11576 |

## 5. Fix

**Nothing more to change in code.** The fix is `6c9606f6`, in `src/gl/draw.rs`
(GL core), on main since `381b0f09`, with its host tests
`tests/es3_vertex_arrays.rs` and `tests/pvr_vertex_attrib_formats.rs`. No
change in `src/pvr/**` or `driver/` is needed.

A native PowerVR attribute path is not a fix for these cases. It would mean
claiming formats in `native_attrib_formats` and converting in the vertex
shader the way Mesa's `vs.attrib_formats` does. It would only save the
CPU-side conversion, and if done later, `tests/pvr_vertex_attrib_formats.rs`
fails first, by design.

What is left is a board measurement: row 45 (`dEQP-GLES3.functional.vertex_arrays.*,
dEQP-GLES3.info.*`) run on the AM62P at a build that has the fix. The
expected result is 0 `vertex_arrays` Fail. Until a board record shows that,
these 41 lines stay in the TSV with this cause.

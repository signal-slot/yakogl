<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 `draw.random`: the 14 AM62P failures

Cause analysis, no code. On the AM62P whole tree at `ad2b5ed`
(`docs/es3/am62p-gles3-notpass.tsv`), `dEQP-GLES3.functional.draw.random.`
{19, 36, 42, 71, 93, 94, 151, 167, 186, 202, 204, 207, 208} fail "Image
comparison failed". `draw.random.118` fails with "glDrawElementsInstanced():
glGetError() returned GL_INVALID_OPERATION at glsDrawTest.cpp:2378". This
note decodes each case, sorts the 14 into three cause classes and gives the
fix and host test for each. Reference: roadmap stage 4, plan #343.

**Result: one GL-core change already fixed all three classes:** `6c9606f6`
"gl: convert packed and half-float buffer arrays, integer w fill for narrow
ivec4 inputs" (2026-10-14; it reached main in the 0.1.0 squash `381b0f09`).
`ad2b5ed` (2026-10-13) predates it. The fix's host tests are
`tests/es3_vertex_arrays.rs` (the `tools/check.sh` row
`host-test,gpu-powervr,shaderc-online`). The Pi 4 confirms it on a second
GPU. The same 14 names failed there on `cd738b1a` (`24fbdcd6`, image D1),
and none fail on `925dbca9`, which has the fix (`96810ab2`:
`draw.random 14 -> 0`; `docs/es3/pi4-gles3-fails.tsv` has no `draw.` row).
So no code item is left for these 14. What remains is an AM62P board rerun of
`draw.random` (6) and one suspected GC7000UL defect outside the 14 (5).

Lines drift, so each place is also named by its function. Commit-pinned lines
(`ad2b5ed:src/gl/draw.rs:3566`) are exact. Mesa is 26.2.1 (the deps cache's
`mesa-pan/mesa-26.2.1`), described in words, not copied. Nothing here was
measured on a board: the decode and the host replays below are HOST-ONLY.

## 1. Decoding the cases

`es3fDrawTests.cpp` `RandomGroup::init` builds case `N` from
`de::Random(0xc551393 + ndx)`, for `ndx` 0..299. It drops specs that are not
valid or not meaningful and duplicate hashes. Names count every inserted
hash, including the cases it then leaves out (unaligned offset or stride, the
disallowed hash 544). The decoding replays that loop: `deRandom`'s xorshift128,
`chooseWeighted` in float32, `DrawTestSpec::valid`, `AttributeSpec::valid`,
the two hashes, `isCompatibilityTest`. It used the VK-GL-CTS sources (main
branch; the draw test generator is unchanged against the `93bca01` the board
images use). Check: the replay's left-out numbers {1, 3, 4, 6, 8, ... 199,
201, 203} are exactly the gaps of the 93bca01-era `gles3-main-*.txt`
mustpass list (0..210). The one exception is 50, which the replay generates
and the mustpass omits. 211 and 212 are left out (unaligned).

Every mustpass `draw.random` case has **one** attribute. The first
`chooseWeighted` of consecutive seeds always lands on `attribCounts[0]`.
Attribute 0 is the position. The vertex shader adds its components into
`coord` (`.xy`, `+.z` to x for a 3-vector, `+.zw` for a 4-vector), and
`gl_Position = vec4(u_coordScale * coord, 1, 1)`. `u_coordScale` is
`1 / (max * 1.1)` (doubled for 3- and 4-vectors), with `max` the input
type's range (`2^31 - 1` for `int`, `2^32 - 1` for `uint`, 255 for `ubyte`,
...). A float-from-int that reaches the shader as the raw bits is a
coordinate far out of range. So is an integer `w` that reads as `0x3f800000`.
Both put the primitive in the wrong place.

How dEQP declares the array (`AttributeArray::bindAttribute`): a non-float
input with a float shader type goes through `glVertexAttribPointer`. An
integer shader type goes through `glVertexAttribIPointer`. `user` storage is
a client array; `buffer` storage is a buffer object.

| case | draw call, primitive | first / indices / range / instances | attribute 0 (input xN -> shader type, storage, offset, stride, divisor) | class |
|---|---|---|---|---|
| 19 | DrawArraysInstanced, triangle_fan x5 | first 7, 1 instance | `GL_INT` x2 -> `vec2`, buffer, 0, 0, 0 | A |
| 36 | DrawRangeElements, line_strip x1 | ubyte indices, client, offset 7, range [1,8] | `GL_UNSIGNED_SHORT` x3 -> `uvec4` (IPointer), buffer, 0, 0, 0 | B |
| 42 | DrawArraysInstanced, triangles x64 | first 7, 1 instance | `GL_UNSIGNED_INT` x2 -> `uvec4` (IPointer), client, 0, 16, 0 | B |
| 71 | DrawRangeElements, lines x1 | uint indices, buffer, offset 0, range [1,128] | `GL_INT` x1 -> `float`, buffer, 0, 0, 0 | A |
| 93 | DrawRangeElements, lines x1 | ushort indices, client, offset 13, range [3,8] | `GL_UNSIGNED_INT` x3 -> `vec3`, buffer, 0, 0, 0 | A |
| 94 | DrawRangeElements, lines x1 | ushort indices, client, offset 7, range [3,257] | `GL_INT` x4 -> `vec4`, buffer, 0, 16, 0 | A |
| 118 | DrawElementsInstanced, line_strip x1 | ushort indices, buffer, offset 0, 2 instances | `GL_UNSIGNED_INT_2_10_10_10_REV` x4 normalized -> `vec2`, buffer, 0, 0, 0 | C |
| 151 | DrawArraysInstanced, triangles x1 | first 7, 1 instance | `GL_UNSIGNED_BYTE` x2 -> `uvec4` (IPointer), buffer, 0, 16, 0 | B |
| 167 | DrawElements, lines x1 | ubyte indices, client, offset 7 | `GL_UNSIGNED_INT` x4 -> `vec4`, buffer, 12, 0, 0 | A |
| 186 | DrawArrays, points x1 | first 7 | `GL_INT` x1 -> `float`, buffer, 0, 0, divisor 1 | A |
| 202 | DrawElements, triangle_strip x5 | ubyte indices, client, offset 0 | `GL_UNSIGNED_INT` x3 -> `vec2`, buffer, 0, 0, 0 | A |
| 204 | DrawArrays, points x5 | first 0 | `GL_UNSIGNED_INT` x2 -> `uvec4` (IPointer), client, 0, 0, divisor 129 | B |
| 207 | DrawElements, lines x1 | uint indices, buffer, offset 0 | `GL_INT` x3 -> `ivec4` (IPointer), buffer, 0, 0, 0 | B |
| 208 | DrawArrays, points x5 | first 0 | `GL_SHORT` x3 -> `ivec4` (IPointer), client, 1, 7, 0 | B |

No case uses base vertex; ES 3.0 has none. The (seed index) of each case:
19 = 27, 36 = 51, 42 = 60, 71 = 100, 93 = 133, 94 = 134, 118 = 167,
151 = 213, 167 = 238, 186 = 267, 202 = 286, 204 = 289, 207 = 293, 208 = 294.

The classes below are not fitted to the 14. Applied to all 151 mustpass cases
with the `ad2b5ed` rules, they predict exactly these 14 to fail, with no
misses and no extra. The closest passing cases agree. Case 37 (`GL_UNSIGNED_INT` x2 -> `vec4`, buffer)
passed because an array narrower than its input was already converted. The
client-array `GL_INT`/`GL_UNSIGNED_INT` cases (5, 105, 115, 126, 148, 171)
passed for the same reason. The integer arrays that pass feed `int`, `uint`
or a 2- or 3-vector, which have no `w`; no mustpass case feeds a full-width
array to an `ivec4`/`uvec4`. Case 138 is a client 2_10_10_10 array, and it
passed.

## 2. The cause classes

The GL core decides per attribute in `build_common` (`src/gl/draw.rs`, the
"attributes" loop). An array in a buffer object is fetched **in place** when
the back end reads its format. Otherwise `convert_attrib` writes float32 (or
integer) rows into a scratch buffer. `map_attrib_format` maps every type
without its own `GLES_ATTRIBF_*` (`GL_INT`, `GL_UNSIGNED_INT`, the packed
types, `GL_HALF_FLOAT`) to `GLES_ATTRIBF_FLOAT32`. The PowerVR back end
claims no other format (`native_attrib_formats: 0`), and the V3D claims only
float32. Mesa shows why the back end cannot convert later. The PowerVR's PDS
only DMAs bytes; Mesa compiles the format conversion into the vertex shader
from `vs.attrib_formats` (`pvr_arch_pipeline.c`). The V3D's attribute record
carries type, sign, normalize and `read_as_int_uint` (`v3d_vertex_state_create`),
and YakoGL's V3D does not use them. So the GL core must hand both of them
rows that are already in their final form.

### Class A: `glVertexAttribPointer` with `GL_INT` / `GL_UNSIGNED_INT` in a buffer (7: 19, 71, 93, 94, 167, 186, 202)

ES 3.0 2.8 (vertex arrays): integer data that `VertexAttribPointer` sets is
converted to floating point (normalized or not). At `ad2b5ed` the in-place
test was only `attrib_format_is_native(src_fmt) && comps >= sa.components`.
`GL_INT` maps to `FLOAT32`, so a full-width int array in a buffer went to the
GPU as is. The shader read the integer bits as floats: values from ±2^31
become denormals or huge numbers, never `coord`. A client array, or one
narrower than the input, went through `convert_attrib`, and `attrib_component`
converted it correctly.

**Fix (in `6c9606f6`):** `attrib_in_place_layout` (`src/gl/draw.rs`, today
line 1110) keeps `GL_INT` / `GL_UNSIGNED_INT` in place only when `ga.integer`
is set (`glVertexAttribIPointer`). The in-place filter (`build_common`, today
line 4078) adds it to the test.
**Host test:** `int_array_through_the_float_entry_point_is_converted`
(`tests/es3_vertex_arrays.rs`): a stand-in back end records the rows the GL
core hands it, and the test expects `-3.0, 7.0, ...` instead of the raw
words.

### Class B: an integer array narrower than its `ivec4` / `uvec4` input (6: 36, 42, 151, 204, 207, 208)

ES 3.0 2.8: an array with fewer components than the shader input reads the
missing ones as (0, 0, 0, 1). For an integer input that `1` is the integer 1.
Every such array goes through `convert_attrib`, whatever its storage. At
`ad2b5ed`, `attrib_row` filled `w` with `1.0f32.to_bits()` (`0x3f800000`,
1065353216) for every array. An `ivec4` / `uvec4` shader then added
1,065,353,216 to `coord.y`. That is far beyond `u_coordScale`'s range for
every input type.

**Fix (in `6c9606f6`):** `attrib_row` (`src/gl/draw.rs`, today lines
1156-1163) fills the integer 1 when `ga.integer`.
**Host tests:** `narrow_integer_arrays_fill_w_with_integer_one` and
`narrow_signed_integer_array_sign_extends` (`tests/es3_vertex_arrays.rs`).

### Class C: a packed 2_10_10_10 array in a buffer (1: 118, the GL error)

ES 3.0 2.8: an element of `GL_[UNSIGNED_]INT_2_10_10_10_REV` is one 32-bit
word, and `size` must be 4. At `ad2b5ed` the packed type mapped to `FLOAT32`
and passed the in-place filter (4 components ≥ the `vec2` input). The range
check then sized one element as **the input's components times
`attrib_type_size`**: 2 x 4 = 8 bytes, when the element is 4 bytes.

dEQP's array for case 118 holds `primitiveElementCount` (2) elements of 4
bytes, so the buffer is 8 bytes. `draw_elements` scans the indices: the type is
not `GL_FLOAT`, so `need_range` is set and `max_index` is 1. It then calls
`build_common(..., 0, max_index + 1)`. The last vertex then ends at
0 + 1 x 4 + 8 = 12 > 8 bytes, and the draw is refused.

**The raising check: `ad2b5ed:src/gl/draw.rs:3566`.** This is the
`return Err((GL_INVALID_OPERATION, FUNC))` of the in-place branch's range
check in `build_common` ("the array runs past the VBO"). The path to it:
`glDrawElementsInstanced` -> `draw_instanced` -> `draw_elements` (line 4434) ->
`build_common`. The spec allows the call: indices 0..1 of a 2-element packed
array in an 8-byte buffer are in range. On the current tip the same check is
`build_common`'s in-place range check at `src/gl/draw.rs:4117`. Packed types
no longer reach it. If the range check had passed, the GPU would have read
packed words as floats, which is the class A defect again.

**Fix (in `6c9606f6`):** `attrib_in_place_layout` sends the packed types and
`GL_HALF_FLOAT` to `convert_attrib`. That function sizes an element with
`attrib_elem_bytes` (one word) and decodes it with `packed_row`.
**Host tests:** `packed_array_filling_the_buffer_is_not_refused` (an array
ending exactly at the buffer's end draws without error), and
`packed_array_in_a_buffer_is_converted_from_first`, `..._honours_the_offset`
and `..._steps_by_the_stride`.

## 3. Host replay of the three shapes (HOST-ONLY)

`host-deqp-run.sh` could not run on the analysis host: there was no
`DEQP_WORK` and no built `deqp-gles3`. So the draws of cases 118, 94 and 207
were replayed as a throwaway cargo test, with the same calls, types,
storage, sizes and index forms. It used the stand-in back end of
`tests/es3_vertex_arrays.rs` and was not committed.

| draw | `ad2b5ed` (`gpu-powervr`) | tip `dfae7366` (`gpu-powervr`, `gpu-vivante`, `gpu-v3d` + `no-shader-table`) |
|---|---|---|
| 118: `glDrawElementsInstanced(GL_LINE_STRIP, 2, GL_UNSIGNED_SHORT, 0, 2)`, indices {0,1}, 8-byte packed array, `vec2` | `GL_INVALID_OPERATION`, no draw. With line 3566's error changed to a marker, the marker came back, so that line raises | no error; float32 rows of the decoded word |
| 94: `glDrawRangeElements(GL_LINES, 0, 2, 2, GL_UNSIGNED_SHORT, client)`, `GL_INT` x4 stride 16 in a buffer, `vec4` | no error, fetched **in place**: the raw int words | converted: `(float)v` per component |
| 207: `glDrawElements(GL_LINES, 2, GL_UNSIGNED_INT, buffer)`, `glVertexAttribIPointer` `GL_INT` x3, `ivec4` | `w` = 1065353216 | `w` = 1 |

The GL core path is the same with every `gpu-*` feature, because the
stand-in back end replaces the real one. The real back ends differ only in
`native_attrib_formats`, which the three shapes do not reach (5).

## 4. Pi 4 and host lists

- `docs/es3/pi4-gles3-fails.tsv` (on `925dbca9`, image D1 among the nine
  run) has no `draw.` row, so all `draw.random` cases pass on the V3D. The
  earlier run on `cd738b1a` failed the same 14 names. The causes are in the
  GL core, not in a GPU.
- `docs/es3/host-gles3-tip.tsv` counts `functional.draw` per GPU (2 pass /
  320 fail on each). Read-backs are black there, so the counts carry no image
  verdict and no `draw.random` row from it means anything. On the tip, no
  `draw.random` case gets a GL error on the host. That holds for the 118
  shape replayed above; the other 150 are not replayed.

## 5. GC7000UL: integer byte/short arrays fetched in place (suspected, not one of the 14)

The Vivante back end claims `BYTE`, `UBYTE`, `SHORT`, `USHORT` and `FIXED`
as native (`yakogl_backend_vivante`, `src/vivante/backend.rs` line 3493).
So a full-width `glVertexAttribIPointer` array of those types in a buffer is
fetched in place. `GlesAttrib` (`src/backend.rs`) carries `format` and
`normalized` but **no integer flag**, so the back end cannot tell an integer
input from a float one. `emit_vertex_elements` (`src/vivante/backend.rs` line
1303) writes `NFE_GENERIC_ATTRIB_SCALE = 1.0f` for every element. Mesa's
etnaviv (`etna_vertex_elements_state_create`) writes the integer `1` there
for a pure-integer format, and `fui(1.0f)` otherwise. If the GC FE converts an
element to float unless that scale is the integer 1, an `ivec`/`uvec` input
reads float bits. The draw.random cases on that path are 14, 72, 141, 162,
179, 200 and 209 (byte/ubyte/short/ushort -> `int`/`ivec2`/`uint`/`uvec2`/`uvec3`, buffer, full
width). The same holds for `vertex_arrays.*` integer rows. The i.MX8MP has
not run dEQP-GLES3 (#316), so this is a reading of code.

- **Fix, two options.** (i) GL core: `attrib_in_place_layout` returns false
  for an integer 8/16-bit array, so `convert_attrib` widens it to 32-bit
  integer rows. That is one place, and it costs CPU on the GC only. (ii) Add
  an `integer: u8` to `GlesAttrib`, set it from `ga.integer`, and have the
  Vivante write the integer 1 to `NFE_GENERIC_ATTRIB_SCALE` for it. That
  matches etnaviv and keeps the in-place fetch. It changes `src/backend.rs`,
  `src/backend.h`, `src/abi_layout*` and every back end's reading of
  `GlesAttrib`. Option (i) is the smaller change. Take option (ii) only if a
  board shows that the fetch cost matters.
- **Host test:** a new `tests/es3_draw_random_gc.rs` with
  `host-test,gpu-vivante,shaderc-online`. For (i), a stand-in back end with
  the Vivante's `native_attrib_formats` draws a `GL_UNSIGNED_BYTE` x4 ->
  `uvec4` buffer array (case 72's shape: offset 5) and expects converted
  integer rows, not an in-place fetch. For (ii), it reads
  `NFE_GENERIC_ATTRIB_SCALE` from the GC command stream of the real Vivante
  back end and expects 1 for that element and `0x3f800000` for a float
  array.
- **Paths and holders:** `src/gl/draw.rs` (#335) for (i); `src/vivante/**`
  (#333/#328), `src/backend.rs` (#333/#328 and #339), and `src/backend.h` and
  `src/abi_layout*` (#339) for (ii). It
  waits for those plans. It needs a GC board check before it is called a
  defect.

## 6. What the fix item and the drafts take from this

- `fix-draw-random`: nothing to change in `src/gl` for the 14. The three
  classes are fixed on main (`6c9606f6`, via `381b0f09`) and tested by
  `tests/es3_vertex_arrays.rs`. The suspected GC defect (5) is a separate
  item for the GC plans, not part of this one.
- The `docs/es3-cts-failures.md` row (draft, #330 holds the file):
  "`draw.random` 14 (AM62P `ad2b5ed`): GL core, fixed by `6c9606f6` (packed
  and `glVertexAttribPointer` int arrays in buffers converted, integer `w`
  fill). Pi 4 14 -> 0 on `925dbca9`. AM62P rerun pending.
  `docs/notes/es3-draw-random.md`."
- The board check (issue draft): on the AM62P, rerun
  `dEQP-GLES3.functional.draw.random.*` (the image with group D1's cases) on
  a current main, and expect 0 Fail. On the i.MX8MP, when #316 runs it, check
  cases 14, 72, 141, 162, 179, 200 and 209 for (5).

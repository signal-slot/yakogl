<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# es3-vertex-arrays, 2026-10-14, item tf-capture

## State of main
- GL core draw path (`src/gl/draw.rs`): a buffer array of a type the GPU
  cannot fetch as it lies is now converted to float32 rows like a client
  array. These types are the packed `GL_INT_2_10_10_10_REV` and
  `GL_UNSIGNED_INT_2_10_10_10_REV`, `GL_HALF_FLOAT`, and `GL_INT` /
  `GL_UNSIGNED_INT` set with `glVertexAttribPointer` (ES 3.0 2.9 converts
  those to float). `attrib_in_place_layout` decides. The conversion reads the
  buffer's CPU view, or the OSAL `gpu_read` if there is none. Element size and
  stride come from the real type (`attrib_elem_bytes`), so a tightly packed
  array that ends at the buffer's end is not refused.
- An integer array narrower than its `ivec4`/`uvec4` input fills w with the
  integer 1 (`attrib_row`, ES 3.0 2.8). The test is `ga.integer`: only
  `glVertexAttribIPointer` arrays feed integer inputs. A float array still
  fills the float 1.0.
- New host test `tests/es3_vertex_arrays.rs` (10 tests; it needs
  `host-test,gpu-powervr,shaderc-online` and no `report-es2`). `tools/check.sh`
  runs it next to `es3_occlusion_gl`.

## Open work
- tf-capture T2 and T3 (vertex_arrays, 41 AM62P failures: 20 int2_10_10_10 +
  21 components{2,3}_{i,u}vec4) are fixed on the host. Not yet seen on a board.

## Known defects
- `GL_HALF_FLOAT_OES` (0x8D61) is not handled by `attrib_type_size` /
  `attrib_component`: it is read as float32 words. Left as is (ES 2.0 answer
  unchanged; the type is only accepted under `report-es2`). It is wrong if an
  OES half-float array is ever used; a separate item should decide.
- The T3 fill follows the array (`ga.integer`), not the shader input's type. A
  float array bound to an `ivec4` input is undefined in GL, so this agrees with
  ES 3.0 for every defined case.
- The convert path with no CPU view (`gpu_read`) is not covered by the host
  test: its OSAL stub has none.

## Next board session
- AM62P: rerun `dEQP-GLES3.functional.vertex_arrays.single_attribute.*`
  (expect the 41 failures gone; the other 920 pass as before).
